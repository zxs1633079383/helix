//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

use crate::sync_session::{EventEnvelope, EventKind, SyncResponse, ChannelSnapshot};
use crate::state::{ChannelId, Seq};
use crate::error::ImError;

/// 解析 WS 入站帧为 EventEnvelope。
///
/// 期望 JSON 格式：
/// ```json
/// {
///   "event": "posted",
///   "data": {
///     "channel_id": 123,
///     "seq": 11,
///     "event_type": 1,
///     "msg_id": "msg_11",
///     "actor_id": "user_1",
///     "created_at": 1000000,
///     "payload": null
///   }
/// }
/// ```
///
/// 失败时返回 ImError::Parse（外部输入 fail-fast，不 unwrap）。
pub fn parse_inbound(bytes: &[u8]) -> Result<EventEnvelope, ImError> {
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| ImError::Parse(format!("invalid JSON: {}", e)))?;

    let data = v.get("data")
        .ok_or_else(|| ImError::Parse("missing 'data' field".to_string()))?;

    let channel_id = data["channel_id"].as_u64()
        .ok_or_else(|| ImError::Parse("missing data.channel_id".to_string()))?;

    let seq = data["seq"].as_u64()
        .ok_or_else(|| ImError::Parse("missing data.seq".to_string()))?;

    let event_type = data["event_type"].as_u64().unwrap_or(1) as u8;

    let kind = parse_event_kind(event_type);
    let raw = bytes::Bytes::copy_from_slice(bytes);

    Ok(EventEnvelope::new(ChannelId(channel_id), Seq(seq), kind, raw))
}

/// 解析 sync/notify HTTP 响应为 SyncResponse。
///
/// 4 种形态之一：
/// - `{ "type": "no_change" }`
/// - `{ "type": "events", "events": [...], "needs_continuation": false }`
/// - `{ "type": "too_long", "reset_to": 100 }`
/// - `{ "type": "snapshot", "channel_id": 1, "reset_to": 50, "messages": [...] }`
pub fn parse_sync_response(bytes: &[u8]) -> Result<SyncResponse, ImError> {
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| ImError::Parse(format!("sync response invalid JSON: {}", e)))?;

    let resp_type = v["type"].as_str()
        .ok_or_else(|| ImError::Parse("sync response missing 'type'".to_string()))?;

    match resp_type {
        "no_change" => Ok(SyncResponse::NoChange),

        "events" => {
            let needs_continuation = v["needs_continuation"].as_bool().unwrap_or(false);
            let raw_events = v["events"].as_array()
                .ok_or_else(|| ImError::Parse("'events' field missing or not array".to_string()))?;

            let mut events = Vec::with_capacity(raw_events.len());
            for ev_json in raw_events {
                let channel_id = ev_json["channel_id"].as_u64()
                    .ok_or_else(|| ImError::Parse("event missing channel_id".to_string()))?;
                let seq = ev_json["seq"].as_u64()
                    .ok_or_else(|| ImError::Parse("event missing seq".to_string()))?;
                let event_type = ev_json["event_type"].as_u64().unwrap_or(1) as u8;
                let kind = parse_event_kind(event_type);
                let raw = bytes::Bytes::from(serde_json::to_vec(ev_json).unwrap_or_default());
                events.push(EventEnvelope::new(ChannelId(channel_id), Seq(seq), kind, raw));
            }
            // 分桶排序（type1 → type2 → type3 → type6，铁律）
            sort_events_by_bucket(&mut events);

            Ok(SyncResponse::Events { events, needs_continuation })
        }

        "too_long" => {
            let reset_to = v["reset_to"].as_u64()
                .ok_or_else(|| ImError::Parse("too_long missing reset_to".to_string()))?;
            Ok(SyncResponse::TooLong { reset_to: Seq(reset_to) })
        }

        "snapshot" => {
            let channel_id = v["channel_id"].as_u64()
                .ok_or_else(|| ImError::Parse("snapshot missing channel_id".to_string()))?;
            let reset_to = v["reset_to"].as_u64()
                .ok_or_else(|| ImError::Parse("snapshot missing reset_to".to_string()))?;
            let raw_messages = v["messages"].as_array()
                .ok_or_else(|| ImError::Parse("snapshot missing messages".to_string()))?;

            let mut messages = Vec::with_capacity(raw_messages.len());
            for msg_json in raw_messages {
                let ch_id = msg_json["channel_id"].as_u64().unwrap_or(channel_id);
                let seq = msg_json["seq"].as_u64()
                    .ok_or_else(|| ImError::Parse("snapshot message missing seq".to_string()))?;
                let event_type = msg_json["event_type"].as_u64().unwrap_or(1) as u8;
                let kind = parse_event_kind(event_type);
                let raw = bytes::Bytes::from(serde_json::to_vec(msg_json).unwrap_or_default());
                messages.push(EventEnvelope::new(ChannelId(ch_id), Seq(seq), kind, raw));
            }

            Ok(SyncResponse::Snapshot(ChannelSnapshot {
                channel_id: ChannelId(channel_id),
                reset_to: Seq(reset_to),
                messages,
            }))
        }

        other => Err(ImError::Parse(format!("unknown sync response type: {}", other))),
    }
}

/// 从 WS type 数字解析 EventKind
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
