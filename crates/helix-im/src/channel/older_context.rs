//! D3 上拉加载更早历史（远端兜底翻页）—— `im_load_older_context` 编排纯逻辑。
//!
//! 解 NEED_HELIX_FIX_PULL_REFRESH：legacy `fetch_older_posts_by_context` 是 8 轮 `posts/postContext`
//! 翻页循环（moving anchor + 严格更早过滤 + 跨批去重 + pivot 推进），而 helix `im_post_context` 仅**单次**
//! request/response，翻不到顶。本命令把那段翻页**编排**下沉：每轮 postContext 回报推进 anchor，凑够 target
//! 条严格更早 / 服务端无更早 / pivot 卡住 / 轮数耗尽 → emit `im:messages:older_loaded` 供前端 prepend。
//!
//! sans-IO（HX-C001）：每 HTTP = `Effect::Http` 经 `PortReply` 回报，多轮**不是**同步 `for` 而是
//! corr_map 续接状态机（module.rs 注册 `LoadOlderContext` 携 [`LoadOlderState`]，每回报 → `ingest_round`
//! 决定续/停）。本文件只产纯数据，零 I/O。透传 wire Post（camelCase）原样回灌不二次序列化（HX-C005）；
//! 只回灌严格更早部分（锚点/重叠批本就在本地）避免重复 prepend。详见 projection-schema.md §1.3。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect, HttpRequest};
use helix_core::Correlation;
use serde_json::{json, Value};
use std::collections::HashSet;

use crate::error::ImError;
use crate::state::ChannelId;

/// 投影/翻页命令名（dispatch 据此路由，与 outbound registry 解耦——本命令是本地编排非纯 outbound）。
pub const LOAD_OLDER_CONTEXT: &str = "im_load_older_context";

/// 每轮 `postContext` 请求条数（`before`），对齐 legacy `batch=50`（含会被砍的 [T0,Tk)）。
const BATCH: i64 = 50;

/// 翻页轮数硬上限（对齐 legacy `for _ in 0..8`）——防服务端病态返回打满 HTTP。
const MAX_ROUNDS: u32 = 8;

/// target 条数 clamp 区间（对齐 legacy `before.clamp(1, 100)`）。
const TARGET_MIN: u32 = 1;
const TARGET_MAX: u32 = 100;
/// `limit` 缺省（对齐前端 invoke `limit:20`，UC-2.2）。
const TARGET_DEFAULT: u32 = 20;

/// 一轮 `ingest_round` 后的续/停决策（停因写进 [`LoadOlderState::has_more`]）。
#[derive(Debug, PartialEq, Eq)]
pub enum RoundDecision {
    /// 还没凑够 + 服务端有更早 + pivot 能前进 + 轮数没耗尽 → 再发一轮 postContext。
    Continue,
    /// 停（凑够 target / 服务端无更早 / pivot 无法前进 / 轮数耗尽）→ emit。
    Done,
}

/// 翻页编排的 owned 累积态（进 corr_map 跨 PortReply 存活，HX-C002：owned 不带 `&'a`）。
///
/// `Debug + Clone + PartialEq`（与 `CorrelationContext` 一致）——内含 `serde_json::Value`（f64）
/// 实现 `PartialEq` 但不 `Eq`，故随上层一并 derive `PartialEq` 不 derive `Eq`。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadOlderState {
    channel_id: ChannelId,
    /// 锚点 createAt——只收 `createAt < anchor_create_at` 的严格更早。
    anchor_create_at: i64,
    /// 要凑够的严格更早条数（已 clamp）。
    target: u32,
    /// 下轮 postContext 的 pivot（本批最旧、带非空 server id 的一条；初始 = anchor_post_id）。
    pivot_id: String,
    /// 上一批最旧 createAt（用于「本轮没翻出更早 → 服务端耗尽」判定）。
    prev_oldest: i64,
    /// 跨批 temporaryId 去重集（锚点 / [T0,Tk) / 重叠批不重复 prepend）。
    seen: HashSet<String>,
    /// 已收集的严格更早 wire Post（透传，emit 前按 createAt 升序 + truncate(target)）。
    older: Vec<Value>,
    /// 已跑轮数（达 MAX_ROUNDS 即停）。
    rounds: u32,
    /// 停时是否「还有更早」（target 凑够 / 轮数耗尽 → true；服务端耗尽 / pivot 卡住 → false）。
    has_more: bool,
}

