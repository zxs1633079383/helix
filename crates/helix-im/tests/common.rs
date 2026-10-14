//! 公共测试辅助（测试 harness）
//!
//! ## 使用方式
//!
//! 在各测试文件顶部：
//! ```
//! mod common;
//! use common::*;
//! ```

use helix_core::effect::{Correlation, Row, SqlValue, StorageOp, TimerId, UpsertSpec};
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Tick};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::{ChannelId, ServerId};
use helix_im::ImModule;

// ─── Harness ────────────────────────────────────────────────────────────────

/// 回放测试 harness
pub struct Harness {
    pub module: ImModule,
    pub sink: EffectSink,
    /// 所有 step 产出的 Effect，按 step 批次分组
    pub effect_log: Vec<Vec<EffectKind>>,
}

/// Effect 的简化分类（用于断言，不含全部字段）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectKind {
    Persist(u64),       // corr.raw()
    PersistAtomic(u64), // corr.raw()
    PersistFire,
    Http(u64),    // corr.raw()
    HttpFire,     // fire-and-forget HTTP（无 corr）
    Request(u64), // corr.raw()（REP 方向① 发起端）
    Emit,
    ScheduleTimer(u64), // timer_id.raw()
    CancelTimer(u64),   // timer_id.raw()
    UploadFile(u64),    // corr.raw()
    Send,
}

impl EffectKind {
    pub fn from_effect(e: &Effect) -> Self {
        match e {
            Effect::Persist { corr, .. } => EffectKind::Persist(corr.raw()),
            Effect::PersistAtomic { corr, .. } => EffectKind::PersistAtomic(corr.raw()),
            Effect::PersistFire { .. } => EffectKind::PersistFire,
            Effect::Http { corr, .. } => EffectKind::Http(corr.raw()),
            Effect::HttpFire { .. } => EffectKind::HttpFire,
            Effect::Request { corr, .. } => EffectKind::Request(corr.raw()),
            Effect::Emit { .. } => EffectKind::Emit,
            Effect::ScheduleTimer { id, .. } => EffectKind::ScheduleTimer(id.raw()),
            Effect::CancelTimer { id } => EffectKind::CancelTimer(id.raw()),
            Effect::UploadFile { corr, .. } => EffectKind::UploadFile(corr.raw()),
            Effect::Send { .. } => EffectKind::Send,
        }
    }
}

/// 测试 fixture：数值索引 → 固定真实形态 26 字符 channel id。
#[allow(dead_code)]
pub fn cid(n: u64) -> String {
    format!("chfixx{n:020x}")
}

/// 测试 fixture：数值索引 → 固定真实形态 26 字符 server id。
#[allow(dead_code)]
pub fn sid(n: u64) -> String {
    format!("srvfix{n:020x}")
}

#[allow(dead_code)]
pub fn channel_id(n: u64) -> ChannelId {
    ChannelId::from_str(&cid(n)).expect("test channel id fixture must be valid Id26")
}

#[allow(dead_code)]
pub fn server_id(n: u64) -> ServerId {
    ServerId::from_str(&sid(n)).expect("test server id fixture must be valid Id26")
}

impl Harness {
    /// 创建 Harness，channel_ids 列表自动预注册（初始 cursor=0）
    #[allow(dead_code)]
    pub fn new(channel_ids: &[u64]) -> Self {
        Self::new_with_identity(channel_ids, "", "", "", "", "")
    }

    /// 以显式平台本地存储模式创建回放 harness（A2 三端 local-first）。
    #[allow(dead_code)]
    pub fn new_with_store_mode(channel_ids: &[u64], mode: LocalStoreMode) -> Self {
        let config = ImConfig {
            ws_url: "ws://test".to_string(),
            api_base_url: "http://test".to_string(),
            default_api_base_url: "http://localhost:3399".to_string(),
            ..Default::default()
        };
        let mut module = ImModule::new_with_local_store(config, mode);
        for &id in channel_ids {
            module.register_channel(channel_id(id), 0);
        }
        Self {
            module,
            sink: EffectSink::new(),
            effect_log: Vec::new(),
        }
    }

