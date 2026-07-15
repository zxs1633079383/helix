use crate::sync_session::PostFields;

/// 从已在手的 `Value`（root + post 视图）一次性提取落库 owned 字段（HX-C005）。
///
/// 字段映射真源 = cses-client `From<types::Post> for Message`：
///   - post 字段可能在顶层，也可能嵌套在 `post` / `data` 子对象下——按优先级探查。
///   - 兼容 snake_case / camelCase（Go wire 多 camelCase；既有测试帧用 snake_case）。
///   - 全程安全回退不 panic（边界零信任，helix-im 不变量 4）。
///
/// 调用方（`parse_inbound` / `parse_channel_events` / ws `post` handler）此时 `Value` 已解析完成
/// （用于取 seq/type），复用同一棵树提取字段——避免落库路径（`event_to_upsert_op`）再
/// `from_slice(raw)` 重解析。
pub(crate) fn extract_post_fields(root: &serde_json::Value) -> PostFields {
    // post 字段可能在顶层，也可能嵌套在 "post" / "data" 子对象下——按优先级探查。
    let post = root
        .get("post")
        .or_else(|| root.get("data"))
        .unwrap_or(root);

    // 取字符串字段：先 post 视图，回退 root 顶层（兼容扁平 wire）。
    let pick_str = |key: &str| -> Option<String> {
        post.get(key)
            .and_then(|v| v.as_str())
            .or_else(|| root.get(key).and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };

    let temporary_id = pick_str("temporary_id")
        .or_else(|| pick_str("temporaryId"))
        .unwrap_or_default();
    let id = pick_str("id").unwrap_or_default();
    // channel_id 此处可能为空（缺省）；event_to_upsert_op 回退到 envelope 权威 channel_id。
    let channel_id = pick_str("channel_id")
        .or_else(|| pick_str("channelId"))
        .unwrap_or_default();
    let user_id = pick_str("user_id")
        .or_else(|| pick_str("userId"))
        .unwrap_or_default();
    // type 空→"TEXT"（render-ready 下沉·C013 业务默认下沉 helix·与 send_build.rs:165 一致·#53）
    let msg_type = pick_str("type")
        .or_else(|| pick_str("post_type"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "TEXT".to_string());
    let message = pick_str("message").unwrap_or_default();
    let simple_message = pick_str("simple_message")
        .or_else(|| pick_str("simpleMessage"))
        .unwrap_or_else(|| message.clone());
    let create_at = post
        .get("create_at")
        .or_else(|| post.get("createAt"))
        .or_else(|| root.get("create_at"))
        .or_else(|| root.get("createAt"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // props 保留服务端原始对象（业务扩展 / event_seq / read map 下游对账）+ UC-1.8 顶层 quickReply
    // emoji 反应合并进 props.quickReply（逻辑外提收口行数基线，详见 ws::parser::post_props）。
    let props = super::super::post_props::merge_props_with_quickreply(post, root);
    let user_snapshot = post
        .get("userSnapshot")
        .or_else(|| post.get("user_snapshot"))
        .or_else(|| root.get("userSnapshot"))
        .or_else(|| root.get("user_snapshot"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    // read_bits（C2 / type=6 真源 post.rs:1088 `readBits`）：服务端权威已读位字符串。
    // 兼容 readBits（Go camelCase wire）/ read_bits（snake 测试帧）；缺省空串。
    let read_bits = pick_str("readBits")
        .or_else(|| pick_str("read_bits"))
        .unwrap_or_default();
    // viewers（A3/CAP-9 可见性受众）：post 子树或 root 顶层的字符串数组；缺省空 Vec（无堆分配）。
    // 元素非字符串项跳过（边界零信任）。
    let viewers = post
        .get("viewers")
        .or_else(|| root.get("viewers"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mentions = post
        .get("mentions")
        .or_else(|| root.get("mentions"))
        .map(collect_string_values)
        .unwrap_or_default();
    let expedite_map = post
        .get("expediteMap")
        .or_else(|| post.get("expedite_map"))
        .or_else(|| root.get("expediteMap"))
        .or_else(|| root.get("expedite_map"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let topic = post
        .get("topic")
        .or_else(|| root.get("topic"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let reply_id = pick_str("replyId")
        .or_else(|| pick_str("reply_id"))
        .unwrap_or_default();
    let reply_root_id = pick_str("replyRootId")
        .or_else(|| pick_str("reply_root_id"))
        .unwrap_or_default();
    let reply_first_level_id = pick_str("replyFirstLevelId")
        .or_else(|| pick_str("reply_first_level_id"))
        .unwrap_or_default();
    let replied_message = post
        .get("repliedMessage")
        .or_else(|| post.get("replied_message"))
        .or_else(|| root.get("repliedMessage"))
        .or_else(|| root.get("replied_message"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let reply_messages_value = post
        .get("replyMessages")
        .or_else(|| post.get("reply_messages"))
        .or_else(|| root.get("replyMessages"))
        .or_else(|| root.get("reply_messages"));
    let reply_messages = reply_messages_value
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let wire_reply_count = post
        .get("replyCount")
        .or_else(|| post.get("reply_count"))
        .or_else(|| root.get("replyCount"))
        .or_else(|| root.get("reply_count"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    // replyMessages 非空而 replyCount 缺失/为 0 是服务端旧表 NULL 默认值造成的矛盾行。
    // 预览容器长度是权威总数的下界，parser 在数据已在手时 O(1) 归一，避免前端补算。
    let reply_count = wire_reply_count.max(json_container_len(reply_messages_value));

    PostFields {
        temporary_id,
        id,
        channel_id,
        user_id,
        msg_type,
        message,
        simple_message,
        props,
        user_snapshot,
        create_at,
        read_bits,
        viewers,
        mentions,
        expedite_map,
        topic,
        reply_id,
        reply_root_id,
        reply_first_level_id,
        replied_message,
        reply_messages,
        reply_count,
    }
}

fn json_container_len(value: Option<&serde_json::Value>) -> i64 {
    match value {
        Some(serde_json::Value::Array(items)) => items.len() as i64,
        Some(serde_json::Value::Object(items)) => items.len() as i64,
        _ => 0,
    }
}

fn collect_string_values(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::extract_post_fields;
    use serde_json::json;

    #[test]
    fn reply_preview_is_a_lower_bound_when_wire_count_is_missing() {
        let fields = extract_post_fields(&json!({
            "id": "root-1",
            "replyCount": 0,
            "replyMessages": {
                "reply-1": {"message": "一"},
                "reply-2": {"message": "二"}
            }
        }));

        assert_eq!(fields.reply_count, 2);
    }

    #[test]
    fn authoritative_wire_reply_count_is_not_reduced_to_preview_size() {
        let fields = extract_post_fields(&json!({
            "id": "root-2",
            "replyCount": 9,
            "replyMessages": {"reply-1": {"message": "预览"}}
        }));

        assert_eq!(fields.reply_count, 9);
    }
}
