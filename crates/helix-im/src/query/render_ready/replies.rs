//! S8 回复族 render-ready 投影 —— `im:channel:replies`（issue #57·C013 纯渲染壳·终局切）。
//!
//! 把回复读接口整形成扁平 RenderNode：前端只维护 `messageById` 与 `replyIdsByRoot` 两个索引，
//! 不递归组树、不按 root 重扫消息数组。
//!
//! ## 形态
//! emit `im:channel:replies{reqId,channelId,rootMessageId,projectionMode,revision,
//! replyIds,nodes,replyCount,cursor,hasMore}`：
//!   - `reqId`    = 前端 bridge 注入的请求关联键（与 `im:read:result` 同一 req_id·壳认领抽屉归属）。
//!   - `replyIds` = 不含 root 的回复 id，去重保序；可 O(1) 写入 `replyIdsByRoot[rootMessageId]`。
//!   - `nodes`    = root + replies 的 render-ready message 行；可 O(n) 写入 `messageById`。
//!   - `replyCount` = 服务端总数与本页回复数的较大值。
//!
//! ## 与冻结投影的关系（契约只读·C004）
//! 本投影是 **render-ready 渲染通道**（与 `im:read:result` §1.2 / `im:channel:members` §1.5 同性质·
//! **不计入** 21 投影冻结集——前端直绑、后端响应体权威）。冻结的 `im:read:result`（`{req_id, body}`）
//! 投影**保持原样**额外 emit（UC-2.4 ② 契约面照旧裁定 verbatim body）——本投影**额外** emit，不改既有键集。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyProjectionMode {
    Snapshot,
    Append,
}

impl ReplyProjectionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Append => "append",
        }
    }
}

/// HTTP 发起时冻结的回复投影关联上下文。业务模式来自 command 名，不从 `reqId` 猜测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyProjectionRequest {
    pub req_id: String,
    pub channel_id: String,
    pub root_hint: String,
    pub mode: ReplyProjectionMode,
    pub revision: u64,
}

