//! IM WS 帧解析器（InboundBytes → EventEnvelope）
//!
//! 解析 14 类 WS type，属于 ACL-1 的边界工作。
//! helix-core 的 Tick::Inbound 只携带不透明字节，由此模块解析为强类型。

use crate::error::ImError;
use crate::state::{ChannelId, Seq, ServerId};
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
/// 调用方（`parse_inbound` / `parse_channel_events`）此时 `Value` 已解析完成（用于取 seq/type），
/// 复用同一棵树提取字段——避免落库路径（`event_to_upsert_op`）再 `from_slice(raw)` 重解析。
fn extract_post_fields(root: &serde_json::Value) -> PostFields {
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
    let msg_type = pick_str("type")
        .or_else(|| pick_str("post_type"))
        .unwrap_or_default();
    let message = pick_str("message").unwrap_or_default();
    let create_at = post
        .get("create_at")
        .or_else(|| post.get("createAt"))
        .or_else(|| root.get("create_at"))
        .or_else(|| root.get("createAt"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // props 保留服务端原始对象（业务扩展 / event_seq / read map 下游对账）。
    let props = post
        .get("props")
        .or_else(|| root.get("props"))
        .map(|v| v.to_string())
        .unwrap_or_default();
    // read_bits（C2 / type=6 真源 post.rs:1088 `readBits`）：服务端权威已读位字符串。
    // 兼容 readBits（Go camelCase wire）/ read_bits（snake 测试帧）；缺省空串。
    let read_bits = pick_str("readBits")
        .or_else(|| pick_str("read_bits"))
        .unwrap_or_default();

    PostFields {
        temporary_id,
        id,
        channel_id,
        user_id,
        msg_type,
        message,
        props,
        create_at,
        read_bits,
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
/// 的 `EventEnvelope`。`classify_inbound` 的「无 action」臂复用此函数，消除第二趟 `from_slice`（痛点 1）。
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

/// 入站 WS 帧一次解析分到底的结果（重构 A：取代 `FrameClass` + legacy `Event`→`parse_inbound` 二趟）。
///
/// 真 Go 信封实测形态：`{"v":1,"action":"<type>","data":{...},"broadcast":{...},"seq":<信封序号>}`
/// （`action` 在顶层、`seq` 是信封序号非 event_seq；样本见 tests/fixtures/real_go_frames/）。
///
/// 分发维度（顶层 `action` + 无 action 的 legacy 数值帧）一次性归类；每个携带数据的变体都装
/// **已解析好的强类型载荷**，`module.handle` 不再二次解析。边界零信任（helix-im 不变量 4）：
/// 任一变体解析失败 → `Ignored`（不 panic / 不 unwrap）。
#[derive(Debug, PartialEq, Eq)]
pub enum InboundEvent {
    /// hello 握手帧，携带 connectionId（A4 握手）。
    Hello(String),
    /// `increment_channel` 帧（B-rest）：per-channel 增量推送，携带解析好的 IncrementChannel。
    Increment(IncrementChannel),
    /// `increment_channel_end` 帧（B-rest）：本批增量收尾信号。
    /// `Some(channel_id)`=某 parent 子 topic 批次结束；`None`=hello/重连整批 global end。
    IncrementEnd(Option<ChannelId>),
    /// echo 回声帧（C1 §4.5 发送对账）：本端发出的 `posted` 帧带 `temporary_id` 回流。
    ///
    /// **真帧待核**（TODO sync-v2 C1）：当前按最可能形态识别——真 Go `action=="posted"`
    /// 帧的 `data` 含本端 send 时注入的 `temporary_id`（非空）即视为 echo（行为真源
    /// cses-client post.rs:58「`post.temporary_id` 非空 → sendMessage 回流对账」）。
    /// 抓到首个真 echo 帧（`HELIX_DUMP_FRAMES=1`）后核对 action 名 / temporary_id / id
    /// 字段路径，补 golden fixture（tests/fixtures/real_go_frames/echo_*.jsonl）。
    ///
    /// - `channel_id`：post 所属 channel（cursor 推进锚）。
    /// - `seq`：event_seq（三级 fallback，与 increment LastEventSeq 同源 §1.5），推进 cursor。
    /// - `temporary_id`：本端 send 注入的 PK，按值匹配 PendingSend。
    /// - `server_id`：服务端权威 `post.id`（26 字符 Id26），覆写 message.id。
    Echo {
        channel_id: ChannelId,
        seq: Seq,
        temporary_id: String,
        server_id: ServerId,
    },
    /// 真 Go 在线广播新消息/echo 统一帧（C1 sync-v2，action==`post`）。
    ///
    /// **现网真相**（行为真源 cses-client router.rs:40 `"post" => post::handle` +
    /// post.rs:42-114 + message_service.rs:64-69）：真 Go WS 在线新消息广播顶层
    /// `action=="post"`（**不是** `posted`——Go 常量 `WebsocketEventPost="post"` 与
    /// `WebsocketEventPosted="posted"` 是两个事件，cses 新消息用 `post`）。现网对 echo
    /// 与新消息**不在落库层分叉**：统一 `handle_new_post`→`batch_upsert ON CONFLICT(temporary_id)`，
    /// echo（本端 temporary_id 命中乐观行）靠 PK 自然对账，server `id` 覆写本地行；
    /// 区分仅发生在 Track 日志（self vs other），不影响数据。
    ///
    /// helix 对齐：`post` 帧解析成 `EventEnvelope` 喂 channel gate（`ch.ingest`），
    /// 复用既有 `event_to_upsert_op`（ON CONFLICT temporary_id）路径——echo 与新消息
    /// 统一走 gate，靠 temporary_id PK upsert 自然对账，不另开 reconcile 特例。
    /// seq=event_seq（三级 fallback，data.event_seq 优先），kind=PostUpsert（post 恒为 upsert）。
    Post(EventEnvelope),
    /// 无 `action` 的 legacy `{event,data}` 数值帧（抽象 wire / 既有测试）。
    ///
    /// 重构 A：**改前**此处是空信号变体 `Event`（需 module 再调 `parse_inbound` 二解析）；
    /// **改后**直接携带 `parse_inbound_value` 解析好的 `EventEnvelope`（`kind` 由 type 数字
    /// 经 `parse_event_kind` 派生），module 直接 `ch.ingest`，不再二趟解析。
    LegacyEvent(EventEnvelope),
    /// 运维/状态帧——优雅忽略（不报错、不刷日志）。
    /// 现网 status_change / multiple_channels_viewed / typing / presence 等本就可丢。
    /// 边界零信任：非法 JSON / 缺字段 / legacy 帧解析失败一律落此，不 panic。
    Ignored,
}

/// 一次解析分到底，按真 Go 信封顶层 `action`（含无 action 的 legacy 数值帧）分类入站帧。
/// 零信任：非法 JSON / 缺字段 → Ignored（不 panic）。
///
/// - `action=="hello"` → `Hello(connectionId)`
/// - `increment_channel` / `increment_channel_end` / `post` / `posted` → 各自强类型变体
/// - 其它 `action`（status_change / typing 等）→ `Ignored`
/// - 无 `action`（legacy `{event,data}` 帧）→ 内联 `parse_inbound_value`：成功 `LegacyEvent(ev)`，
///   失败 `Ignored`（保留原 `module.rs` legacy 路径「parse error 静默 drop」语义，行为等价）
pub fn classify_inbound(bytes: &[u8]) -> InboundEvent {
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => return InboundEvent::Ignored,
    };
    match v.get("action").and_then(|a| a.as_str()) {
        Some("hello") => v
            .get("data")
            .and_then(|d| d.get("connectionId"))
            .and_then(|c| c.as_str())
            .map(|id| InboundEvent::Hello(id.to_string()))
            .unwrap_or(InboundEvent::Ignored),
        Some("increment_channel") => match v.get("data").and_then(parse_increment_channel) {
            Some(ic) => InboundEvent::Increment(ic),
            None => InboundEvent::Ignored, // 解析失败/未实现 → 不误报
        },
        Some("increment_channel_end") => {
            // data.channelId 非空=子 topic 批次结束；空/缺=global end。零信任。
            let ch = v
                .get("data")
                .and_then(|d| d.get("channelId"))
                .and_then(|c| c.as_str())
                .and_then(ChannelId::from_str);
            InboundEvent::IncrementEnd(ch)
        }
        // C1：真 Go 在线广播新消息/echo 统一帧（action==`post`，行为真源 router.rs:40）。
        // echo 与新消息**不在落库层分叉**——统一解析成 EventEnvelope 喂 channel gate
        // （ch.ingest → event_to_upsert_op ON CONFLICT temporary_id），echo 靠 PK 自然对账。
        // 解析失败（缺 channelId / event_seq）→ Ignored（边界零信任，不误报）。
        Some("post") => parse_post_frame(&v)
            .map(InboundEvent::Post)
            .unwrap_or(InboundEvent::Ignored),
        // `posted` 帧若带本端 `temporary_id` → echo 回声（发送对账）。**保留向后兼容**：
        // 真 wire 在线广播用 `post`（上一臂），`posted` 仅作既有 echo 单测/历史兼容路径。
        // 非 echo 的 `posted`（他人推送，temporary_id 空 / 字段缺失 / id 非 26 字符）→ Ignored。
        Some("posted") => classify_echo(&v).unwrap_or(InboundEvent::Ignored),
        Some(_) => InboundEvent::Ignored,
        // 无 action：legacy 数值帧。内联 parse_inbound_value（复用同一棵 `v`，消除第二趟 from_slice）。
        // 解析失败 → Ignored + debug log（迁移原 module.rs legacy 路径的「Err(Parse) 静默 drop」语义）。
        None => match parse_inbound_value(&v) {
            Ok(ev) => InboundEvent::LegacyEvent(ev),
            Err(ImError::Parse(msg)) => {
                tracing::debug!("im: inbound parse error: {}", msg);
                InboundEvent::Ignored
            }
            Err(_) => InboundEvent::Ignored,
        },
    }
}

/// 从 `posted` 帧识别 echo 回声（C1 §4.5）。边界零信任：任一必填缺失/非法 → None（不 panic）。
///
/// **真帧待核**（TODO sync-v2 C1，见 `InboundEvent::Echo` 文档）：当前按最可能形态解析——
/// `data`（或嵌套 `data.post`）含 `temporary_id`（非空）+ `id`（26 字符 server_id）+
/// `channel_id` + `event_seq`（三级 fallback）。抓到真 echo 帧后核对字段路径并补 fixture。
///
/// 识别条件（全部满足才算 echo，否则非 echo `posted` → 调用方按 Ignored 处理）：
///   1. `temporary_id` 非空（本端 send 注入，他人推送无此字段 → 排除）；
///   2. `id` 是合法 26 字符 Id26（server 权威 post.id）；
///   3. `channel_id` 是合法 26 字符 Id26；
///   4. `event_seq` 存在（三级 fallback：data.event_seq → props.channel_event_seq → 顶层）。
fn classify_echo(v: &serde_json::Value) -> Option<InboundEvent> {
    let data = v.get("data")?;
    // post 字段可能嵌在 data 顶层或 data.post 子对象下（与 extract_post_fields 同源探查）。
    let post = data.get("post").unwrap_or(data);

    // temporary_id：兼容 snake/camel；空串视为「无 temporary_id」→ 非 echo。
    let temporary_id = post
        .get("temporary_id")
        .or_else(|| post.get("temporaryId"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();

    // server_id = 权威 post.id（26 字符 Id26）；非 26 字符 → None（不误判截断）。
    let server_id = post
        .get("id")
        .and_then(|i| i.as_str())
        .and_then(ServerId::from_str)?;

    // channel_id（cursor 推进锚）：兼容 snake/camel；缺/非 26 字符 → None。
    let channel_id = post
        .get("channel_id")
        .or_else(|| post.get("channelId"))
        .and_then(|c| c.as_str())
        .and_then(ChannelId::from_str)?;

    // event_seq 三级 fallback（§1.5，与 parse_inbound 同源）：
    //   ① data.event_seq（权威 int64）→ ② per-post props.channel_event_seq → ③ 顶层。
    let seq = Seq(data
        .get("event_seq")
        .and_then(|s| s.as_u64())
        .or_else(|| {
            post.get("props")
                .and_then(|p| p.get("channel_event_seq"))
                .and_then(|s| s.as_u64())
        })
        .or_else(|| v.get("channel_event_seq").and_then(|s| s.as_u64()))?);

    Some(InboundEvent::Echo {
        channel_id,
        seq,
        temporary_id,
        server_id,
    })
}

/// 解析真 Go `action=="post"` 在线广播帧 → `EventEnvelope`（C1 sync-v2，喂 channel gate）。
///
/// 行为真源（现网 source of truth）：
///   - `cses-client/src-tauri/src/features/im/router.rs:40` `"post" => post::handle`；
///   - `post.rs:42-114` 统一 `handle_new_post`，不在落库层分叉 echo/新消息；
///   - `message_service.rs:64-69` `batch_upsert ON CONFLICT(temporary_id)`。
///
/// 硬 gate（缺则 None → 调用方 Ignored，边界零信任不 panic，helix-im 不变量 4）：
///   1. `channel_id`：`data.channelId` ?? `data.channel_id`（26 字符 Id26）——cursor 推进锚；
///   2. `seq`：event_seq 三级 fallback（**data.event_seq 优先**，与 `parse_inbound`/§1.5 同源）：
///      ① `data.event_seq`（权威 int64，post_core.go:424 注入顶层 data）
///      → ② per-post `data.props.channel_event_seq`（单帧冗余兜底，与顶层同值）
///      → ③ 顶层 `channel_event_seq`（legacy 键名）。零信任只取 `as_u64`，非数值/缺 → None。
///
/// 宽松字段（**不**像 classify_echo 硬校验 26 字符 id——现网 handle_new_post 对 id 不卡
/// `message_service.rs:55 unwrap_or("unknown")`）：
///   - `kind`：恒 `PostUpsert`（type1；post 帧恒为新消息 upsert）；
///   - `fields`：`extract_post_fields(data 或 root)`（复用既有提取，读 id/temporary_id/
///     channelId/message/props/readBits 等，全程安全回退）；
///   - `msg_id`：宽松取 `data.id` / `data.post.id`（缺省 None，with_msg_id 内部回退 fields 派生）。
fn parse_post_frame(v: &serde_json::Value) -> Option<EventEnvelope> {
    let data = v.get("data")?;
    // post 字段可能嵌在 data 顶层或 data.post 子对象下（与 extract_post_fields 同源探查）。
    let post = data.get("post").unwrap_or(data);

    // channel_id 硬 gate（cursor 推进锚）：兼容 camel/snake；缺/非 26 字符 → None。
    let channel_id = data
        .get("channelId")
        .or_else(|| data.get("channel_id"))
        .or_else(|| post.get("channelId"))
        .or_else(|| post.get("channel_id"))
        .and_then(|c| c.as_str())
        .and_then(ChannelId::from_str)?;

    // event_seq 三级 fallback（§1.5，data.event_seq 优先，与 parse_inbound 同源）：
    //   ① data.event_seq（权威 int64）→ ② per-post props.channel_event_seq → ③ 顶层 channel_event_seq。
    // 零信任只取 as_u64；非数值/缺 → None（不 panic）。
    let seq = Seq(data
        .get("event_seq")
        .and_then(|s| s.as_u64())
        .or_else(|| {
            post.get("props")
                .and_then(|p| p.get("channel_event_seq"))
                .and_then(|s| s.as_u64())
        })
        .or_else(|| v.get("channel_event_seq").and_then(|s| s.as_u64()))?);

    // post 帧恒为新消息 upsert（type1）；字段复用既有提取（宽松回退，含 temporary_id/id/message/props/readBits）。
    let fields = extract_post_fields(data);

    // msg_id 宽松取（缺省 None → with_msg_id 内部回退 fields 派生的 id/temporary_id）。
    // **不**硬校验 26 字符（现网 handle_new_post 对 id 不 gate）。
    let msg_id = post.get("id").and_then(|i| i.as_str()).map(str::to_string);

    Some(EventEnvelope::new(channel_id, seq, EventKind::PostUpsert, fields).with_msg_id(msg_id))
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
    let entry = entries
        .iter()
        .find(|e| e.get("channelId").and_then(|c| c.as_str()) == Some(channel_id.as_str()));
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
            let max_event_seq = events.iter().map(|e| e.seq.0).max().unwrap_or(0);
            let next_seq = Seq(entry["nextSeq"].as_u64().unwrap_or(max_event_seq));
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
            let reset_to = entry["resetTo"]
                .as_u64()
                .ok_or_else(|| ImError::Parse("too_long missing resetTo".to_string()))?;
            Ok(SyncResponse::TooLong {
                reset_to: Seq(reset_to),
            })
        }

        "snapshot" => {
            // 保留未启用：防御性按 entry.events + resetTo 构造快照（resetTo 缺省 0）。
            let reset_to = entry["resetTo"].as_u64().unwrap_or(0);
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
