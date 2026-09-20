use helix_core::effect::{BatchUpdateSpec, SqlValue, StorageOp, UpsertSpec};
use serde_json::Value;

use crate::state::TemporaryId;

/// Build the local optimistic message write before transport begins.
pub fn optimistic_message_persist_op(
    temporary_id: &str,
    channel_id: &str,
    body: &Value,
) -> StorageOp {
    let clean_body = crate::message_identity::sanitize(body.clone());
    let body = &clean_body;
    // 上传写凭据不进入 durable 行；失败上传必须重新获取签名。
    let json_text = |key: &str, fallback: &str| {
        body.get(key)
            .and_then(|value| serde_json::to_string(value).ok())
            .unwrap_or_else(|| fallback.to_string())
    };
    let string = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let persisted_props = body
        .get("props")
        .map(crate::send::upload_props::sanitized_for_persistence)
        .and_then(|props| serde_json::to_string(&props).ok())
        .unwrap_or_else(|| "{}".to_string());
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "message",
        rows: vec![vec![
            (
                "temporary_id".to_string(),
                SqlValue::Text(temporary_id.to_string()),
            ),
            (
                "channel_id".to_string(),
                SqlValue::Text(channel_id.to_string()),
            ),
            ("id".to_string(), SqlValue::Text(string("id"))),
            ("user_id".to_string(), SqlValue::Text(string("userId"))),
            ("team_id".to_string(), SqlValue::Text(string("teamId"))),
            ("message".to_string(), SqlValue::Text(string("message"))),
            ("type".to_string(), SqlValue::Text(string("type"))),
            ("props".to_string(), SqlValue::Text(persisted_props)),
            (
                "viewers".to_string(),
                SqlValue::Text(json_text("viewers", "[]")),
            ),
            (
                "mentions".to_string(),
                SqlValue::Text(json_text("mentions", "[]")),
            ),
            ("user_snapshot".to_string(), SqlValue::Text(String::new())),
            (
                "simple_message".to_string(),
                SqlValue::Text(string("simpleMessage")),
            ),
            ("topic".to_string(), SqlValue::Text(string("topicId"))),
            ("reply_id".to_string(), SqlValue::Text(string("replyId"))),
            (
                "reply_root_id".to_string(),
                SqlValue::Text(string("replyRootId")),
            ),
            (
                "reply_first_level_id".to_string(),
                SqlValue::Text(string("replyFirstLevelId")),
            ),
            (
                "replied_message".to_string(),
                SqlValue::Text(json_text("repliedMessage", "")),
            ),
            (
                "reply_messages".to_string(),
                SqlValue::Text(json_text("replyMessages", "[]")),
            ),
            (
                "reply_count".to_string(),
                SqlValue::Integer(
                    body.get("replyCount")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                ),
            ),
            (
                "create_at".to_string(),
                SqlValue::Integer(
                    body.get("createAt")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                ),
            ),
            (
                "send_status".to_string(),
                SqlValue::Text("sending".to_string()),
            ),
            ("upload_progress_percent".to_string(), SqlValue::Integer(0)),
        ]],
        conflict_key: Some("temporary_id"),
        exclude_from_update: Vec::new(),
    })
}

pub fn upload_progress_persist_op(temporary_id: &TemporaryId, progress_percent: u8) -> StorageOp {
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "temporary_id",
        key_vals: vec![SqlValue::Text(temporary_id.0.clone())],
        patch: vec![(
            "upload_progress_percent".to_string(),
            SqlValue::Integer(i64::from(progress_percent)),
        )],
    })
}

/// Persist only the send status for the stable temporary identity.
pub fn send_status_persist_op(temporary_id: &TemporaryId, status: &str) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec {
        version_column: None,
        update_guard: None,
        table: "message",
        rows: vec![vec![
            (
                "temporary_id".to_string(),
                SqlValue::Text(temporary_id.0.clone()),
            ),
            (
                "send_status".to_string(),
                SqlValue::Text(status.to_string()),
            ),
        ]],
        conflict_key: Some("temporary_id"),
        exclude_from_update: Vec::new(),
    })
}