impl ReplyProjectionRequest {
    pub fn from_command(command: &str, payload: &[u8], revision: u64) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
        let req_id = value
            .get("req_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())?
            .to_string();
        let (mode, root_key) = match command {
            "im_get_replies" => (ReplyProjectionMode::Snapshot, "reply_id"),
            "im_get_reply_branch" => (ReplyProjectionMode::Append, "reply_first_level_id"),
            _ => return None,
        };
        Some(Self {
            req_id,
            channel_id: value
                .get("channel_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            root_hint: value
                .get("root_message_id")
                .or_else(|| value.get(root_key))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            mode,
            revision,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlatReplyProjection {
    pub channel_id: String,
    pub root_message_id: String,
    pub projection_mode: ReplyProjectionMode,
    pub revision: u64,
    pub reply_ids: Vec<String>,
    pub nodes: Vec<serde_json::Value>,
    pub reply_count: usize,
    pub has_more: bool,
}

impl FlatReplyProjection {
    pub fn empty() -> Self {
        Self {
            channel_id: String::new(),
            root_message_id: String::new(),
            projection_mode: ReplyProjectionMode::Snapshot,
            revision: 0,
            reply_ids: Vec::new(),
            nodes: Vec::new(),
            reply_count: 0,
            has_more: false,
        }
    }
}

/// 从读族透传 body 防御性抽 Post id 列表（render-ready·下沉自前端 `read-result-extract.ts`）。
///
/// 探针顺序兼容两 endpoint 不同壳形态：
///  - getReplies：`{rootPost, replies:[Post]}`（partial 1 §15 GetRepliesResp）
///  - getReplyBranch：HasMorePage[Post]（`{data:[Post]}` / `{list:[Post]}` / 顶层 `[Post]`）
/// 取 rootPost.id + replies[].id + data[].id + list[].id + 顶层数组[].id。去重保序·仅非空 string。
/// body 缺/非对象（失败回灌 `{req_id, error}` 或畸形）→ 返空数组（前端 reject 语义·清抽屉）。
pub fn extract_flat_replies(body: &serde_json::Value) -> FlatReplyProjection {
    // Go HTTP 统一 CommonRes 会把业务结果包在 `data`；先剥一层信封，再按两类业务结果抽取。
    // getReplyBranch 的业务结果自身仍有 `data:[Post]`，下方循环继续处理该数组。
    let body = body
        .get("data")
        .filter(|value| value.is_object() || value.is_array())
        .unwrap_or(body);
    let mut projection = FlatReplyProjection::empty();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |post: &serde_json::Value, is_root: bool| {
        let id = post
            .get("id")
            .or_else(|| post.get("msgId"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if id.is_empty() || !seen.insert(id.to_string()) {
            return;
        }
        if is_root {
            projection.root_message_id = id.to_string();
        } else {
            projection.reply_ids.push(id.to_string());
        }
        projection.nodes.push(super::core::shape_row(post, ""));
    };
    if let Some(obj) = body.as_object() {
        if let Some(root) = obj.get("rootPost") {
            push(root, true);
        }
        if let Some(arr) = ["replies", "data", "list"]
            .iter()
            .find_map(|key| obj.get(*key).and_then(serde_json::Value::as_array))
        {
            for post in arr {
                push(post, false);
            }
        }
    } else if let Some(arr) = body.as_array() {
        for post in arr {
            push(post, false);
        }
    }
    drop(push);

    if projection.root_message_id.is_empty() {
        projection.root_message_id = projection
            .nodes
            .first()
            .and_then(|node| {
                ["replyRootId", "replyId", "msgId"]
                    .iter()
                    .find_map(|key| node.get(*key).and_then(serde_json::Value::as_str))
            })
            .unwrap_or("")
            .to_string();
    }
    let server_count = projection
        .nodes
        .first()
        .and_then(|node| node.get("replyCount"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as usize;
    projection.reply_count = server_count.max(projection.reply_ids.len());
    projection.channel_id = projection
        .nodes
        .iter()
        .find_map(|node| {
            node.get("channelId")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
        .to_string();
    projection.has_more = body
        .get("hasMore")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    projection
}

/// 把 command 语义和响应数据合并成稳定投影；缺失的服务端游标明确冻结为 `null`。
pub fn finalize_projection(
    request: &ReplyProjectionRequest,
    mut projection: FlatReplyProjection,
) -> FlatReplyProjection {
    projection.projection_mode = request.mode;
    projection.revision = request.revision;
    if projection.channel_id.is_empty() {
        projection.channel_id.clone_from(&request.channel_id);
    }
    if projection.root_message_id.is_empty() {
        projection.root_message_id.clone_from(&request.root_hint);
    }
    projection
}

/// 在 Helix 内完成乱序抑制和 append 跨页去重，Angular 不解释 revision/模式。
///
/// - snapshot：权威替换已见 id 集；
/// - append：只下发从未出现的 id，保持本次服务端顺序；
/// - 旧 revision：不 emit render-ready 投影，避免覆盖更新快照。
pub fn accept_projection(
    projection: &mut FlatReplyProjection,
    revisions: &mut HashMap<String, u64>,
    seen_ids: &mut HashMap<String, HashSet<String>>,
) -> bool {
    let root = projection.root_message_id.as_str();
    if root.is_empty() {
        return true;
    }
    if revisions
        .get(root)
        .is_some_and(|latest| *latest > projection.revision)
    {
        return false;
    }
    revisions.insert(root.to_string(), projection.revision);

    match projection.projection_mode {
        ReplyProjectionMode::Snapshot => {
            seen_ids.insert(
                root.to_string(),
                projection.reply_ids.iter().cloned().collect(),
            );
        }
        ReplyProjectionMode::Append => {
            let seen = seen_ids.entry(root.to_string()).or_default();
            let accepted: HashSet<String> = projection
                .reply_ids
                .iter()
                .filter(|id| !seen.contains(*id))
                .cloned()
                .collect();
            projection.reply_ids.retain(|id| accepted.contains(id));
            projection.nodes.retain(|node| {
                let id = node
                    .get("msgId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                id.is_empty() || id == root || accepted.contains(id)
            });
            seen.extend(accepted);
        }
    }
    true
}

/// emit 显式快照/增量语义；`cursor:null` 表示当前 Go 接口未提供可恢复 opaque cursor。
///
/// 静态 shape（序列化失败=编程错·边界输入零信任已在 `extract_flat_replies` 完成）。
pub fn emit_channel_replies(req_id: &str, projection: FlatReplyProjection) -> Effect {
    let payload = serde_json::json!({
        "event": "im:channel:replies",
        "data": {
            "reqId": req_id,
            "channelId": projection.channel_id,
            "rootMessageId": projection.root_message_id,
            "projectionMode": projection.projection_mode.as_str(),
            "revision": projection.revision,
            "replyIds": projection.reply_ids,
            "nodes": projection.nodes,
            "replyCount": projection.reply_count,
            "cursor": serde_json::Value::Null,
            "hasMore": projection.has_more,
        },
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_replies: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

#[cfg(test)]
#[path = "replies_tests.rs"]
mod tests;
