//! 公共测试辅助（测试 harness）
//!
//! ## 使用方式
//!
//! 在各测试文件顶部：
//! ```
//! mod common;
//! use common::*;
//! ```

use helix_core::effect::{Correlation, TimerId};
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Tick};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
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
    Persist(u64), // corr.raw()
    PersistFire,
    Http(u64),    // corr.raw()
    Request(u64), // corr.raw()（REP 方向① 发起端）
    Emit,
    ScheduleTimer(u64), // timer_id.raw()
    CancelTimer(u64),   // timer_id.raw()
    Send,
}

impl EffectKind {
    pub fn from_effect(e: &Effect) -> Self {
        match e {
            Effect::Persist { corr, .. } => EffectKind::Persist(corr.raw()),
            Effect::PersistFire { .. } => EffectKind::PersistFire,
            Effect::Http { corr, .. } => EffectKind::Http(corr.raw()),
            Effect::Request { corr, .. } => EffectKind::Request(corr.raw()),
            Effect::Emit { .. } => EffectKind::Emit,
            Effect::ScheduleTimer { id, .. } => EffectKind::ScheduleTimer(id.raw()),
            Effect::CancelTimer { id } => EffectKind::CancelTimer(id.raw()),
            Effect::Send { .. } => EffectKind::Send,
        }
    }
}

/// 测试桥接：数值 channel id → 真 wire 的 26 字符字符串（与 ChannelId::from_test_u64 一致）。
#[allow(dead_code)]
pub fn cid(n: u64) -> String {
    ChannelId::from_test_u64(n).as_str().to_string()
}

impl Harness {
    /// 创建 Harness，channel_ids 列表自动预注册（初始 cursor=0）
    pub fn new(channel_ids: &[u64]) -> Self {
        let config = ImConfig {
            ws_url: "ws://test".to_string(),
            api_base_url: "http://test".to_string(),
            ping_interval_ms: 8_000,
            send_timeout_ms: 15_000,
        };
        let mut module = ImModule::new(config);
        for &id in channel_ids {
            module.register_channel(ChannelId::from_test_u64(id), 0);
        }
        Self {
            module,
            sink: EffectSink::new(),
            effect_log: Vec::new(),
        }
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

// ─── 辅助构造器 ──────────────────────────────────────────────────────────────

/// 构造 PortReply Ok Tick（用于回报 Persist/Http 成功）
#[allow(dead_code)]
pub fn port_reply_ok(corr_raw: u64) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Ok(ReplyBytes::default()),
    }
}

/// 构造 Timer Tick
#[allow(dead_code)]
pub fn timer_tick(id_raw: u64) -> Tick {
    Tick::Timer(TimerId::from_raw(id_raw))
}

/// 构造带 seq 的 WS Inbound 帧（JSON，用于测试 gate 状态机）
#[allow(dead_code)]
pub fn inbound_event(channel_id: u64, seq: u64, event_type: u8) -> Tick {
    use serde_json::json;
    let payload = json!({
        "event": "posted",
        "data": {
            "channel_id": cid(channel_id),
            "seq": seq,
            "event_type": event_type,
            "msg_id": format!("msg_{}", seq),
            "actor_id": "user_1",
            "created_at": 1000000_u64,
            "payload": null
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

/// 构造 im_send_message 命令 Tick
#[allow(dead_code)]
pub fn send_message_cmd(channel_id: u64, temporary_id: &str, text: &str) -> Tick {
    use serde_json::json;
    let payload = json!({
        "channel_id": cid(channel_id),
        "temporary_id": temporary_id,
        "text": text
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new("im_send_message", bytes))
}

/// 构造 PortReply Ok，带自定义 ReplyBytes（用于 scan / sync 回报）
#[allow(dead_code)]
pub fn port_reply_with_body(corr_raw: u64, body: bytes::Bytes) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Ok(ReplyBytes(body)),
    }
}

/// 构造 sync 响应帧（events 类型，可控 needs_continuation）
#[allow(dead_code)]
pub fn sync_reply_events(channel_id: u64, seqs: &[u64], needs_continuation: bool) -> bytes::Bytes {
    use serde_json::json;
    let events: Vec<_> = seqs
        .iter()
        .map(|&seq| {
            json!({
                "channel_id": cid(channel_id),
                "seq": seq,
                "event_type": 1
            })
        })
        .collect();
    let body = json!({
        "type": "events",
        "events": events,
        "needs_continuation": needs_continuation
    });
    bytes::Bytes::from(serde_json::to_vec(&body).unwrap())
}

/// 构造 scan 回报（_helix_monotonic 扫描结果）
///
/// 格式遵循 engine_loop rows_to_reply_bytes 约定：`[{"scope_key": "<channel_id>", "value": N}, ...]`
/// scope_key 是 TEXT（channel_id.to_string()），value 是 INTEGER cursor seq。
/// 对应 advance_cursor 写路径（acl/to_effect.rs：MonotonicUpsert{scope_key=channel_id.to_string(), value=seq}）
#[allow(dead_code)]
pub fn scan_reply(rows: &[(u64, u64)]) -> bytes::Bytes {
    use serde_json::json;
    let arr: Vec<_> = rows
        .iter()
        .map(|&(channel_id, cursor_seq)| {
            json!({
                "scope_key": cid(channel_id),
                "value": cursor_seq as i64
            })
        })
        .collect();
    bytes::Bytes::from(serde_json::to_vec(&arr).unwrap())
}
