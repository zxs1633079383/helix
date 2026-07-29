//! Read 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造 viewer 的频道已读绝对态。
pub fn channel(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:read:channel", data)
}

/// 构造单条消息已读事实。
pub fn post(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:read", data)
}

/// 构造消息作者可见的 reader 列表。
pub fn post_readers(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:readers", data)
}

/// 按当前 viewer 与权威 reader 的关系构造 G-14 单视角终态。
pub fn post_for_viewer(
    channel_id: &str,
    post_id: &str,
    reader_id: &str,
    read_bits: &str,
    receipt_revision: i64,
    viewer_user_id: &str,
) -> Result<MessageV3Event, crate::ImError> {
    if !reader_id.is_empty() && reader_id == viewer_user_id {
        return post(serde_json::json!({
            "id": post_id,
            "channelId": channel_id,
            "readBits": read_bits,
            "readerId": reader_id,
            "receiptRevision": receipt_revision,
        }));
    }
    post_readers(serde_json::json!({
        "id": post_id,
        "channelId": channel_id,
        "readBits": read_bits,
        "readerIds": if reader_id.is_empty() {
            Vec::<&str>::new()
        } else {
            vec![reader_id]
        },
        "receiptRevision": receipt_revision,
    }))
}
