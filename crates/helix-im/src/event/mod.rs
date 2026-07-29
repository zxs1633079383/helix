//! MessageV3 领域事件唯一构造入口。

use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;
use serde::Serialize;
use serde_json::Value;

pub mod channel;
pub mod channel_member;
pub mod post;
pub mod read;
pub mod schedule;
pub mod sync;
pub mod timeline;

/// 已编码但尚未跨端发布的 MessageV3 业务事件。
#[derive(Debug, Clone)]
pub struct MessageV3Event {
    bytes: DomainEventBytes,
}

#[derive(Serialize)]
struct MessageV3Envelope {
    event: &'static str,
    data: Value,
}

impl MessageV3Event {
    /// 构造只含 `event` 与对象 `data` 的业务信封。
    pub fn new(event: &'static str, data: Value) -> Result<Self, crate::ImError> {
        if !data.is_object() {
            return Err(crate::ImError::Parse(format!(
                "{event} MessageV3 data must be an object"
            )));
        }
        let bytes = serde_json::to_vec(&MessageV3Envelope { event, data })
            .map_err(|error| crate::ImError::Parse(error.to_string()))?;
        Ok(Self {
            bytes: DomainEventBytes(Bytes::from(bytes)),
        })
    }

    /// 把类型化事件降到 core 唯一跨端业务 Effect。
    pub fn into_effect(self) -> Effect {
        Effect::Emit { event: self.bytes }
    }

    /// 在 Persist barrier 中移动已编码字节，避免二次序列化。
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes.0.to_vec()
    }
}

/// 把静态领域事件名与业务数据编码成 MessageV3 信封。
pub(crate) fn encode(event: &'static str, data: Value) -> Result<MessageV3Event, crate::ImError> {
    MessageV3Event::new(event, data)
}
