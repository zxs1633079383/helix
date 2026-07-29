//! G-06 reaction 的单列确定性写模型。

use helix_core::effect::{BatchUpdateSpec, SqlValue, StorageOp};

/// 只覆盖服务端权威 quick_reply，避免 reaction 误写普通编辑或本地状态列。
pub fn message_v3_reaction_op(message_id: &str, quick_reply: String) -> StorageOp {
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(message_id.to_string())],
        patch: vec![("quick_reply".to_string(), SqlValue::Text(quick_reply))],
    })
}