    /// 带注入身份的 Harness（P1 send userSnapshot 断言用）。
    ///
    /// 身份字段对应 ImConfig：auth_user_id / company_id / user_name / org_name / dept_name。
    #[allow(dead_code, clippy::too_many_arguments)]
    pub fn new_with_identity(
        channel_ids: &[u64],
        auth_user_id: &str,
        company_id: &str,
        user_name: &str,
        org_name: &str,
        dept_name: &str,
    ) -> Self {
        let config = ImConfig {
            ws_url: "ws://test".to_string(),
            api_base_url: "http://test".to_string(),
            default_api_base_url: "http://localhost:3399".to_string(),
            ping_interval_ms: 8_000,
            send_timeout_ms: 15_000,
            auth_user_id: auth_user_id.to_string(),
            company_id: company_id.to_string(),
            user_name: user_name.to_string(),
            org_name: org_name.to_string(),
            dept_name: dept_name.to_string(),
            client_platform: helix_im::ClientPlatform::Native,
            offline_sync_diagnostics: Default::default(),
        };
        let mut module = ImModule::new(config);
        for &id in channel_ids {
            module.register_channel(channel_id(id), 0);
        }
        let mut harness = Self {
            module,
            sink: EffectSink::new(),
            effect_log: Vec::new(),
        };
        // 非空 RuntimeAuth + 显式 channel_ids 代表可信本地频道行；空身份/空列表仍 fail-closed。
        if !auth_user_id.is_empty() && !company_id.is_empty() {
            for &id in channel_ids {
                harness.seed_channel_scope(&cid(id), company_id, auth_user_id);
            }
            // Fixture setup 不应改变被测首批 Effect 或 effect_log 的起点。
            harness.sink.clear();
            harness.effect_log.clear();
        }
        harness
    }

    /// 通过真实 MessageV3 提交 seam 建立可信频道归属；空身份不会被隐式授权。
    #[allow(dead_code)]
    pub fn seed_channel_scope(&mut self, channel_id: &str, team_id: &str, viewer_id: &str) {
        let row: Row = vec![
            ("id".to_string(), SqlValue::Text(channel_id.to_string())),
            ("team_id".to_string(), SqlValue::Text(team_id.to_string())),
            ("user_id".to_string(), SqlValue::Text(viewer_id.to_string())),
        ];
        let corr = self.module.queue_message_v3_commit(
            vec![StorageOp::BatchUpsert(UpsertSpec::new(
                "channel",
                vec![row],
                Some("id"),
            ))],
            Vec::new(),
            &mut self.sink,
        );
        self.step(port_reply_ok(corr.raw()), 0);
    }

    /// 取本批次最后一条 `Effect::Http` 的 body（解析为 JSON）。P1 send body 断言用。
    #[allow(dead_code)]
    pub fn last_http_body(&self) -> Option<serde_json::Value> {
        self.sink.as_slice().iter().rev().find_map(|e| match e {
            Effect::Http { req, .. } => req
                .body
                .as_ref()
                .and_then(|b| serde_json::from_slice(b.as_ref()).ok()),
            _ => None,
        })
    }

    /// 取本批次最后一条 `Effect::Http` 的 body 原始字节（casing grep 断言用：直接搜 raw 文本）。
    #[allow(dead_code)]
    pub fn last_http_body_raw(&self) -> Option<Vec<u8>> {
        self.sink.as_slice().iter().rev().find_map(|e| match e {
            Effect::Http { req, .. } => req.body.as_ref().map(|b| b.as_ref().to_vec()),
            _ => None,
        })
    }

    /// 执行 on_start，收集 Effects
    #[allow(dead_code)]
    pub fn start(&mut self, now_ms: u64) -> Vec<EffectKind> {
        use helix_core::Module;
        self.sink.clear();
        self.module.on_start(&mut self.sink).unwrap();
        let kinds = self.collect_kinds();
        let _ = now_ms;
        self.effect_log.push(kinds.clone());
        kinds
    }

    /// 喂一个 Tick，收集 Effects
    #[allow(dead_code)]
    pub fn step(&mut self, tick: Tick, now_ms: u64) -> Vec<EffectKind> {
        use helix_core::Module;
        self.sink.clear();
        self.module.handle(&tick, now_ms, &mut self.sink).unwrap();
        let kinds = self.collect_kinds();
        self.effect_log.push(kinds.clone());
        kinds
    }

    fn collect_kinds(&self) -> Vec<EffectKind> {
        self.sink
            .as_slice()
            .iter()
            .map(EffectKind::from_effect)
            .collect()
    }
}

/// 只读取本批实际 sync/notify Effect 的 correlation 与请求体，排除其他 HTTP。
#[allow(dead_code)]
pub fn sync_notify_requests(harness: &Harness) -> Vec<(u64, serde_json::Value)> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => Some((
                corr.raw(),
                serde_json::from_slice(req.body.as_ref().expect("notify body").as_ref())
                    .expect("notify JSON"),
            )),
            _ => None,
        })
        .collect()
}

