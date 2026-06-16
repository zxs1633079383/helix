//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::sync_session::{
    ChannelSnapshot, EventEnvelope, EventKind, IncrementChannel, SyncResponse,
};

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
        .as_str()
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse("missing/invalid data.channel_id".to_string()))?;

    // event_seq 三级 fallback（设计文档 §1.5，后端注入位置不统一）：
    //   ① data.event_seq（权威，真 Go int64 数值）
    //   ② per-post props.channel_event_seq（posted 帧冗余）
    //   ③ 顶层 channel_event_seq（legacy）
    // 兼容兜底：legacy data.seq（无 action 抽象 wire / 既有测试帧）。
    // 真 event_seq 是 int64 数值（Seq(u64)），非 26 字符 id；零信任只取 as_u64。
    let seq = data["event_seq"]
        .as_u64()
        .or_else(|| data["props"]["channel_event_seq"].as_u64())
        .or_else(|| v["channel_event_seq"].as_u64())
        .or_else(|| data["seq"].as_u64())
        .ok_or_else(|| {
            ImError::Parse(
                "missing event_seq (data.event_seq / props.channel_event_seq / channel_event_seq / data.seq)"
                    .to_string(),
            )
        })?;

    let event_type = data["event_type"].as_u64().unwrap_or(1) as u8;

    let kind = parse_event_kind(event_type);
    let raw = bytes::Bytes::copy_from_slice(bytes);

    Ok(EventEnvelope::new(channel_id, Seq(seq), kind, raw))
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
    /// `increment_channel` 帧（B-rest）：per-channel 增量推送，携带解析好的 IncrementChannel。
    Increment(IncrementChannel),
    /// `increment_channel_end` 帧（B-rest）：本批增量收尾信号。
    /// `Some(channel_id)`=某 parent 子 topic 批次结束；`None`=hello/重连整批 global end。
    IncrementEnd(Option<ChannelId>),
    /// 运维/状态帧——优雅忽略（不报错、不刷日志）。
    /// 现网 status_change / multiple_channels_viewed / typing / presence 等本就可丢。
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
        Some("increment_channel") => match v.get("data").and_then(parse_increment_channel) {
            Some(ic) => FrameClass::Increment(ic),
            None => FrameClass::Ignored, // 解析失败/未实现 → 不误报
        },
        Some("increment_channel_end") => {
            // data.channelId 非空=子 topic 批次结束；空/缺=global end。零信任。
            let ch = v
                .get("data")
                .and_then(|d| d.get("channelId"))
                .and_then(|c| c.as_str())
                .and_then(ChannelId::from_str);
            FrameClass::IncrementEnd(ch)
        }
        Some(_) => FrameClass::Ignored,
        None => FrameClass::Event,
    }
}

/// 解析 `increment_channel` 帧的 `data` → `IncrementChannel`（边界零信任，非法→None）。
///
/// 按设计文档 §1.3 解析真 Go `IncrementChannel.ToMap()` 的 data：
///   - `id`：26 字符 base32 channelId → `ChannelId::from_str`（零信任，非 26 字符→None）。
///   - `lastEventSeq`：int64 服务端当前 max event_seq → `Seq`（作离线同步 v2 cursor 种子）。
///   - `needSync`：bool，**缺省 true**（多拉不漏；false=已追平可省 sync）。
///   - `raw`：`serde_json::to_vec(data)`（供 emit im:channel:increment 透传 mentionList/urgentPostList）。
///
/// 任一必填非法（缺 id / id 非 26 字符 / lastEventSeq 非数值）→ None（不 panic / unwrap）。
pub fn parse_increment_channel(data: &serde_json::Value) -> Option<IncrementChannel> {
    let channel_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str)?;
    let last_event_seq = Seq(data.get("lastEventSeq").and_then(|v| v.as_u64())?);
    // needSync 缺省 true：宁可多拉一次 sync，也不漏增量（§1.3 防前端永久 loading）。
    let need_sync = data
        .get("needSync")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let raw = bytes::Bytes::from(serde_json::to_vec(data).ok()?);

    Some(IncrementChannel {
        channel_id,
        last_event_seq,
        need_sync,
        raw,
    })
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
                    .as_str()
                    .and_then(ChannelId::from_str)
                    .ok_or_else(|| {
                        ImError::Parse("event missing/invalid channel_id".to_string())
                    })?;
                let seq = ev_json["seq"]
                    .as_u64()
                    .ok_or_else(|| ImError::Parse("event missing seq".to_string()))?;
                let event_type = ev_json["event_type"].as_u64().unwrap_or(1) as u8;
                let kind = parse_event_kind(event_type);
                let raw = bytes::Bytes::from(serde_json::to_vec(ev_json).unwrap_or_default());
                events.push(EventEnvelope::new(channel_id, Seq(seq), kind, raw));
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
                .as_str()
                .and_then(ChannelId::from_str)
                .ok_or_else(|| ImError::Parse("snapshot missing/invalid channel_id".to_string()))?;
            let reset_to = v["reset_to"]
                .as_u64()
                .ok_or_else(|| ImError::Parse("snapshot missing reset_to".to_string()))?;
            let raw_messages = v["messages"]
                .as_array()
                .ok_or_else(|| ImError::Parse("snapshot missing messages".to_string()))?;

            let mut messages = Vec::with_capacity(raw_messages.len());
            for msg_json in raw_messages {
                let ch_id = msg_json["channel_id"]
                    .as_str()
                    .and_then(ChannelId::from_str)
                    .unwrap_or(channel_id);
                let seq = msg_json["seq"]
                    .as_u64()
                    .ok_or_else(|| ImError::Parse("snapshot message missing seq".to_string()))?;
                let event_type = msg_json["event_type"].as_u64().unwrap_or(1) as u8;
                let kind = parse_event_kind(event_type);
                let raw = bytes::Bytes::from(serde_json::to_vec(msg_json).unwrap_or_default());
                messages.push(EventEnvelope::new(ch_id, Seq(seq), kind, raw));
            }

            Ok(SyncResponse::Snapshot(ChannelSnapshot {
                channel_id,
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
