//! Announcement 版本化绝对列表事件。
//!
//! 公告正文仍由既有 Announcement Core/Go authority 负责；本模块只把权威列表
//! 归一为一个可幂等覆盖 Angular 状态的 MessageV3 事件，不保存第二份公告集合。

use super::MessageV3Event;
use serde_json::Value;

/// 构造完整公告列表绝对投影；调用方必须已校验 channelId/version/announcements。
pub fn list_updated(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:announcement:list-updated", data)
}
