//! D3 上拉加载更早历史（远端兜底翻页）—— `im_load_older_context` 编排纯逻辑。
//!
//! 解 NEED_HELIX_FIX_PULL_REFRESH：legacy `fetch_older_posts_by_context` 是 8 轮 `posts/postContext`
//! 翻页循环（moving anchor + 严格更早过滤 + 跨批去重 + pivot 推进），而 helix `im_post_context` 仅**单次**
//! request/response，翻不到顶。本命令把那段翻页**编排**下沉：每轮 postContext 回报推进 anchor，凑够 target
//! 条严格更早 / 服务端无更早 / pivot 卡住 / 轮数耗尽 → durable message readback 后发布 timeline 事件。
//!
//! sans-IO（HX-C001）：每 HTTP = `Effect::Http` 经 `PortReply` 回报，多轮**不是**同步 `for` 而是
//! corr_map 续接状态机（module.rs 注册 `LoadOlderContext` 携 [`LoadOlderState`]，每回报 → `ingest_round`
//! 决定续/停）。本文件只产纯数据，零 I/O。透传 wire Post（camelCase）原样回灌不二次序列化（HX-C005）；
//! 只回灌严格更早部分（锚点/重叠批本就在本地）避免重复 prepend。详见 projection-schema.md §1.3。

use serde_json::Value;
use std::collections::HashSet;

use crate::state::ChannelId;

/// 投影/翻页命令名（dispatch 据此路由，与 outbound registry 解耦——本命令是本地编排非纯 outbound）。
pub const LOAD_OLDER_CONTEXT: &str = "im_load_older_context";

/// 翻页轮数硬上限（对齐 legacy `for _ in 0..8`）——防服务端病态返回打满 HTTP。
const MAX_ROUNDS: u32 = 8;

/// canonical `before` 合法区间。
const TARGET_MIN: u32 = 1;
const TARGET_MAX: u32 = crate::timeline_state::MAX_TIMELINE_PAGE_SIZE;
/// `before` 缺省。
const TARGET_DEFAULT: u32 = crate::timeline_state::DEFAULT_TIMELINE_PAGE_SIZE;

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
    /// 首轮请求的权威锚点 ID；时间边界必须从 Go 回包中同 ID 的 post 建立。
    anchor_post_id: String,
    /// Go 权威锚点 createAt；首轮响应前不可用。
    anchor_create_at: Option<i64>,
    /// 要凑够的严格更早条数。
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
    /// 响应缺锚或字段畸形后锁死；已收集消息会清空，不发布伪历史。
    failed: bool,
    /// Helix 从已 attach timeline 解析出的 opaque window token；wire command 不信任端侧补传。
    window_token: Option<String>,
    /// 本次 readback 应返回的完整窗口大小，受 typed timeline 60 行上限约束。
    readback_limit: u32,
    /// Host 注入的可选 transport correlation，只用于 HTTP header 与真实链证据。
    request_id: Option<String>,
}

impl LoadOlderState {
    /// 新建（pivot 初始 = anchor_post_id；时间边界等待首轮 Go 回包）。
    pub fn new(channel_id: ChannelId, anchor_post_id: String, target: u32) -> Self {
        debug_assert!(
            (TARGET_MIN..=TARGET_MAX).contains(&target),
            "LoadOlderState requires a prevalidated page size"
        );
        Self {
            channel_id,
            anchor_post_id: anchor_post_id.clone(),
            anchor_create_at: None,
            target,
            pivot_id: anchor_post_id,
            prev_oldest: i64::MAX,
            seen: HashSet::new(),
            older: Vec::new(),
            rounds: 0,
            has_more: false,
            failed: false,
            window_token: None,
            readback_limit: 0,
            request_id: None,
        }
    }

