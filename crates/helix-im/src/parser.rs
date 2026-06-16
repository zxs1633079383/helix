//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::sync_session::{ChannelSnapshot, EventEnvelope, EventKind, SyncResponse};

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

    let data = v
        .get("data")
        .ok_or_else(|| ImError::Parse("missing 'data' field".to_string()))?;

    let channel_id = data["channel_id"]
        .as_u64()
        .ok_or_else(|| ImError::Parse("missing data.channel_id".to_string()))?;

    let seq = data["seq"]
        .as_u64()
        .ok_or_else(|| ImError::Parse("missing data.seq".to_string()))?;

    let event_type = data["event_type"].as_u64().unwrap_or(1) as u8;

    let kind = parse_event_kind(event_type);
    let raw = bytes::Bytes::copy_from_slice(bytes);

    Ok(EventEnvelope::new(
        ChannelId(channel_id),
        Seq(seq),
        kind,
        raw,
    ))
}

/// A4：识别 hello 握手帧——顶层 `action=="hello"` 时返回 `data.connectionId`，否则 None。
///
/// 现网 Go 服务端连上后**单向下发** `{"action":"hello","data":{"connectionId",...}}`，
/// 客户端收到即握手完成（无需回 ack）。非 hello 帧返回 None（调用方继续走事件解析路径）。
/// 边界输入零信任（HX-C005）：JSON 非法 / 缺字段一律 None，绝不 panic / unwrap。
pub fn parse_hello(bytes: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if v.get("action").and_then(|a| a.as_str()) != Some("hello") {
        return None;
    }
    v.get("data")
        .and_then(|d| d.get("connectionId"))
        .and_then(|c| c.as_str())
        .map(str::to_string)
}

/// 入站帧按真 Go 信封顶层 `action` 分类的结果。
///
/// 真 Go 信封实测形态：`{"v":1,"action":"<type>","data":{...},"broadcast":{...},"seq":<信封序号>}`
/// （`action` 在顶层、`seq` 是信封序号非 event_seq；样本见 tests/fixtures/real_go_frames/）。
#[derive(Debug, PartialEq, Eq)]
pub enum FrameClass {
    /// hello 握手帧，携带 connectionId（A4 握手）。
    Hello(String),
    /// 运维/状态/真 Go action 帧——当前优雅忽略（不报错、不刷日志）。
    ///
    /// 现网 status_change / multiple_channels_viewed / typing / presence 等本就可丢；
    /// 而 increment_channel / posted 等真事件帧的**真实处理**需先做 `ChannelId u64→String`
    /// 迁移（真 Go channelId 是 26 字符串，helix 当前是 u64）——该迁移是 Phase B 余下部分的
    /// 前置阻塞项，未完成前这些帧归为 Ignored 不误报。
    Ignored,
    /// legacy `{event,data}` 数值帧（抽象 wire / 既有测试）——走 `parse_inbound` 事件路径。
    Event,
}

/// 按真 Go 信封顶层 `action` 分类入站帧。零信任：非法 JSON / 缺字段 → Ignored（不 panic）。
///
/// - `action=="hello"` → `Hello(connectionId)`
/// - 其它 `action`（status_change / increment_channel / posted 等真 Go 帧）→ `Ignored`
///   （真实处理待 ChannelId u64→String 迁移；当前不误报）
/// - 无 `action`（legacy `{event,data}` 帧）→ `Event`（既有数值事件路径）
pub fn classify_frame(bytes: &[u8]) -> FrameClass {
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return FrameClass::Ignored,
    };
    match v.get("action").and_then(|a| a.as_str()) {
        Some("hello") => v
            .get("data")
            .and_then(|d| d.get("connectionId"))
            .and_then(|c| c.as_str())
            .map(|id| FrameClass::Hello(id.to_string()))
            .unwrap_or(FrameClass::Ignored),
        Some(_) => FrameClass::Ignored,
        None => FrameClass::Event,
    }
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

    let resp_type = v["type"]
        .as_str()
        .ok_or_else(|| ImError::Parse("sync response missing 'type'".to_string()))?;

    match resp_type {
        "no_change" => Ok(SyncResponse::NoChange),

        "events" => {
            let needs_continuation = v["needs_continuation"].as_bool().unwrap_or(false);
            let raw_events = v["events"]
                .as_array()
                .ok_or_else(|| ImError::Parse("'events' field missing or not array".to_string()))?;

            let mut events = Vec::with_capacity(raw_events.len());
            for ev_json in raw_events {
                let channel_id = ev_json["channel_id"]
                    .as_u64()
                    .ok_or_else(|| ImError::Parse("event missing channel_id".to_string()))?;
                let seq = ev_json["seq"]
                    .as_u64()
                    .ok_or_else(|| ImError::Parse("event missing seq".to_string()))?;
                let event_type = ev_json["event_type"].as_u64().unwrap_or(1) as u8;
                let kind = parse_event_kind(event_type);
                let raw = bytes::Bytes::from(serde_json::to_vec(ev_json).unwrap_or_default());
                events.push(EventEnvelope::new(
                    ChannelId(channel_id),
                    Seq(seq),
                    kind,
                    raw,
                ));
            }
            // 分桶排序（type1 → type2 → type3 → type6，铁律）
            sort_events_by_bucket(&mut events);

            Ok(SyncResponse::Events {
                events,
                needs_continuation,
            })
        }

        "too_long" => {
            let reset_to = v["reset_to"]
                .as_u64()
                .ok_or_else(|| ImError::Parse("too_long missing reset_to".to_string()))?;
            Ok(SyncResponse::TooLong {
                reset_to: Seq(reset_to),
            })
        }

        "snapshot" => {
            let channel_id = v["channel_id"]
                .as_u64()
                .ok_or_else(|| ImError::Parse("snapshot missing channel_id".to_string()))?;
            let reset_to = v["reset_to"]
                .as_u64()
                .ok_or_else(|| ImError::Parse("snapshot missing reset_to".to_string()))?;
            let raw_messages = v["messages"]
                .as_array()
                .ok_or_else(|| ImError::Parse("snapshot missing messages".to_string()))?;

            let mut messages = Vec::with_capacity(raw_messages.len());
            for msg_json in raw_messages {
                let ch_id = msg_json["channel_id"].as_u64().unwrap_or(channel_id);
                let seq = msg_json["seq"]
                    .as_u64()
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

        other => Err(ImError::Parse(format!(
            "unknown sync response type: {}",
            other
        ))),
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
        EventKind::PostEdit => 1,
        EventKind::PostRevoke => 2,
        EventKind::PostRead => 3,
        EventKind::Other(_) => 4,
    });
}