impl LoadOlderState {
    /// 新建（pivot 初始 = anchor_post_id；prev_oldest = MAX 让首轮恒「翻出更早」）。
    pub fn new(
        channel_id: ChannelId,
        anchor_post_id: String,
        anchor_create_at: i64,
        target: u32,
    ) -> Self {
        Self {
            channel_id,
            anchor_create_at,
            target: target.clamp(TARGET_MIN, TARGET_MAX),
            pivot_id: anchor_post_id,
            prev_oldest: i64::MAX,
            seen: HashSet::new(),
            older: Vec::new(),
            rounds: 0,
            has_more: false,
        }
    }

    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }
    pub fn anchor_create_at(&self) -> i64 {
        self.anchor_create_at
    }
    pub fn target(&self) -> u32 {
        self.target
    }
    pub fn pivot_id(&self) -> &str {
        &self.pivot_id
    }
    pub fn older_count(&self) -> usize {
        self.older.len()
    }
    pub fn has_more(&self) -> bool {
        self.has_more
    }

    /// 吃一轮 postContext 响应（wire Post 数组）→ 推进态 + 返回续/停决策（逐字对齐 legacy 翻页循环）。
    ///
    /// 停因 → `has_more`：凑够 target / 轮数达上限 → true（仍有更早）；空批 / 本批最旧≥上批（服务端耗尽）/
    /// pivot 卡住（无带 id 的条或 pivot 不变）→ false（到顶）。否则推进 pivot+prev_oldest → 续。
    /// 收**严格更早**（`createAt < anchor`）+ 按 temporaryId 跨批去重；pivot = 本批最旧、带非空 server id 的一条。
    pub fn ingest_round(&mut self, rows: &[Value]) -> RoundDecision {
        self.rounds += 1;

        // (1) 空批 → 服务端无更早。
        if rows.is_empty() {
            return self.stop(false);
        }

        // 收严格更早 + 去重；算本批最旧 createAt + next_pivot（最旧带非空 server id 的一条——服务端
        // 返回的 self 消息也带真 id，故 pivot 总能越过本地 id='' 墙）。
        let mut batch_oldest = i64::MAX;
        let mut next_pivot: Option<&str> = None;
        let mut next_pivot_at = i64::MAX;
        for row in rows {
            let create_at = row.get("createAt").and_then(Value::as_i64).unwrap_or(0);
            batch_oldest = batch_oldest.min(create_at);
            if let Some(id) = row
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                if create_at < next_pivot_at {
                    next_pivot_at = create_at;
                    next_pivot = Some(id);
                }
            }
            // 严格更早 + 跨批去重（锚点 / [T0,Tk) / 重叠批不重复 prepend）。
            if create_at < self.anchor_create_at {
                let tmp = row
                    .get("temporaryId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !tmp.is_empty() && self.seen.insert(tmp) {
                    self.older.push(row.clone());
                }
            }
        }

        // 凑够 target / 轮数达上限 → 停（仍有更早）。
        if self.older.len() >= self.target as usize || self.rounds >= MAX_ROUNDS {
            return self.stop(true);
        }
        // 本批没翻出更早（最旧 ≥ 上批最旧）→ 服务端耗尽 → 停（到顶）。
        if batch_oldest >= self.prev_oldest {
            return self.stop(false);
        }
        // pivot 能前进 → 续拉；否则（无可锚 id / pivot 不变）→ 停（到顶）。
        match next_pivot {
            Some(id) if id != self.pivot_id => {
                self.pivot_id = id.to_string();
                self.prev_oldest = batch_oldest;
                RoundDecision::Continue
            }
            _ => self.stop(false),
        }
    }

    /// 收尾：truncate 到 target（凑超也只回灌 target 条）+ 记 has_more → Done。
    fn stop(&mut self, has_more: bool) -> RoundDecision {
        self.older.sort_by_key(sort_key);
        self.older.truncate(self.target as usize);
        self.has_more = has_more;
        RoundDecision::Done
    }
}

