//! S8 回复族 render-ready 投影 —— `im:channel:replies`（issue #57·C013 纯渲染壳·终局切）。
//!
//! 把**回复链 postId 抽取**（读族透传 body 的 rootPost/replies/data/list 解析重组）从渲染壳下沉
//! helix-im：前端 `applyChannelReplies` 退化为**纯绑定**（直接把 helix 给的 `replyIds` 1:1 绑到
//! AX reply-drawer 的 `data-reply-id`，零 extract / 零 body 探针 / 零 shape 兼容）。
//!
//! ## 形态
//! emit `im:channel:replies{reqId, replyIds:[postId]}`：
//!   - `reqId`    = 前端 bridge 注入的请求关联键（与 `im:read:result` 同一 req_id·壳认领抽屉归属）。
//!   - `replyIds` = render-ready 回复链 post id 终态数组（去重保序·helix 已抽好）。
//!
//! ## 与冻结投影的关系（契约只读·C004）
//! 本投影是 **render-ready 渲染通道**（与 `im:read:result` §1.2 / `im:channel:members` §1.5 同性质·
//! **不计入** 21 投影冻结集——前端直绑、后端响应体权威）。冻结的 `im:read:result`（`{req_id, body}`）
//! 投影**保持原样**额外 emit（UC-2.4 ② 契约面照旧裁定 verbatim body）——本投影**额外** emit，不改既有键集。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};

/// 从读族透传 body 防御性抽 Post id 列表（render-ready·下沉自前端 `read-result-extract.ts`）。
///
/// 探针顺序兼容两 endpoint 不同壳形态：
///  - getReplies：`{rootPost, replies:[Post]}`（partial 1 §15 GetRepliesResp）
///  - getReplyBranch：HasMorePage[Post]（`{data:[Post]}` / `{list:[Post]}` / 顶层 `[Post]`）
/// 取 rootPost.id + replies[].id + data[].id + list[].id + 顶层数组[].id。去重保序·仅非空 string。
/// body 缺/非对象（失败回灌 `{req_id, error}` 或畸形）→ 返空数组（前端 reject 语义·清抽屉）。
pub fn extract_reply_ids(body: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |v: &serde_json::Value| {
        if let Some(id) = v.get("id").and_then(serde_json::Value::as_str) {
            if !id.is_empty() && seen.insert(id.to_string()) {
                out.push(id.to_string());
            }
        }
    };
    if let Some(obj) = body.as_object() {
        if let Some(root) = obj.get("rootPost") {
            push(root);
        }
        for k in ["replies", "data", "list"] {
            if let Some(arr) = obj.get(k).and_then(serde_json::Value::as_array) {
                for el in arr {
                    push(el);
                }
            }
        }
    } else if let Some(arr) = body.as_array() {
        for el in arr {
            push(el);
        }
    }
    out
}

/// emit `im:channel:replies{reqId, replyIds}`。
///
/// 静态 shape（序列化失败=编程错·边界输入零信任已在 `extract_reply_ids` 完成）。
pub fn emit_channel_replies(req_id: &str, reply_ids: Vec<String>) -> Effect {
    let payload = serde_json::json!({
        "event": "im:channel:replies",
        "data": { "reqId": req_id, "replyIds": reply_ids },
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