    /// 返回当前上拉所属频道。
    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }

    /// 返回 host transport correlation；它不得进入 Go body 或业务事件字段。
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// 绑定可选 host transport correlation，供每轮 HTTP 保持同一证据键。
    pub(crate) fn attach_request_id(&mut self, request_id: Option<String>) {
        self.request_id = request_id;
    }

    /// 返回首轮权威响应建立的 anchor 时间；响应前固定为 0。
    pub fn anchor_create_at(&self) -> i64 {
        self.anchor_create_at.unwrap_or_default()
    }

    /// 返回 canonical `before` 目标。
    pub fn target(&self) -> u32 {
        self.target
    }

    /// 返回下一轮 postContext 的服务端 pivot。
    pub fn pivot_id(&self) -> &str {
        &self.pivot_id
    }

    /// 返回已去重、严格更早的消息数量。
    pub fn older_count(&self) -> usize {
        self.older.len()
    }

    /// 返回服务端是否仍可能存在更早消息。
    pub fn has_more(&self) -> bool {
        self.has_more
    }

    /// 将 wire intent 绑定到 Helix 已 attach 的 typed timeline，而不是相信平台窗口状态。
    pub(crate) fn attach_window(&mut self, window_token: String, visible_items: usize) {
        self.window_token = Some(window_token);
        self.readback_limit = visible_items
            .saturating_add(self.target as usize)
            .min(crate::timeline_state::MAX_TIMELINE_WINDOW_ITEMS)
            as u32;
    }

    /// 返回 Helix 已解析的 attached window token。
    pub(crate) fn window_token(&self) -> Option<&str> {
        self.window_token.as_deref()
    }

    /// 返回持久化后完整 readback 的有界行数。
    pub(crate) fn readback_limit(&self) -> u32 {
        self.readback_limit.max(1)
    }

    /// 返回原始 anchor id，用于 attached-window 与 failure patch 关联。
    pub(crate) fn anchor_post_id(&self) -> &str {
        self.anchor_post_id.as_str()
    }

    /// 返回已完成严格校验和确定性排序的旧消息副本。
    pub(crate) fn older_rows(&self) -> Vec<Value> {
        self.older.clone()
    }

    /// 返回本轮响应是否因缺锚或畸形字段失效。
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    /// 吃一轮 postContext 响应（wire Post 数组）→ 推进态 + 返回续/停决策（逐字对齐 legacy 翻页循环）。
    ///
    /// 停因 → `has_more`：凑够 target / 轮数达上限 → true（仍有更早）；空批 / 本批最旧≥上批（服务端耗尽）/
    /// pivot 卡住（无带 id 的条或 pivot 不变）→ false（到顶）。否则推进 pivot+prev_oldest → 续。
    /// 收**严格更早**（`createAt < anchor`）+ 按 temporaryId 跨批去重；pivot = 本批最旧、带非空 server id 的一条。
    pub fn ingest_round(&mut self, rows: &[Value]) -> RoundDecision {
        if self.failed {
            return RoundDecision::Done;
        }
        self.rounds += 1;

        // 首轮/续轮都必须包含本次请求 pivot；否则不能证明响应属于当前 operation。
        if rows.is_empty() {
            return if self.anchor_create_at.is_none() {
                self.invalidate()
            } else {
                self.stop(false)
            };
        }

        let expected_pivot = self.pivot_id.clone();
        let mut response_pivot_at = None;
        for row in rows {
            let Some((id, channel_id, create_at)) = required_row_fields(row) else {
                return self.invalidate();
            };
            if id == expected_pivot {
                if channel_id != self.channel_id.as_str() || response_pivot_at.is_some() {
                    return self.invalidate();
                }
                response_pivot_at = Some(create_at);
            }
        }
        let Some(response_pivot_at) = response_pivot_at else {
            return self.invalidate();
        };

        // 第一轮只信原始 anchor post 的 createAt，不读取 UI 时间戳。
        if self.anchor_create_at.is_none() {
            if expected_pivot != self.anchor_post_id {
                return self.invalidate();
            }
            self.anchor_create_at = Some(response_pivot_at);
        }
        let Some(anchor_create_at) = self.anchor_create_at else {
            return self.invalidate();
        };

        // 在写入累积态前完整校验候选旧消息，保证坏字段不会留下部分结果。
        for row in rows {
            let Some((_, channel_id, create_at)) = required_row_fields(row) else {
                return self.invalidate();
            };
            if channel_id == self.channel_id.as_str()
                && create_at < anchor_create_at
                && row
                    .get("temporaryId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                return self.invalidate();
            }
        }

        // 收严格更早 + 同频道 + 去重；pivot 只从当前频道的权威 server id 推进。
        let mut batch_oldest = i64::MAX;
        let mut next_pivot: Option<&str> = None;
        let mut next_pivot_at = i64::MAX;
        for row in rows {
            let Some((id, channel_id, create_at)) = required_row_fields(row) else {
                return self.invalidate();
            };
            if channel_id != self.channel_id.as_str() {
                continue;
            }
            batch_oldest = batch_oldest.min(create_at);
            if create_at < next_pivot_at {
                next_pivot_at = create_at;
                next_pivot = Some(id);
            }
            if create_at < anchor_create_at {
                if let Some(tmp) = row.get("temporaryId").and_then(Value::as_str) {
                    if self.seen.insert(tmp.to_string()) {
                        self.older.push(row.clone());
                    }
                } else {
                    return self.invalidate();
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

    fn invalidate(&mut self) -> RoundDecision {
        self.older.clear();
        self.has_more = false;
        self.failed = true;
        RoundDecision::Done
    }
}

fn required_row_fields(row: &Value) -> Option<(&str, &str, i64)> {
    let id = row.get("id")?.as_str().filter(|value| !value.is_empty())?;
    let channel_id = row
        .get("channelId")?
        .as_str()
        .filter(|value| !value.is_empty())?;
    let create_at = row.get("createAt")?.as_i64().filter(|value| *value > 0)?;
    Some((id, channel_id, create_at))
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

mod effects;
pub use effects::{post_context_http, post_context_http_tracked};
mod request;
pub use request::{build_post_context_body, parse_request};

#[cfg(test)]
#[path = "older_context_tests.rs"]
mod tests;
