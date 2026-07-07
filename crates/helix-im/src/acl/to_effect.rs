//! IM 概念 → core Effect 翻译（ACL-1 写出端）
//!
//! 这是兼容 facade：外部调用继续走 `acl::to_effect::*`，具体 Effect 工厂按职责拆到
//! sibling module，避免单文件重新聚合 transport/storage/sync/projection 多类变更理由。

pub use super::projection_control_effects::{
    emit_channel_increment, emit_channel_update, emit_channel_update_by_post, emit_channels_loaded,
    emit_connection_established, emit_post_batch_updated, emit_post_send_failed, emit_post_sending,
    emit_reconnect_requested, emit_sync_too_long,
};
pub use super::projection_message_effects::{
    emit_channel_read_echo, emit_post_deleted, emit_post_read, emit_post_received,
    emit_post_updated,
};
pub use super::storage_effects::{
    advance_cursor, bump_channel_unread, bump_channel_unread_op, update_channel_partial,
    upsert_channel_full, upsert_message,
};
pub use super::sync_effects::{
    batch_upsert_events, batch_upsert_events_with_messages, sync_mutation_emits,
    sync_unread_bump_ops,
};
pub use super::sync_http_effects::{connection_id_headers, increment_http_trigger, sync_notify};
pub use super::transport_effects::ping_frame;
