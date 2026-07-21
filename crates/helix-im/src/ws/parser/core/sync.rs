use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::sync_session::{ChannelSnapshot, EventEnvelope, EventKind, PostFields, SyncResponse};

use super::fields::extract_post_fields;
use super::kind::parse_event_kind;

/// 单个 SyncEntry 的 events 上限（真 Go `SyncEventsPerEntry`，设计文档 §1.4）。
///
/// 真 wire 的 `events` kind **无显式续拉标志**——满批（命中上限）即视为「还有更多」，
/// 进续拉链拉到不足上限 / `no_change` 为止（B3 验收：离线>500→续拉至 cursor==服务端 maxSeq）。
pub const SYNC_EVENTS_PER_ENTRY: usize = 500;

/// 解析 sync/notify HTTP 响应为指定 channel 的 `SyncResponse`（真 Go wire / B3）。
///
/// 真 Go 形态（`docs/过会方案/helix-sync-v2-real-go-wire-设计.md §1.4`，对齐现网
/// cses-client `domain/sync.rs` 行为真源）：
/// ```json
/// { "status": "SUCCESS",
///   "data": { "entries": [ { "channelId": "<id26>", "kind": "events",
///                            "events": [ChannelEvent], "nextSeq": 11,
///                            "resetTo": null, "messages": {} } ],
///             "serverTime": 1700000000 } }
/// ```
/// helix 逐 channel 发 sync（cursors 单元素），故响应 entries 命中本 channel 至多一条。
/// `kind`（snake_case 字面量，不可改）：`no_change` / `events` / `too_long` / `snapshot`（保留）。
/// `ChannelEvent` 游标字段 `eventSeq`(int64)，类型字段 `eventType`(1/2/3/6/7)。
/// type7 只接受密封 closed wire：`id,channelId,eventSeq,eventType:7,payload:{state:"closed"}`，
/// 不允许携带 post/actor/content 等额外字段。
///
/// 边界零信任（helix-im 不变量 4）：非法 JSON / status≠SUCCESS / 未知 kind → Err；
/// entries 缺失或无本 channel 条目 → NoChange（服务端对本 channel 无新事件，安全空转，
/// 不再误报 "missing 'type'"——旧抽象 wire 的根因）。
pub fn parse_sync_response(bytes: &[u8], channel_id: ChannelId) -> Result<SyncResponse, ImError> {
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| ImError::Parse(format!("sync response invalid JSON: {}", e)))?;

    // status 存在则必须 SUCCESS（缺省容忍裸 data 回报，便于 driver 直灌测试帧）。
    if let Some(status) = v.get("status").and_then(|s| s.as_str()) {
        if status != "SUCCESS" {
            return Err(ImError::Parse(format!(
                "sync response status not SUCCESS: {}",
                status
            )));
        }
    }

    // entries 落点：优先 data.entries（真 wire），兼容顶层 entries。
    let entries = v
        .get("data")
        .and_then(|d| d.get("entries"))
        .or_else(|| v.get("entries"))
        .and_then(|e| e.as_array());
    let entries = match entries {
        Some(e) => e,
        // 无 entries：服务端无本 channel 数据 → 空转，不报错。
        None => return Ok(SyncResponse::NoChange),
    };

    // 取命中本 channel 的 entry（helix 逐 channel 发，至多一条）；无 → NoChange。
    // S10：SyncEntry 真源 `domain/sync.rs:120 #[serde(rename_all="camelCase")]` → wire `channelId`；
    // 兼容 snake `channel_id`（server app/sync.go 历史 json tag，零信任双形态，helix-im 不变量 4）。
    let entry = entries.iter().find(|e| {
        let cid = e
            .get("channelId")
            .or_else(|| e.get("channel_id"))
            .and_then(|c| c.as_str());
        cid == Some(channel_id.as_str())
    });
    let entry = match entry {
        Some(e) => e,
        None => return Ok(SyncResponse::NoChange),
    };

    let kind = entry["kind"]
        .as_str()
        .ok_or_else(|| ImError::Parse("sync entry missing 'kind'".to_string()))?;

    match kind {
        "no_change" => Ok(SyncResponse::NoChange),

        "events" => {
            let mut events = parse_channel_events(entry, channel_id)?;
            // C2：消息内容快照（key=msgId）。phantom = event.msg_id ∉ messages（真源 post.rs:1069）。
            let messages = parse_messages_map(entry);
            // 真 wire 无续拉标志：满批（命中上限）即认为还有更多 → 续拉；不足 → 已追平。
            let needs_continuation = events.len() >= SYNC_EVENTS_PER_ENTRY;
            // cursor 锚服务端权威 nextSeq（Go `next = events[末].EventSeq`）；缺省/异常
            // 回退到 max(event.seq)（与 nextSeq 当前恒等），再退 Seq(0)。
            // S10：真源 wire `nextSeq`（camelCase）；兼容 snake `next_seq`（零信任双形态）。
            let max_event_seq = events.iter().map(|e| e.seq.0).max().unwrap_or(0);
            let next_seq = Seq(entry["nextSeq"]
                .as_u64()
                .or_else(|| entry["next_seq"].as_u64())
                .unwrap_or(max_event_seq));
            // 分桶排序（type1 → type2 → type3 → type6，铁律）
            sort_events_by_bucket(&mut events);
            Ok(SyncResponse::Events {
                events,
                messages,
                next_seq,
                needs_continuation,
            })
        }

        "too_long" => {
            // S10：真源 wire `resetTo`（camelCase）；兼容 snake `reset_to`（零信任双形态）。
            let reset_to = entry["resetTo"]
                .as_u64()
                .or_else(|| entry["reset_to"].as_u64())
                .ok_or_else(|| ImError::Parse("too_long missing resetTo".to_string()))?;
            Ok(SyncResponse::TooLong {
                reset_to: Seq(reset_to),
            })
        }

        "snapshot" => {
            // 保留未启用：防御性按 entry.events + resetTo 构造快照（resetTo 缺省 0）。
            let reset_to = entry["resetTo"]
                .as_u64()
                .or_else(|| entry["reset_to"].as_u64())
                .unwrap_or(0);
            let messages = parse_channel_events(entry, channel_id)?;
            if messages
                .iter()
                .any(|event| event.kind == EventKind::ChannelTerminalClosed)
            {
                return Err(ImError::Parse(
                    "terminal event is only valid in sync events entries".to_string(),
                ));
            }
            Ok(SyncResponse::Snapshot(ChannelSnapshot {
                channel_id,
                reset_to: Seq(reset_to),
                messages,
            }))
        }

        other => Err(ImError::Parse(format!(
            "unknown sync entry kind: {}",
            other
        ))),
    }
}

