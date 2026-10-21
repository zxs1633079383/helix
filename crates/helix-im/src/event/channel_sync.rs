//! MessageV3 频道同步分页事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造批量落库成功后的轻量会话就绪事件；不得携带频道数组。
pub(crate) fn ready(
    channel_sync_session_id: &str,
    generation: u64,
) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        "im:channel-sync-ready",
        serde_json::json!({
            "channelSyncSessionId": channel_sync_session_id,
            "generation": generation,
            "defaultPageSize": crate::channel_sync::PAGE_SIZE,
        }),
    )
}

/// 构造一个有界频道页；items 只包含当前页，不允许嵌套快照或旧事件数组。
pub(crate) fn page(
    channel_sync_session_id: &str,
    generation: u64,
    items: Vec<Value>,
    has_more: bool,
    next_cursor: Option<&str>,
) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        "im:channel-sync-page",
        serde_json::json!({
            "channelSyncSessionId": channel_sync_session_id,
            "generation": generation,
            "items": items,
            "hasMore": has_more,
            "nextCursor": next_cursor,
        }),
    )
}

/// 构造分页会话关闭事件。
pub(crate) fn complete(
    channel_sync_session_id: &str,
    generation: u64,
) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        "im:channel-sync-complete",
        serde_json::json!({
            "channelSyncSessionId": channel_sync_session_id,
            "generation": generation,
        }),
    )
}

/// 构造话题增量落库成功后的轻量 ready，不晋升普通频道列表会话。
pub(crate) fn subtopics_ready(
    parent_channel_id: &str,
    request_id: Option<&str>,
    snapshot_complete: bool,
) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        "im:subtopics-sync-ready",
        serde_json::json!({
            "channelId": parent_channel_id,
            "requestId": request_id,
            "snapshotComplete": snapshot_complete,
        }),
    )
}
