//! G-04 撤回最后一条消息时的 channel 与当前 viewer 绝对态写集。

use crate::state::ChannelId;
use helix_core::effect::{BatchUpdateSpec, SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

/// 同一原子批次更新公共 channel 与当前 viewer 的 lastPost，不改变 unread_count。
pub fn message_v3_revoke_channel_ops(
    channel_id: ChannelId,
    auth_user_id: &str,
    last_post: &Value,
) -> Vec<StorageOp> {
    let encoded = last_post.to_string();
    let create_at = last_post
        .get("createAt")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    vec![
        StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
            patch: vec![
                ("last_post".to_string(), SqlValue::Text(encoded.clone())),
                ("last_post_at".to_string(), SqlValue::Integer(create_at)),
                (
                    "last_root_post_at".to_string(),
                    SqlValue::Integer(create_at),
                ),
            ],
        }),
        StorageOp::BatchUpsert(UpsertSpec {
            table: "channel_member",
            rows: vec![vec![
                (
                    "channel_id".to_string(),
                    SqlValue::Text(channel_id.as_str().to_string()),
                ),
                (
                    "user_id".to_string(),
                    SqlValue::Text(auth_user_id.to_string()),
                ),
                ("last_post".to_string(), SqlValue::Text(encoded)),
                ("last_post_at".to_string(), SqlValue::Integer(create_at)),
                (
                    "last_root_post_at".to_string(),
                    SqlValue::Integer(create_at),
                ),
            ]],
            conflict_key: Some("channel_id,user_id"),
            exclude_from_update: Vec::new(),
        }),
    ]
}
