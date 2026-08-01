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
    extract_post_fields_with_event_seq(root, None)
}

/// Extracts the same owned post view while injecting a trusted envelope event sequence into props.
pub(crate) fn extract_post_fields_with_event_seq(
    root: &serde_json::Value,
    event_seq: Option<u64>,
) -> PostFields {
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
    let user_id = post
        .get("userSnapshot")
        .or_else(|| post.get("user_snapshot"))
        .or_else(|| root.get("userSnapshot"))
        .or_else(|| root.get("user_snapshot"))
        .and_then(|snapshot| {
            snapshot
                .get("userId")
                .or_else(|| snapshot.get("user_id"))
                .and_then(|value| value.as_str())
        })
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| pick_str("user_id").filter(|value| !value.is_empty()))
        .or_else(|| pick_str("userId").filter(|value| !value.is_empty()))
        // Historical reconnect/system rows use createBy=SYS while the old
        // user snapshot is empty. Preserve that authoritative identity in the
        // typed field so both the immediate event and the SQLite readback stay
        // renderable.
        .or_else(|| pick_str("create_by").filter(|value| !value.is_empty()))
        .or_else(|| pick_str("createBy").filter(|value| !value.is_empty()))
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
    let update_at = post
        .get("update_at")
        .or_else(|| post.get("updateAt"))
        .or_else(|| root.get("update_at"))
        .or_else(|| root.get("updateAt"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // props 只保留服务端业务对象与受信 event_seq；quickReply 使用独立 durable 列。
    let props = super::super::post_props::props_with_event_seq(post, root, event_seq);
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
    let snapshot_id = pick_str("snapshotId")
        .or_else(|| pick_str("snapshot_id"))
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
    let quick_reply = post
        .get("quickReply")
        .or_else(|| post.get("quick_reply"))
        .or_else(|| root.get("quickReply"))
        .or_else(|| root.get("quick_reply"))
        .or_else(|| post.get("props").and_then(|props| props.get("quickReply")))
        .or_else(|| post.get("props").and_then(|props| props.get("quick_reply")))
        .or_else(|| root.get("props").and_then(|props| props.get("quickReply")))
        .or_else(|| root.get("props").and_then(|props| props.get("quick_reply")))
        .and_then(normalize_quick_reply)
        .map(|items| items.to_string())
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
        update_at,
        read_bits,
        snapshot_id,
        viewers,
        mentions,
        expedite_map,
        quick_reply,
        topic,
        reply_id,
        reply_root_id,
        reply_first_level_id,
        replied_message,
        reply_messages,
        reply_count,
    }
}

/// 把 Go 存储态 emoji map 与 wire 数组态统一成稳定的 reaction 绝对态数组。
fn normalize_quick_reply(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => Some(serde_json::Value::Array(items.clone())),
        serde_json::Value::Object(items) => {
            let mut emojis = items.iter().collect::<Vec<_>>();
            emojis.sort_by(|left, right| left.0.cmp(right.0));
            Some(serde_json::Value::Array(
                emojis
                    .into_iter()
                    .map(|(emoji, user_ids)| {
                        serde_json::json!({
                            "emoji": emoji,
                            "userIds": user_ids,
                        })
                    })
                    .collect(),
            ))
        }
        serde_json::Value::String(raw) => serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .as_ref()
            .and_then(normalize_quick_reply),
        _ => None,
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

    #[test]
    fn historical_system_creator_becomes_persisted_author_identity() {
        let fields = extract_post_fields(&json!({
            "id": "legacy-reconnect",
            "userSnapshot": { "userId": "" },
            "createBy": "SYS"
        }));

        assert_eq!(fields.user_id, "SYS");
    }

    /// Keeps remote create/update clocks distinct so offline storage can be byte-for-byte authoritative.
    #[test]
    fn remote_update_at_is_not_collapsed_into_create_at() {
        let fields = extract_post_fields(&json!({
            "id": "post-clock",
            "createAt": 1_000,
            "updateAt": 1_025
        }));

        assert_eq!(fields.create_at, 1_000);
        assert_eq!(fields.update_at, 1_025);
    }

    /// Go post authority 的 snapshotId 必须进入 typed fields，不能在 WS parser 丢失。
    #[test]
    fn snapshot_id_is_extracted_from_camel_and_snake_wire() {
        let camel = extract_post_fields(&json!({"id":"post-1","snapshotId":"snapshot-1"}));
        let snake = extract_post_fields(&json!({"id":"post-2","snapshot_id":"snapshot-2"}));

        assert_eq!(camel.snapshot_id, "snapshot-1");
        assert_eq!(snake.snapshot_id, "snapshot-2");
    }

    /// 兼容运行面把 reaction 放进 props 的旧帧，并仍提升为独立 durable quick_reply。
    #[test]
    fn nested_quick_reply_is_promoted_to_the_reaction_column() {
        let fields = extract_post_fields(&json!({
            "id": "post-reaction",
            "props": {
                "quickReply": [{"emoji":"thumb","userIds":["member-1"]}]
            }
        }));

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&fields.quick_reply).unwrap(),
            json!([{"emoji":"thumb","userIds":["member-1"]}])
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&fields.props)
                .unwrap()
                .get("quickReply")
                .is_none(),
            "reaction must not remain duplicated inside props"
        );
    }

    /// 运行面仍回传 Go jsonb map 时必须归一为 Angular 消费的稳定数组。
    #[test]
    fn quick_reply_map_is_normalized_to_a_sorted_array() {
        let fields = extract_post_fields(&json!({
            "id": "post-map-reaction",
            "quick_reply": {
                "wave": ["member-2"],
                "thumb": ["member-1"]
            }
        }));

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&fields.quick_reply).unwrap(),
            json!([
                {"emoji":"thumb","userIds":["member-1"]},
                {"emoji":"wave","userIds":["member-2"]}
            ])
        );
    }

    /// 历史 WS codec 把 RawMessage 再包成 JSON 字符串时仍须恢复 reaction 绝对态。
    #[test]
    fn string_encoded_quick_reply_is_decoded_once() {
        let fields = extract_post_fields(&json!({
            "id": "post-string-reaction",
            "quickReply": "[{\"emoji\":\"thumb\",\"userIds\":[\"member-1\"]}]"
        }));

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&fields.quick_reply).unwrap(),
            json!([{"emoji":"thumb","userIds":["member-1"]}])
        );
    }
}
