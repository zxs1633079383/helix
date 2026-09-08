//! Channel 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造频道创建后的权威快照。
pub fn created(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:created", data)
}

/// 表达建群命令的失败结果；仅请求关联和authority错误，不含伪频道事实。
pub fn create_failed(
    request_id: &str,
    reason: &str,
    message: &str,
) -> Result<MessageV3Event, crate::ImError> {
    super::encode(
        "im:channel:create-failed",
        serde_json::json!({"requestId":request_id,"reason":reason,"message":message}),
    )
}

/// 构造频道 roster 权威快照。
pub fn members(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:members", data)
}

/// 构造频道及当前 viewer 绝对态更新。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:update", data)
}

/// 构造当前 viewer 的频道列表绝对态。
pub fn list(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:list", data)
}

/// 构造频道关闭终态。
pub fn closed(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:closed", data)
}

/// 构造话题频道创建事实。
pub fn topic_created(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:topic-created", data)
}

/// 构造频道设置权威更新。
pub fn settings_updated(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:settings-updated", data)
}

/// 从已持久化的 HTTP/WS authority 提取真实频道设置字段。
pub fn settings_from_authority(
    channel_id: &str,
    authority: &Value,
) -> Result<Option<MessageV3Event>, crate::ImError> {
    let settings = authority
        .get("settings")
        .filter(|value| value.is_object())
        .unwrap_or(authority);
    let Some(version) = authority
        .get("settingVersion")
        .or_else(|| authority.get("setting_version"))
        .or_else(|| settings.get("settingVersion"))
        .or_else(|| settings.get("setting_version"))
        .cloned()
    else {
        return Ok(None);
    };
    let mut data = serde_json::Map::new();
    data.insert(
        "channelId".to_string(),
        Value::String(channel_id.to_string()),
    );
    data.insert("settingVersion".to_string(), version);
    for key in [
        "displayName",
        "header",
        "purpose",
        "notice",
        "orient",
        "module",
        "source",
        "pictureType",
        "picture",
    ] {
        if let Some(value) = settings.get(key) {
            data.insert(key.to_string(), value.clone());
        }
    }
    settings_updated(Value::Object(data)).map(Some)
}
