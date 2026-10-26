//! Sync 与 recovery 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造 hello 初始化完成事实。
pub fn loaded(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:sync:loaded", data)
}

/// 构造离线恢复收敛事实。
pub fn recovered(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:sync:recovered", data)
}

/// 构造 pong gap 补偿结果。
pub fn gap_repaired(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:sync:gap-repaired", data)
}

/// 构造陌生频道增量补全结果。
pub fn channel_hydrated(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:sync:channel-hydrated", data)
}
