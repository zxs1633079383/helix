//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::sync_session::{
    ChannelSnapshot, EventEnvelope, EventKind, IncrementChannel, PostFields, SyncResponse,
};

/// 从已在手的 `Value`（root + post 视图）一次性提取落库 owned 字段（HX-C005）。
///
/// 字段映射真源 = cses-client `From<types::Post> for Message`：
///   - post 字段可能在顶层，也可能嵌套在 `post` / `data` 子对象下——按优先级探查。
///   - 兼容 snake_case / camelCase（Go wire 多 camelCase；既有测试帧用 snake_case）。
///   - 全程安全回退不 panic（边界零信任，helix-im 不变量 4）。
///
/// 调用方（`parse_inbound` / `parse_channel_events` / ws `post` handler）此时 `Value` 已解析完成
/// （用于取 seq/type），复用同一棵树提取字段——避免落库路径（`event_to_upsert_op`）再
/// `from_slice(raw)` 重解析。
pub(crate) fn extract_post_fields(root: &serde_json::Value) -> PostFields {
    // post 字段可能在顶层，也可能嵌套在 "post" / "data" 子对象下——按优先级探查。
    let post = root
        .get("post")
        .or_else(|| root.get("data"))
        .unwrap_or(root);

    // 取字符串字段：先 post 视图，回退 root 顶层（兼容扁平 wire）。
    let pick_str = |key: &str| -> Option<String> {
        post.get(key)
            .and_then(|v| v.as_str())
            .or_else(|| root.get(key).and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };

    let temporary_id = pick_str("temporary_id")
        .or_else(|| pick_str("temporaryId"))
        .unwrap_or_default();
    let id = pick_str("id").unwrap_or_default();
    // channel_id 此处可能为空（缺省）；event_to_upsert_op 回退到 envelope 权威 channel_id。
    let channel_id = pick_str("channel_id")
        .or_else(|| pick_str("channelId"))
        .unwrap_or_default();
    let user_id = pick_str("user_id")
        .or_else(|| pick_str("userId"))
        .unwrap_or_default();
    // type 空→"TEXT"（render-ready 下沉·C013 业务默认下沉 helix·与 send_build.rs:165 一致·#53）
    let msg_type = pick_str("type")
        .or_else(|| pick_str("post_type"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "TEXT".to_string());
    let message = pick_str("message").unwrap_or_default();
    let simple_message = pick_str("simple_message")
        .or_else(|| pick_str("simpleMessage"))
        .unwrap_or_else(|| message.clone());
    let create_at = post
        .get("create_at")
        .or_else(|| post.get("createAt"))
        .or_else(|| root.get("create_at"))
        .or_else(|| root.get("createAt"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // props 保留服务端原始对象（业务扩展 / event_seq / read map 下游对账）+ UC-1.8 顶层 quickReply
    // emoji 反应合并进 props.quickReply（逻辑外提收口行数基线，详见 ws::parser::post_props）。
    let props = super::post_props::merge_props_with_quickreply(post, root);
    let user_snapshot = post
        .get("userSnapshot")
        .or_else(|| post.get("user_snapshot"))
        .or_else(|| root.get("userSnapshot"))
        .or_else(|| root.get("user_snapshot"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    // read_bits（C2 / type=6 真源 post.rs:1088 `readBits`）：服务端权威已读位字符串。
    // 兼容 readBits（Go camelCase wire）/ read_bits（snake 测试帧）；缺省空串。
    let read_bits = pick_str("readBits")
        .or_else(|| pick_str("read_bits"))
        .unwrap_or_default();
    // viewers（A3/CAP-9 可见性受众）：post 子树或 root 顶层的字符串数组；缺省空 Vec（无堆分配）。
    // 元素非字符串项跳过（边界零信任）。
    let viewers = post
        .get("viewers")
        .or_else(|| root.get("viewers"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mentions = post
        .get("mentions")
        .or_else(|| root.get("mentions"))
        .map(collect_string_values)
        .unwrap_or_default();
    let expedite_map = post
        .get("expediteMap")
        .or_else(|| post.get("expedite_map"))
        .or_else(|| root.get("expediteMap"))
        .or_else(|| root.get("expedite_map"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let topic = post
        .get("topic")
        .or_else(|| root.get("topic"))
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let reply_id = pick_str("replyId")
        .or_else(|| pick_str("reply_id"))
        .unwrap_or_default();
    let reply_root_id = pick_str("replyRootId")
        .or_else(|| pick_str("reply_root_id"))
        .unwrap_or_default();
    let reply_first_level_id = pick_str("replyFirstLevelId")
        .or_else(|| pick_str("reply_first_level_id"))
        .unwrap_or_default();

    PostFields {
        temporary_id,
        id,
        channel_id,
        user_id,
        msg_type,
        message,
        simple_message,
        props,
        user_snapshot,
        create_at,
        read_bits,
        viewers,
        mentions,
        expedite_map,
        topic,
        reply_id,
        reply_root_id,
        reply_first_level_id,
    }
}

fn collect_string_values(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

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
    parse_inbound_value(&v)
}

/// `parse_inbound` 的内核：从**已解析好的 root `Value`**（非 `data` 子树）提取 legacy 数值帧
/// 的 `EventEnvelope`。`ws::handlers::legacy_event`（无 action 的 legacy 数值帧）复用此函数，
/// 复用 `WsFrame::parse` 已解析的 root Value，消除第二趟 `from_slice`（痛点 1）。
///
/// **边界等价铁律（重构 A 不可漂移）**：取数源逐字保持 `parse_inbound` 历史行为——
///   - `data` 硬要求存在（缺 → Err），channel_id / event_seq 等取数全部基于 `data`；
///   - `channel_id` 仅 `data["channel_id"]`（snake_case，**无 camelCase fallback**）；
///   - event_seq 四级 fallback：① `data.event_seq` ② `data.props.channel_event_seq`
///     ③ **顶层** `v["channel_event_seq"]`（第③级必须取 root，故入参 = root） ④ `data.seq`；
///   - `extract_post_fields(v)` 传 **root**（不是 data）。
/// 任何「顺手统一成 parse_post_frame 风格」都会让 increment_parse_test 的 fallback 断言语义漂移。
pub fn parse_inbound_value(v: &serde_json::Value) -> Result<EventEnvelope, ImError> {
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
    // HX-C005：字段提取上移此处（`v` 已解析在手），落库路径零再解析、零回序列化。
    // 边界等价铁律：传 ROOT（不是 data）——与历史 parse_inbound 一字不动。
    let fields = extract_post_fields(v);
    // WS 帧 wire `msg_id`（兼容 msgId）；缺省 None（with_msg_id 内部回退 fields 派生值）。
    let msg_id = data
        .get("msg_id")
        .or_else(|| data.get("msgId"))
        .and_then(|m| m.as_str())
        .map(str::to_string);

    Ok(EventEnvelope::new(channel_id, Seq(seq), kind, fields).with_msg_id(msg_id))
}

/// 解析 `increment_channel` 帧的 `data` → `IncrementChannel`（边界零信任，非法→None）。
///
/// 按设计文档 §1.3 解析真 Go `IncrementChannel.ToMap()` 的 data：
///   - `id`：26 字符 base32 channelId → `ChannelId::from_str`（**唯一必填**，cursor/路由锚；
///     零信任，非 26 字符→None）。
///   - `lastEventSeq`：int64 服务端当前 max event_seq → `Seq`（作离线同步 v2 cursor 种子）。
///     **真 Go 为 Optional**（types.rs:257 `Option<i64>`，legacy mattermost 9.7.3 的 GetMaxSeq
///     降级场景帧不带此字段，channel_state.rs:345 `unwrap_or(0)`）——缺省回退 `Seq(0)`，**绝不**
///     因缺失整帧丢弃（P0：实测 650 increment 帧因 `?` 早退被全静默丢、落库 0 行）。
///   - `needSync`：bool，**缺省 true**（多拉不漏；false=已追平可省 sync）。
///   - `raw`：`serde_json::to_vec(data)`（供 emit im:channel:increment 透传 mentionList/urgentPostList）。
///
/// 仅 `id` 非法（缺 / 非 26 字符）→ None（边界零信任，不 panic / unwrap）；其余字段缺省回退默认值。
pub fn parse_increment_channel(data: &serde_json::Value) -> Option<IncrementChannel> {
    let channel_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str)?;
    // lastEventSeq 真 Go Optional（GetMaxSeq 降级场景缺失）→ 缺省 Seq(0)，不早退丢帧。
    let last_event_seq = Seq(data
        .get("lastEventSeq")
        .and_then(|v| v.as_u64())
        .unwrap_or(0));
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
/// `ChannelEvent` 游标字段 `eventSeq`(int64)，类型字段 `eventType`(1/2/3/6)。
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
/// 类型字段 `eventType`（1/2/3/6），兼容 `event_type`（缺省 1）；
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
        let kind = parse_event_kind(event_type);
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
        events.push(EventEnvelope::new(channel_id, Seq(seq), kind, fields).with_msg_id(msg_id));
    }
    Ok(events)
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
