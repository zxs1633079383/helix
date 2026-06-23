//! UC-10 通知/待办装配链（messageNotice / Rust todo · full-map partials/3 集合十）。
//!
//! ## 链路（行为真源 cses-client `handlers/channel.rs:232-355`）
//!   1. **收集**：每个 `increment_channel` 帧把 `channel.mentionList + channel.urgentPostList`
//!      的 post id 累入会话级缓冲 `ImState::about_me_post_ids`（hello 边界 reset，无残留跨会话）。
//!   2. **触发（hello 收尾）**：global `increment_channel_end`（None 分支）收尾时，若缓冲非空 →
//!      build `posts/queryTodoList {postIds}` HTTP Effect + 注册 `CorrelationContext::TodoQuery`，
//!      然后**读完即清空**缓冲（避免下轮 hello 残留，对齐真源 `clear_about_me_post_ids`）。
//!   3. **回报装配**：`queryTodoList` HTTP 回报 → 剥 ADR-007 信封 → 解 `data.channels[].posts[]`
//!      → 每 post 装配成 todo item `{id:"{postId}_{messageType}", channel, post, type, canDel}`
//!      → emit `im:todo:updated {items:[...]}`（前端 `getTodoUpdated$` → `INIT_TODO_LIST_DATA`）。
//!
//! ## 不变量
//! - **透传业务数据**：channel/post 原样嵌入（后端权威，HX-C005 不在热路径瞎序列化）。
//! - **id 装配契约**：`"{postId}_{messageType}"`（真源 `channel.rs:288`）——messageType 缺省空串。
//! - **canDel**：仅 `messageType == "mention"` 为 true（真源 `channel.rs:292`）。
//! - **payload object 包裹**：emit `{items:[...]}` 而非裸数组（避免前端 Electron 多参兼容层
//!   `Array.isArray → spread` 误判，真源 `channel.rs:346-348`）。
//! - **边界零信任**：缺字段/坏 JSON → 跳过该 post/空 items，绝不 panic（helix-im 不变量 4）。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};

/// 从一个 `increment_channel` 帧 data 收集本群「about-me」post id（mention + urgent）。
///
/// 真源 `channel.rs:142-148`：`channel.mentionList` ++ `channel.urgentPostList`（两源拼接，
/// 顺序保真）。字段可能直接在 data 顶层或嵌 `data.channel` 下（与 increment parser 同源探查）。
/// 缺/非数组 → 空 Vec（零信任）。非字符串元素跳过。
pub(crate) fn collect_about_me_ids(data: &Value) -> Vec<String> {
    let channel = data.get("channel").unwrap_or(data);
    let mut ids = Vec::new();
    for key in ["mentionList", "urgentPostList"] {
        if let Some(arr) = channel.get(key).and_then(Value::as_array) {
            ids.extend(arr.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
    }
    ids
}

/// build `posts/queryTodoList` 的 wire body（真源 partials/1 §18：`{postIds:[]string}` 非空）。
///
/// `post_ids` 调用方保证非空（global-end 已先判 `!about_me_post_ids.is_empty()`）。
pub(crate) fn query_todo_body(post_ids: &[String]) -> Value {
    json!({ "postIds": post_ids })
}

/// `queryTodoList` HTTP 成功回报（裸响应体，信封已剥）→ emit `im:todo:updated {items:[...]}`。
///
/// 解 `data.channels[].posts[]`（真源 `channel.rs:255-296`）。响应 `status != SUCCESS/200` 或
/// 结构缺失 → 空 items（仍 emit 让前端清空/无害刷新，不挂起）。
pub(crate) fn emit_todo_updated(raw_body: &[u8]) -> Effect {
    let items = parse_todo_items(raw_body);
    emit_todo(json!({ "items": items }))
}

/// 解析 `queryTodoList` 响应体 → todo item 列表（行为真源 `channel.rs:252-296`）。
///
/// 边界零信任：坏 JSON / 非 SUCCESS / 结构缺失 → 空 Vec（不 panic，前端拿空列表无害）。
fn parse_todo_items(raw_body: &[u8]) -> Vec<Value> {
    let Ok(resp) = serde_json::from_slice::<Value>(raw_body) else {
        return Vec::new();
    };
    // status 门控（真源 channel.rs:252-253）：SUCCESS 字符串 或 200 数字 才装配。
    let ok = resp.get("status").and_then(Value::as_str) == Some("SUCCESS")
        || resp.get("status").and_then(Value::as_i64) == Some(200);
    if !ok {
        return Vec::new();
    }
    let channels = resp
        .get("data")
        .and_then(|d| d.get("channels"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::new();
    for channel in &channels {
        let posts = channel
            .get("posts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for post in &posts {
            let message_type = post
                .get("messageType")
                .and_then(Value::as_str)
                .unwrap_or("");
            let id = post.get("id").and_then(Value::as_str).unwrap_or("");
            // id 装配契约（真源 channel.rs:288）：`"{postId}_{messageType}"`。
            items.push(json!({
                "id": format!("{id}_{message_type}"),
                "channel": channel,
                "post": post,
                "type": message_type,
                // canDel：仅 mention 可删（真源 channel.rs:292）。
                "canDel": message_type == "mention",
            }));
        }
    }
    items
}

/// 内部：把 `data` 包成 `im:todo:updated` 信封 Effect::Emit。
fn emit_todo(data: Value) -> Effect {
    let payload = json!({ "event": "im:todo:updated", "data": data });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_todo: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

#[cfg(test)]
#[path = "todo_tests.rs"]
mod tests;