/// 解析 `SyncEntry.events`（`[ChannelEvent]`）为 `Vec<EventEnvelope>`（边界零信任）。
///
/// ChannelEvent 游标字段 `eventSeq`（int64，权威），兼容 `event_seq` / `seq` 旧字面量；
/// 类型字段 `eventType`（1/2/3/6），兼容 `event_type`（缺省 1）。type7 不走兼容分支，
/// 必须满足最小 closed wire；
/// `channelId` 缺省回退到 entry 的 channel_id。`events` 缺失 → 空列表（no_change 形态）。
fn parse_channel_events(
    entry: &serde_json::Value,
    fallback_channel: ChannelId,
) -> Result<Vec<EventEnvelope>, ImError> {
    let raw_events = match entry.get("events").and_then(|e| e.as_array()) {
        Some(e) => e,
        None => return Ok(Vec::new()),
    };
    let mut events = Vec::with_capacity(raw_events.len());
    for ev in raw_events {
        if ev.get("eventType").and_then(serde_json::Value::as_u64) == Some(7) {
            events.push(parse_terminal_event(ev, fallback_channel)?);
            continue;
        }
        let channel_id = ev
            .get("channelId")
            .and_then(|c| c.as_str())
            .and_then(ChannelId::from_str)
            .unwrap_or(fallback_channel);
        // eventSeq 是真 Go int64 游标；兼容 event_seq / seq 既有测试帧。
        let seq = ev["eventSeq"]
            .as_u64()
            .or_else(|| ev["event_seq"].as_u64())
            .or_else(|| ev["seq"].as_u64())
            .ok_or_else(|| ImError::Parse("channel event missing eventSeq".to_string()))?;
        let event_type = ev["eventType"]
            .as_u64()
            .or_else(|| ev["event_type"].as_u64())
            .unwrap_or(1) as u8;
        let kind = parse_event_kind(event_type)?;
        // HX-C005：`ev`（ChannelEvent Value）已在手，直接提取落库字段——
        // 消除原 `serde_json::to_vec(ev)` 回序列化 + 落库时 `from_slice(raw)` 再解析的三趟开销。
        let fields = extract_post_fields(ev);
        // C2：wire 权威 `msgId`（phantom 判定真源）。兼容 msgId（Go camelCase）/ msg_id（snake 测试帧）。
        // 零信任：缺省 / 空串 → None（with_msg_id 内部再过滤空串）。
        let msg_id = ev
            .get("msgId")
            .or_else(|| ev.get("msg_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let event_id = ev
            .get("id")
            .or_else(|| ev.get("eventId"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let actor_id = ev
            .get("actorId")
            .or_else(|| ev.get("actor_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let occurred_at = ev
            .get("occurredAt")
            .or_else(|| ev.get("createAt"))
            .or_else(|| ev.get("created_at"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let event_payload = ev
            .get("payload")
            .filter(|payload| !payload.is_null())
            .and_then(|payload| serde_json::to_string(payload).ok())
            .unwrap_or_default();
        events.push(
            EventEnvelope::new(channel_id, Seq(seq), kind, fields)
                .with_msg_id(msg_id)
                .with_event_identity(event_id, actor_id, occurred_at, event_payload),
        );
    }
    Ok(events)
}

/// 解析 type7 closed terminal 的最小 wire。
///
/// terminal 没有 post、actor 或时间语义。拒绝多余字段，避免终态路径把不可信的附带数据
/// 保存或投影为消息。只有这里能构造 `ChannelTerminalClosed`；legacy WS 数值帧会拒绝 type7。
fn parse_terminal_event(
    event: &serde_json::Value,
    expected_channel: ChannelId,
) -> Result<EventEnvelope, ImError> {
    let object = event
        .as_object()
        .ok_or_else(|| ImError::Parse("terminal event must be an object".to_string()))?;
    const REQUIRED: [&str; 5] = ["id", "channelId", "eventSeq", "eventType", "payload"];
    if object.len() != REQUIRED.len() || REQUIRED.iter().any(|key| !object.contains_key(*key)) {
        return Err(ImError::Parse(
            "terminal event must contain only id,channelId,eventSeq,eventType,payload".to_string(),
        ));
    }
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ImError::Parse("terminal event missing/invalid id".to_string()))?;
    let channel_id = object
        .get("channelId")
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse("terminal event missing/invalid channelId".to_string()))?;
    if channel_id != expected_channel {
        return Err(ImError::Parse(
            "terminal event channelId does not match sync entry".to_string(),
        ));
    }
    let seq = object
        .get("eventSeq")
        .and_then(serde_json::Value::as_u64)
        .filter(|seq| *seq > 0 && *seq <= i64::MAX as u64)
        .ok_or_else(|| ImError::Parse("terminal event missing/invalid eventSeq".to_string()))?;
    if object.get("eventType").and_then(serde_json::Value::as_u64) != Some(7) {
        return Err(ImError::Parse(
            "terminal event eventType must be 7".to_string(),
        ));
    }
    let payload = object
        .get("payload")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ImError::Parse("terminal event payload must be an object".to_string()))?;
    if payload.len() != 1
        || payload.get("state").and_then(serde_json::Value::as_str) != Some("closed")
    {
        return Err(ImError::Parse(
            "terminal event payload must be exactly {state:closed}".to_string(),
        ));
    }

    Ok(EventEnvelope::new(
        channel_id,
        Seq(seq),
        EventKind::ChannelTerminalClosed,
        PostFields::default(),
    )
    .with_event_identity(Some(id.to_string()), None, 0, String::new()))
}

/// 解析 `SyncEntry.messages`（key=msgId → Post JSON 对象）为 owned `PostFields` map（C2 真源）。
///
/// 真源 post.rs:1047 `messages: &HashMap<String, serde_json::Value>`——服务端按可见性过滤后
/// 下发的消息内容快照。helix 在 parser 阶段一次解析为 owned `PostFields`（复用 `extract_post_fields`，
/// HX-C005 热路径零再解析）。`event.msg_id ∈ map` → 落内容行；`∉` → phantom。
/// `messages` 缺失 / 非对象 → 空 map（= 全 phantom，仅推 cursor）。边界零信任不 panic。
fn parse_messages_map(entry: &serde_json::Value) -> std::collections::HashMap<String, PostFields> {
    let mut map = std::collections::HashMap::new();
    let Some(obj) = entry.get("messages").and_then(|m| m.as_object()) else {
        return map;
    };
    map.reserve(obj.len());
    for (msg_id, post) in obj {
        map.insert(msg_id.clone(), extract_post_fields(post));
    }
    map
}

/// 分桶顺序（铁律，来自源码事实 2026-06-08）
///
/// 处理顺序必须严格：type1 → type2 → type3 → type6 → type7
/// 不得乱序，否则业务状态不一致。
pub fn sort_events_by_bucket(events: &mut Vec<EventEnvelope>) {
    events.sort_by_key(|ev| match ev.kind {
        EventKind::PostUpsert => 0u8,
        EventKind::PostEdit => 1,
        EventKind::PostRevoke => 2,
        EventKind::PostRead => 3,
        EventKind::ChannelTerminalClosed => 4,
        EventKind::Other(_) => 5,
    });
}
