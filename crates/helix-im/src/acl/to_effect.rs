//! IM 概念 → core Effect 翻译（ACL-1 写出端）
//!
//! 这是兼容 facade：外部调用继续走 `acl::to_effect::*`，具体 Effect 工厂按职责拆到
//! sibling module，避免单文件重新聚合 transport/storage/sync/projection 多类变更理由。

pub use super::projection_control_effects::{
    emit_channel_increment, emit_channel_update, emit_channels_loaded, emit_connection_established,
    emit_post_batch_updated, emit_post_send_failed, emit_post_sending,
    emit_post_sending_with_reply, emit_reconnect_requested, emit_sync_state,
    emit_sync_state_with_trigger, emit_sync_too_long, emit_topic_operation_status,
    member_dialog_patch_data, post_dialog_patch_data,
};
pub use super::projection_message_effects::{
    emit_channel_read_echo, emit_channel_read_echo_for_viewer, emit_post_deleted,
    emit_post_deleted_for_viewer, emit_post_read, emit_post_read_for_viewer,
    emit_post_read_with_receipt_revision, emit_post_read_with_receipt_revision_for_viewer,
    emit_post_received, emit_post_received_for_viewer, emit_post_updated,
    emit_post_updated_for_viewer, emit_sync_post_read, emit_sync_post_read_for_viewer,
};
pub use super::storage_effects::{
    advance_cursor, advance_cursor_op, bump_channel_unread, bump_channel_unread_op,
    get_channel_row_op, recovery_coverage_op, recovery_ledger_op, recovery_tenant_actor_scope,
    reset_channel_dialog_op, terminal_tombstone_and_cursor_op, update_channel_partial,
    update_channel_partial_op, upsert_channel_full, upsert_channel_member_dialog,
    upsert_channel_member_dialog_op, upsert_message, RecoveryCoverage, RecoveryLedgerEntry,
};
pub use super::sync_effects::{
    batch_upsert_events, batch_upsert_events_with_messages,
    batch_upsert_events_with_messages_and_auth, sync_dialog_patch_plans, sync_mutation_emits,
    sync_mutation_emits_with_auth, sync_unread_bump_ops,
};
pub use super::sync_http_effects::{connection_id_headers, increment_http_trigger, sync_notify};
pub use super::transport_effects::ping_frame;
