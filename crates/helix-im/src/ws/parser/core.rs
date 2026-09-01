//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

mod fields;
mod increment;
mod kind;
mod sync;

pub(crate) use fields::{extract_post_fields, extract_post_fields_with_event_seq};
pub use increment::parse_increment_channel;
pub(crate) use kind::parse_event_kind;
pub use sync::{parse_sync_response, sort_events_by_bucket, SYNC_EVENTS_PER_ENTRY};
