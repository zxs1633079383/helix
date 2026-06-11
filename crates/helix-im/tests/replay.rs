//! 回放测试基建（Phase B Commit 1）
//!
//! ## 设计意图
//!
//! - `Harness` 持有 `ImModule`，通过 `step()` 调用处理一批 Tick，收集 Effect 序列
//! - 所有测试均使用确定性时间（now_ms 注入），零真实 I/O，零 async
//! - `assert_effect!` / `assert_has_persist!` / `assert_no_pending_sends!` 宏提供
//!   人类可读的失败信息
//!
//! ## 不变量
//!
//! - Harness::step 输出 Effect 的顺序与 ImModule 吐出顺序完全一致（无乱序）
//! - Harness 不 clone Effects，collect_effects() 按 step 批次分组

use helix_core::{Effect, EffectSink, Tick};
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome, ReplyBytes};
use helix_core::effect::{Correlation, TimerId};
use helix_im::{ImModule};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;

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
    Persist(u64),     // corr.raw()
    PersistFire,
    Http(u64),        // corr.raw()
    Emit,
    ScheduleTimer(u64), // timer_id.raw()
    CancelTimer(u64),   // timer_id.raw()
    Send,
}

impl EffectKind {
    fn from_effect(e: &Effect) -> Self {
        match e {
            Effect::Persist { corr, .. }      => EffectKind::Persist(corr.raw()),
            Effect::PersistFire { .. }         => EffectKind::PersistFire,
            Effect::Http { corr, .. }          => EffectKind::Http(corr.raw()),
            Effect::Emit { .. }                => EffectKind::Emit,
            Effect::ScheduleTimer { id, .. }   => EffectKind::ScheduleTimer(id.raw()),
            Effect::CancelTimer { id }         => EffectKind::CancelTimer(id.raw()),
            Effect::Send { .. }                => EffectKind::Send,
        }
    }
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
            module.register_channel(ChannelId(id), 0);
        }
        Self {
            module,
            sink: EffectSink::new(),
            effect_log: Vec::new(),
        }
    }

    /// 执行 on_start，收集 Effects
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

    /// 喂多个 Tick，收集所有 Effects（展平）
    pub fn step_all(&mut self, ticks: Vec<Tick>, now_ms: u64) -> Vec<EffectKind> {
        ticks.into_iter().flat_map(|t| self.step(t, now_ms)).collect()
    }

    fn collect_kinds(&self) -> Vec<EffectKind> {
        self.sink.as_slice().iter().map(EffectKind::from_effect).collect()
    }

    /// 从 effect_log 中提取全部 Persist corr（用于构造 PortReply Ok）
    pub fn all_persist_corrs(&self) -> Vec<u64> {
        self.sink.as_slice().iter().filter_map(|e| {
            if let Effect::Persist { corr, .. } = e { Some(corr.raw()) } else { None }
        }).collect()
    }

    /// 从当前 sink 中提取全部 Http corr
    pub fn all_http_corrs(&self) -> Vec<u64> {
        self.sink.as_slice().iter().filter_map(|e| {
            if let Effect::Http { corr, .. } = e { Some(corr.raw()) } else { None }
        }).collect()
    }

    /// 从当前 sink 中提取全部 ScheduleTimer ids
    pub fn all_timer_ids(&self) -> Vec<u64> {
        self.sink.as_slice().iter().filter_map(|e| {
            if let Effect::ScheduleTimer { id, .. } = e { Some(id.raw()) } else { None }
        }).collect()
    }
}

// ─── 辅助构造器 ──────────────────────────────────────────────────────────────

/// 构造 PortReply Ok Tick（用于回报 Persist/Http 成功）
pub fn port_reply_ok(corr_raw: u64) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr_raw),
        outcome: PortOutcome::Ok(ReplyBytes::default()),
    }
}

/// 构造 Timer Tick
pub fn timer_tick(id_raw: u64) -> Tick {
    Tick::Timer(TimerId::from_raw(id_raw))
}

/// 构造带 seq 的 WS Inbound 帧（JSON，用于测试 gate 状态机）
pub fn inbound_event(channel_id: u64, seq: u64, event_type: u8) -> Tick {
    use serde_json::json;
    let payload = json!({
        "event": "posted",
        "data": {
            "channel_id": channel_id,
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

/// 构造 im_send_message 命令 Tick
pub fn send_message_cmd(channel_id: u64, temporary_id: &str, text: &str) -> Tick {
    use serde_json::json;
    let payload = json!({
        "channel_id": channel_id,
        "temporary_id": temporary_id,
        "text": text
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new("im_send_message", bytes))
}

// ─── 冒烟测试 ────────────────────────────────────────────────────────────────

#[test]
fn smoke_harness_on_start_emits_schedule_timer() {
    let mut h = Harness::new(&[42]);
    let effects = h.start(0);
    // on_start 至少应包含一个 ScheduleTimer（ping 心跳）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "on_start should emit ScheduleTimer for ping, got: {:?}",
        effects
    );
}

#[test]
fn smoke_inbound_returns_without_panic() {
    let mut h = Harness::new(&[1]);
    // parser 目前是 stub（返回 Parse 错误），inbound 应静默 drop，不 panic
    let ev = inbound_event(1, 1, 1);
    let effects = h.step(ev, 1000);
    // 应不 panic；Effects 可以是空（parser stub 还未实现时 drop 掉）
    let _ = effects;
}

#[test]
fn smoke_port_reply_ok_does_not_panic() {
    let mut h = Harness::new(&[1]);
    // 喂一个没有对应 corr 的 PortReply，应静默忽略
    let effects = h.step(port_reply_ok(9999), 2000);
    assert!(effects.is_empty(), "unknown corr PortReply should produce no effects");
}

#[test]
fn smoke_timer_fires_without_panic() {
    let mut h = Harness::new(&[1]);
    let start_effects = h.start(0);
    // 找到 ping timer id
    let timer_ids: Vec<u64> = start_effects.iter().filter_map(|e| {
        if let EffectKind::ScheduleTimer(id) = e { Some(*id) } else { None }
    }).collect();
    assert!(!timer_ids.is_empty(), "should have at least one timer from on_start");
    // fire the timer
    let effects = h.step(timer_tick(timer_ids[0]), 8_001);
    // ping timer 重新 arm → 应再有一个 ScheduleTimer
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "ping timer re-arm should emit another ScheduleTimer, got: {:?}",
        effects
    );
}
