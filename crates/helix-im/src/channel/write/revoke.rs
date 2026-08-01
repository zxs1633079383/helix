//! G-04 撤回最后一条消息时的 channel 与当前 viewer 绝对态写集。

use crate::state::ChannelId;
use helix_core::effect::{BatchUpdateSpec, Row, SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

/// 同一原子批次更新公共 channel 与当前 viewer 的 lastPost，不改变 unread_count。
///
/// `last_post` 是 Go `update_channel` 广播的**撤回后摘要权威**：撤回的若是频道末条，
/// Go 会把摘要退回「从尾部往前第一条未撤回消息」再广播；整频道全撤回时才回传撤回态本条。
/// Helix 只做逐字段透传持久化，**不**在本地二次推导摘要（对齐 MV3-G05a contract
/// `lastPostOrder` 与 INV-01）。
///
/// `createAt` 缺失或非整数时**不写** `last_post_at` / `last_root_post_at`：撤回是 partial
/// patch，把缺省字段并成 0 会清空本地既有时间戳并破坏会话列表排序
/// （MV3-G05a contract negativeAssertions「spread 前先 filter undefined」）。
pub fn message_v3_revoke_channel_ops(
    channel_id: ChannelId,
    auth_user_id: &str,
    last_post: &Value,
) -> Vec<StorageOp> {
    let encoded = last_post.to_string();
    let create_at = last_post.get("createAt").and_then(Value::as_i64);

    let mut patch: Row = vec![("last_post".to_string(), SqlValue::Text(encoded.clone()))];
    let mut member_row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(channel_id.as_str().to_string()),
        ),
        (
            "user_id".to_string(),
            SqlValue::Text(auth_user_id.to_string()),
        ),
        ("last_post".to_string(), SqlValue::Text(encoded)),
    ];
    if let Some(create_at) = create_at {
        for target in [&mut patch, &mut member_row] {
            target.push(("last_post_at".to_string(), SqlValue::Integer(create_at)));
            target.push((
                "last_root_post_at".to_string(),
                SqlValue::Integer(create_at),
            ));
        }
    }

    vec![
        StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
            patch,
        }),
        StorageOp::BatchUpsert(UpsertSpec {
            table: "channel_member",
            rows: vec![member_row],
            conflict_key: Some("channel_id,user_id"),
            exclude_from_update: Vec::new(),
        }),
    ]
}