/// 从当前 effect sink 找到 hello 触发的本地 message 水位扫描 correlation。
#[allow(dead_code)]
pub fn message_timestamp_scan_corr(sink: &EffectSink) -> Option<u64> {
    for effect in sink.as_slice() {
        let Effect::Persist { corr, ops } = effect else {
            continue;
        };
        match ops.as_slice() {
            [StorageOp::ScopedMax(spec)]
                if spec.table == "message"
                    && spec.scope_col == "channel_id"
                    && spec.value_col == "create_at" =>
            {
                return Some(corr.raw());
            }
            _ => {}
        }
    }
    None
}

// ─── 辅助构造器 ──────────────────────────────────────────────────────────────

/// 构造 PortReply Ok Tick（用于回报 Persist/Http 成功）
#[allow(dead_code)]
pub fn port_reply_ok(corr_raw: u64) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Ok(ReplyBytes::default()),
    }
}

/// 构造 PortReply Err Tick（用于回报 Http/Persist 失败，F1 反路验证）。
#[allow(dead_code)]
pub fn port_reply_err(corr_raw: u64) -> Tick {
    use helix_core::tick::PortError;
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Err(PortError::Http(500)),
    }
}

/// 构造 Timer Tick
#[allow(dead_code)]
pub fn timer_tick(id_raw: u64) -> Tick {
    Tick::Timer(TimerId::from_raw(id_raw))
}

