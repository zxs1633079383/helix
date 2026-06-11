//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

use crate::sync_session::{EventEnvelope, EventKind};
use crate::state::{ChannelId, Seq};
use crate::error::ImError;

/// 解析 WS 入站帧为 EventEnvelope。
///
/// 失败时返回 ImError::Parse（外部输入 fail-fast，不 unwrap）。
pub fn parse_inbound(bytes: &[u8]) -> Result<EventEnvelope, ImError> {
    // TODO: 实现真实的 JSON 解析
    // wire 格式来自 Mattermost/cses WS 协议
    // {
    //   "event": "posted",
    //   "data": { "channel_id": "...", "seq": 123, "type": 1, ... },
    //   ...
    // }
    let _ = bytes;
    Err(ImError::Parse("TODO: implement parse_inbound".to_string()))
}

/// 从 JSON 对象解析 EventKind（type 字段）
fn parse_event_kind(type_num: u8) -> EventKind {
    match type_num {
        1 => EventKind::PostUpsert,
        2 => EventKind::PostEdit,
        3 => EventKind::PostRevoke,
        6 => EventKind::PostRead,
        n => EventKind::Other(n),
    }
}

/// 分桶顺序（铁律，来自源码事实 2026-06-08）
///
/// 处理顺序必须严格：type1 → type2 → type3 → type6
/// 不得乱序，否则业务状态不一致。
pub fn sort_events_by_bucket(events: &mut Vec<EventEnvelope>) {
    events.sort_by_key(|ev| match ev.kind {
        EventKind::PostUpsert => 0u8,
        EventKind::PostEdit   => 1,
        EventKind::PostRevoke => 2,
        EventKind::PostRead   => 3,
        EventKind::Other(_)   => 4,
    });
}
