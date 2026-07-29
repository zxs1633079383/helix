//! G-07 expedite_map 的单列确定性写模型。

use helix_core::effect::{BatchUpdateSpec, SqlValue, StorageOp};

/// 只覆盖服务端权威加急列与其版本时间，避免改写普通消息内容或 viewer 本地列。
pub fn message_v3_urgent_op(
    message_id: &str,
    expedite_map: String,
    event_seq: u64,
    update_at: i64,
) -> StorageOp {
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(message_id.to_string())],
        patch: vec![
            ("expedite_map".to_string(), SqlValue::Text(expedite_map)),
            (
                "event_seq".to_string(),
                SqlValue::Integer(event_seq.min(i64::MAX as u64) as i64),
            ),
            ("update_at".to_string(), SqlValue::Integer(update_at)),
        ],
    })
}