/// 构造现役 Go `action=post` WS 帧，用于测试 gate 状态机。
#[allow(dead_code)]
pub fn inbound_event(channel_id: u64, seq: u64, _event_type: u8) -> Tick {
    use serde_json::json;
    let payload = json!({
        "action": "post",
        "data": {
            "id": sid(seq),
            "temporaryId": format!("tmp-{seq}"),
            "channelId": cid(channel_id),
            "event_seq": seq,
            "userId": "user_1",
            "type": "TEXT",
            "message": format!("message-{seq}"),
            "createAt": 1000000_u64 + seq
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Inbound(InboundBytes(bytes.into()))
}

/// 构造 hello 握手帧 Inbound（A4：现网 Go 连上后单向下发；触发 connection:established + resync）
#[allow(dead_code)]
pub fn hello_inbound(connection_id: &str) -> Tick {
    use serde_json::json;
    let payload = json!({
        "action": "hello",
        "data": { "connectionId": connection_id }
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Inbound(InboundBytes(bytes.into()))
}

/// 构造带已分配 ID 的 im_send_message replay Tick。
///
/// 常规公开入口不得提交 ID；大量既有状态机用例需要稳定锚点，故这里模拟 Helix
/// 已完成 allocation 后的内部 envelope。
#[allow(dead_code)]
pub fn send_message_cmd(channel_id: u64, temporary_id: &str, text: &str) -> Tick {
    use serde_json::json;
    let payload = json!({
        "channel_id": cid(channel_id),
        "allocated_temporary_id": temporary_id,
        "text": text
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new("im_send_message", bytes))
}

/// 构造 im_send_message replay Tick；若给 temporary_id，将其转成内部 allocation 字段。
#[allow(dead_code)]
pub fn send_message_cmd_json(mut payload: serde_json::Value) -> Tick {
    if let Some(object) = payload.as_object_mut() {
        if let Some(temporary_id) = object.remove("temporary_id") {
            object.insert("allocated_temporary_id".to_string(), temporary_id);
        }
    }
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new("im_send_message", bytes))
}

/// 构造任意命令 Tick（retry/upload 等非 send 命令）。
#[allow(dead_code)]
pub fn command_json(name: &'static str, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new(name, bytes))
}

/// 构造 PortReply Ok，带自定义 ReplyBytes（用于 scan / sync 回报）
#[allow(dead_code)]
pub fn port_reply_with_body(corr_raw: u64, body: bytes::Bytes) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Ok(ReplyBytes(body)),
    }
}

/// 标准 base64 编码（内联，与 host base64_encode 同表）。dev-zlc ADR-007 信封 body 用。
#[allow(dead_code)]
fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// 构造真 Go wire 的 sync 响应 body：`{status:SUCCESS, data:{entries:[entry], serverTime}}`。
/// 这是**裸 Go body**——直接喂 parse_sync_response 的 parser 单测用；走 handle_sync_reply 的集成
/// 测试须先经 `enveloped()` 包 ADR-007 信封（dev-zlc driver 回灌形态）。
fn sync_envelope(entry: serde_json::Value) -> bytes::Bytes {
    use serde_json::json;
    let body = json!({
        "status": "SUCCESS",
        "data": { "entries": [entry], "serverTime": 1_700_000_000_i64 }
    });
    bytes::Bytes::from(serde_json::to_vec(&body).unwrap())
}

/// 把裸 Go body 包成 ADR-007 HTTP 信封 `{status,headers,body:base64(raw)}`——即 host
/// run_http_envelope 对 2xx sync 响应的实际回灌形态。集成测试经 handle_sync_reply 灌 sync
/// PortReply 时须先包（handle_sync_reply 解信封 + base64 decode 后才喂 parse_sync_response）。
/// scan reply **不**经此（scan 路径非信封，见 scan_reply）。
#[allow(dead_code)]
pub fn enveloped(raw_go_body: bytes::Bytes) -> bytes::Bytes {
    use serde_json::json;
    let env = json!({
        "status": 200,
        "headers": {},
        "body": base64_std(raw_go_body.as_ref()),
    });
    bytes::Bytes::from(serde_json::to_vec(&env).unwrap())
}

/// 构造 sync 响应帧（真 Go wire `events` SyncEntry）。
///
/// 续拉与否由 batch 满批（≥500）派生，**不再**有显式 needs_continuation 字段
/// （真 wire 无此字段，见 parser::SYNC_EVENTS_PER_ENTRY）。
#[allow(dead_code)]
pub fn sync_reply_events(channel_id: u64, seqs: &[u64]) -> bytes::Bytes {
    use serde_json::json;
    let events: Vec<_> = seqs
        .iter()
        .map(|&seq| {
            json!({
                "channelId": cid(channel_id),
                "eventSeq": seq,
                "eventType": 1
            })
        })
        .collect();
    let next_seq = seqs.iter().copied().max();
    sync_envelope(json!({
        "channelId": cid(channel_id),
        "kind": "events",
        "events": events,
        "nextSeq": next_seq,
        "resetTo": serde_json::Value::Null,
    }))
}

/// 构造 `events` SyncEntry 并显式指定 nextSeq（用于测「cursor 锚 nextSeq 而非 max(event.seq)」）。
///
/// 真 wire 下 nextSeq==末条 seq，但服务端权威水位可 > 实际下发事件的 max（尾部 phantom seq
/// 不进 events 数组的契约漂移场景）——此时 cursor 必须跳到 nextSeq，不能停在 max(event.seq)。
#[allow(dead_code)]
pub fn sync_reply_events_with_next(channel_id: u64, seqs: &[u64], next_seq: u64) -> bytes::Bytes {
    use serde_json::json;
    let events: Vec<_> = seqs
        .iter()
        .map(|&seq| json!({ "channelId": cid(channel_id), "eventSeq": seq, "eventType": 1 }))
        .collect();
    sync_envelope(json!({
        "channelId": cid(channel_id),
        "kind": "events",
        "events": events,
        "nextSeq": next_seq,
        "resetTo": serde_json::Value::Null,
    }))
}

/// 构造真 Go wire 的 `no_change` SyncEntry（续拉链终止信号，nextSeq=maxSeq）。
#[allow(dead_code)]
pub fn sync_reply_no_change(channel_id: u64, next_seq: u64) -> bytes::Bytes {
    use serde_json::json;
    sync_envelope(json!({
        "channelId": cid(channel_id),
        "kind": "no_change",
        "events": [],
        "nextSeq": next_seq,
        "resetTo": serde_json::Value::Null,
    }))
}

/// 构造真 Go wire 的 `too_long` SyncEntry（resetTo=maxSeq，客户端清表重拉）。
#[allow(dead_code)]
pub fn sync_reply_too_long(channel_id: u64, reset_to: u64) -> bytes::Bytes {
    use serde_json::json;
    sync_envelope(json!({
        "channelId": cid(channel_id),
        "kind": "too_long",
        "events": [],
        "nextSeq": serde_json::Value::Null,
        "resetTo": reset_to,
    }))
}

/// 构造 scan 回报（channel_event_cursor 扫描结果，S2）
///
/// 格式遵循 engine_loop rows_to_reply_bytes 约定：
/// `[{"channel_id": "<id26>", "last_event_seq": N, "updated_at": T}, ...]`。
/// channel_id 是 TEXT（26 字符 id），last_event_seq 是 INTEGER cursor seq。
/// 对应 advance_cursor 写路径（acl/to_effect.rs：MonotonicUpsert{table=channel_event_cursor,
/// key_col=channel_id, value_col=last_event_seq}）。
#[allow(dead_code)]
pub fn scan_reply(rows: &[(u64, u64)]) -> bytes::Bytes {
    use serde_json::json;
    let arr: Vec<_> = rows
        .iter()
        .map(|&(channel_id, cursor_seq)| {
            json!({
                "channel_id": cid(channel_id),
                "last_event_seq": cursor_seq as i64,
                "updated_at": 0i64
            })
        })
        .collect();
    bytes::Bytes::from(serde_json::to_vec(&arr).unwrap())
}

/// 构造 channel 投影扫描回报，用于启动阶段删除/关闭频道 terminal 过滤。
#[allow(dead_code)]
pub fn channel_projection_scan_reply(rows: &[(u64, bool, u64)]) -> bytes::Bytes {
    use serde_json::json;
    let arr: Vec<_> = rows
        .iter()
        .map(|&(channel_id, deleted, last_event_seq)| {
            json!({
                "id": cid(channel_id),
                "delete_at": if deleted { 1_i64 } else { 0_i64 },
                "is_remove": 0_i64,
                "last_event_seq": last_event_seq as i64,
            })
        })
        .collect();
    bytes::Bytes::from(serde_json::to_vec(&arr).unwrap())
}

/// 构造带可信频道公司/本地 viewer 归属的启动扫描回报；稀疏旧 fixture 不得冒充授权行。
#[allow(dead_code)]
pub fn channel_projection_scan_reply_with_scope(
    rows: &[(u64, bool, u64)],
    team_id: &str,
    viewer_id: &str,
) -> bytes::Bytes {
    use serde_json::json;
    let arr: Vec<_> = rows
        .iter()
        .map(|&(channel_id, deleted, last_event_seq)| {
            json!({
                "id": cid(channel_id),
                "team_id": team_id,
                "user_id": viewer_id,
                "delete_at": if deleted { 1_i64 } else { 0_i64 },
                "is_remove": 0_i64,
                "last_event_seq": last_event_seq as i64,
            })
        })
        .collect();
    bytes::Bytes::from(serde_json::to_vec(&arr).unwrap())
}

// ─── C2：sync events + messages map（phantom / readBits / edit / revoke）────────

/// 单条 ChannelEvent 描述（C2 测试用）：seq + type + msgId。
#[allow(dead_code)]
pub struct EvSpec {
    pub seq: u64,
    pub event_type: u8,
    pub msg_id: &'static str,
}

#[allow(dead_code)]
pub fn ev(seq: u64, event_type: u8, msg_id: &'static str) -> EvSpec {
    EvSpec {
        seq,
        event_type,
        msg_id,
    }
}

/// 构造 sync `events` 响应，显式给定 events 列表 + messages 内容 map（C2 真源对齐）。
///
/// - `events`：每条带 msgId（phantom 判定靠 msgId 是否在 messages map）。
/// - `messages`：`(msg_id, content_text, read_bits)` 三元组——content_text 落 message 列，
///   read_bits 落 read_bits 列（type6 单调覆盖源）。不在此列表的 msgId = phantom。
/// - `next_seq`：服务端权威水位（cursor 推进锚点）。
#[allow(dead_code)]
pub fn sync_reply_events_with_messages(
    channel_id: u64,
    events: &[EvSpec],
    messages: &[(&str, &str, &str)],
    next_seq: u64,
) -> bytes::Bytes {
    use serde_json::json;
    let ev_json: Vec<_> = events
        .iter()
        .map(|e| {
            json!({
                "channelId": cid(channel_id),
                "eventSeq": e.seq,
                "eventType": e.event_type,
                "msgId": e.msg_id,
            })
        })
        .collect();
    let mut msg_map = serde_json::Map::new();
    for (mid, text, read_bits) in messages {
        msg_map.insert(
            mid.to_string(),
            json!({
                "id": mid,
                "temporary_id": mid,
                "channel_id": cid(channel_id),
                "message": text,
                "userId": "sender-678",
                "readBits": read_bits,
                "viewers": ["viewer-444"],
            }),
        );
    }
    sync_envelope(json!({
        "channelId": cid(channel_id),
        "kind": "events",
        "events": ev_json,
        "messages": serde_json::Value::Object(msg_map),
        "nextSeq": next_seq,
        "resetTo": serde_json::Value::Null,
    }))
}
