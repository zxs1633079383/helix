//! G-14a 模板确认 props 的单列确定性写模型。

use helix_core::effect::{BatchUpdateSpec, SqlValue, StorageOp};

/// 只覆盖服务端权威模板 props 与版本列，不改写内容或 viewer 本地状态。
pub fn message_v3_template_op(
    message_id: &str,
    props: String,
    event_seq: u64,
    update_at: i64,
) -> StorageOp {
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(message_id.to_string())],
        patch: vec![
            ("props".to_string(), SqlValue::Text(props)),
            (
                "event_seq".to_string(),
                SqlValue::Integer(event_seq.min(i64::MAX as u64) as i64),
            ),
            ("update_at".to_string(), SqlValue::Integer(update_at)),
        ],
    })
}