/// emit 升序排序键：createAt 升序（前端 prepend）；同毫秒按 temporaryId 确定化（HX-C010 有序 Effect）。
fn sort_key(row: &Value) -> (i64, String) {
    (
        row.get("createAt").and_then(Value::as_i64).unwrap_or(0),
        row.get("temporaryId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    )
}

/// 解析 `im_load_older_context` payload → 初始 [`LoadOlderState`]。入参（snake_case，前端 bridge 注入）：
/// `channel_id`/`anchor_post_id`/`anchor_create_at`（int 毫秒）/`limit`（缺省 20，clamp 1..=100）。
/// 边界零信任：缺/坏 → `Err`，不 panic。
pub fn parse_request(payload: &[u8]) -> Result<LoadOlderState, ImError> {
    let v: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} payload: {e}")))?;
    let channel_str = v
        .get("channel_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} 缺 channel_id")))?;
    let channel_id = ChannelId::from_str(channel_str)
        .ok_or_else(|| ImError::Parse(format!("非法 channel_id: {channel_str}")))?;
    let anchor_post_id = v
        .get("anchor_post_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} 缺 anchor_post_id")))?
        .to_string();
    let anchor_create_at = v
        .get("anchor_create_at")
        .and_then(Value::as_i64)
        .ok_or_else(|| ImError::Parse(format!("{LOAD_OLDER_CONTEXT} 缺/坏 anchor_create_at")))?;
    let target = v
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| (l as u32).clamp(TARGET_MIN, TARGET_MAX))
        .unwrap_or(TARGET_DEFAULT);
    Ok(LoadOlderState::new(
        channel_id,
        anchor_post_id,
        anchor_create_at,
        target,
    ))
}

/// 从剥信封后的裸 postContext body 取 wire Post 数组（喂 [`LoadOlderState::ingest_round`]）。
/// 服务端 `WriteJSON(SetData(posts))`——裸 body 可能是**数组**或 `{data:[...]}` 嵌套（legacy
/// 同款两形容错）。JSON 畸形 / 非数组 → 空 `Vec`（不 panic）。
pub fn extract_post_rows(raw_body: &[u8]) -> Vec<Value> {
    match serde_json::from_slice::<Value>(raw_body) {
        Ok(Value::Array(arr)) => arr,
        Ok(Value::Object(obj)) => obj
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// 构造一轮 `postContext` 的 `(endpoint_path, wire_body)`。真源 body `{postId, before:50}`（http.rs:96）。
pub fn build_post_context_body(state: &LoadOlderState) -> (&'static str, Value) {
    (
        "posts/postContext",
        json!({ "postId": state.pivot_id(), "before": BATCH }),
    )
}

/// 组装一轮 `posts/postContext` POST Effect（IM 网关 base + connectionId 身份头，与既有 outbound 同壳）。
/// 真凭据由 driver 注入（C6，core 不拼）；body 由 [`build_post_context_body`] 产。
pub fn post_context_http(
    base_url: &str,
    body: &Value,
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::to_effect::connection_id_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{base_url}/posts/postContext"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(body).unwrap_or_default())),
        },
    }
}

/// emit `im:messages:older_loaded{channelId, messages[], hasMore}`（升序 wire Post 透传 + hasMore）。
///
/// `messages` = 严格更早 wire Post **升序**数组（前端 prepend），透传 camelCase（HX-C005 不改业务数据）。
/// **emit 是唯一权威排序点**（createAt 升序 + 同毫秒 temporaryId 确定化，HX-C010）——不依赖调用前
/// `stop()` 是否已排（防中途 emit / Err 收尾路径漏排）；对已排序输入幂等。空集合 → `messages:[]`，不 panic。
pub fn emit_older_loaded(state: &LoadOlderState) -> Effect {
    let mut messages = state.older.clone();
    messages.sort_by_key(sort_key);
    let payload = json!({
        "event": "im:messages:older_loaded",
        "data": {
            "channelId": state.channel_id.as_str(),
            "messages": crate::render_ready::shape_message_rows(&serde_json::Value::Array(messages)),
            "hasMore": state.has_more,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_older_loaded: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

#[cfg(test)]
#[path = "older_context_tests.rs"]
mod tests;
