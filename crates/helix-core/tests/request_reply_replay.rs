//! 方向① 最小垂直切片回放测试：EventBus REP 模式 core↔host↔core 请求-应答闭环。
//!
//! 回归锚: EventBus REP 方向①（core 主动问 host）
//!
//! 验证：模块吐 `Effect::Request{corr}` → driver 兑现 → `Tick::PortReply{corr}`
//! → `corr_map` 定向回投同一模块 → 模块据回复吐后续 Effect。
//!
//! 全程零网络零库，纯状态机回放（sans-IO），毫秒级。

use bytes::Bytes;
use helix_core::effect::Correlation;
use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
use helix_core::{CoreError, DomainEventBytes, Effect, EffectSink, ExecutionShell, Module, Tick};

/// 测试桩模块：演示 EventBus REP 方向① 闭环。
///
/// 收到 UI 的 `Command("ask")` → 吐 `Effect::Request{corr}` 向宿主提问；
/// 收到 `Tick::PortReply{corr}` → 据宿主回复吐 `Effect::Emit`（把决策广播出去）。
struct ReqReplyModule {
    /// 模块本地 corr 分配器（单调递增；真实模块走 `ImModule::alloc_corr`）
    next_corr: u64,
    /// 记录最近发起的 corr，用于断言回复确实回到本模块（corr 配对不变量）
    last_request_corr: Option<Correlation>,
}

impl ReqReplyModule {
    fn new() -> Self {
        Self {
            next_corr: 1,
            last_request_corr: None,
        }
    }

    fn alloc_corr(&mut self) -> Correlation {
        let c = Correlation::from_raw(self.next_corr);
        self.next_corr += 1;
        c
    }
}

impl Module for ReqReplyModule {
    fn name(&self) -> &'static str {
        "req_reply_test"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        // 只认 Command("ask")；PortReply 不走 accepts（走 corr_map 定向投递）
        matches!(tick, Tick::Command(cmd) if cmd.name.as_ref() == "ask")
    }

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        match tick {
            Tick::Command(cmd) if cmd.name.as_ref() == "ask" => {
                // 方向① 发起：吐 Request{corr}，把 UI 载荷转给宿主提问
                let corr = self.alloc_corr();
                self.last_request_corr = Some(corr);
                out.push(Effect::Request {
                    corr,
                    kind: "ask_user",
                    payload: cmd.payload.clone(),
                });
            }
            Tick::PortReply { corr, outcome } => {
                // 回复必须是本模块发起的那个 corr（定向回投不变量）
                assert_eq!(
                    Some(*corr),
                    self.last_request_corr,
                    "reply corr 必须等于本模块发起的 request corr"
                );
                // 据宿主回复吐后续 Effect（用 Emit 表示"把宿主决策广播出去"）
                if let PortOutcome::Ok(ReplyBytes(bytes)) = outcome {
                    out.push(Effect::Emit {
                        event: DomainEventBytes(bytes.clone()),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn ask_command(payload: &'static [u8]) -> Tick {
    Tick::Command(AppCommand::new("ask", payload))
}

/// 主闭环：Command("ask") → Request{corr} →（宿主）→ PortReply{corr} → Emit。
#[test]
fn direction_one_request_reply_loop() {
    let mut shell = ExecutionShell::new();
    shell.register(ReqReplyModule::new());
    shell.start().expect("start");

    // 1. UI 发 Command("ask") → 模块吐 Effect::Request{corr=1}
    let effects = shell.step(ask_command(b"who"), 1_000).expect("step ask");
    assert_eq!(effects.len(), 1, "ask 应只产出一个 Request");
    let issued = match &effects[0] {
        Effect::Request {
            corr,
            kind,
            payload,
        } => {
            assert_eq!(*kind, "ask_user");
            assert_eq!(&payload[..], b"who", "请求载荷应透传 UI 命令载荷");
            *corr
        }
        other => panic!("expected Effect::Request, got {other:?}"),
    };
    assert_eq!(issued.raw(), 1, "corr 由模块分配器单调发放，首个为 1");

    // 2. 宿主处理完，回喂 PortReply{corr=1, Ok} → corr_map 定向回投 → 模块吐 Emit
    let reply = Tick::PortReply {
        corr: issued,
        outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"yes"))),
    };
    let effects = shell.step(reply, 2_000).expect("step reply");
    assert_eq!(effects.len(), 1, "回复应驱动恰好一个后续 Emit");
    match &effects[0] {
        Effect::Emit { event } => assert_eq!(&event.0[..], b"yes", "Emit 应携带宿主回复字节"),
        other => panic!("expected Effect::Emit, got {other:?}"),
    }
}

/// corr 命名空间隔离不变量：corr_map 里没有的 corr（如宿主自造 / 迟到响应），
/// 静默忽略，模块 handle 根本不被调用，零 Effect 产出。
#[test]
fn unknown_corr_reply_silently_ignored() {
    let mut shell = ExecutionShell::new();
    shell.register(ReqReplyModule::new());
    shell.start().expect("start");

    let reply = Tick::PortReply {
        corr: Correlation::from_raw(999),
        outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"orphan"))),
    };
    let effects = shell.step(reply, 1_000).expect("step orphan reply");
    assert!(
        effects.is_empty(),
        "未登记 corr 的回复必须零 Effect（不串台到任意模块）"
    );
}

/// 错误回复也能定向回投（PortError 分类回路与 Ok 同源）。
#[test]
fn direction_one_error_reply_routes_back() {
    let mut shell = ExecutionShell::new();
    shell.register(ReqReplyModule::new());
    shell.start().expect("start");

    let effects = shell.step(ask_command(b"q"), 1_000).expect("step ask");
    let issued = match &effects[0] {
        Effect::Request { corr, .. } => *corr,
        other => panic!("expected Effect::Request, got {other:?}"),
    };

    // 宿主返回错误：回路相同（corr_map 定向回投），模块据策略不吐 Emit
    let reply = Tick::PortReply {
        corr: issued,
        outcome: PortOutcome::Err(helix_core::tick::PortError::Timeout),
    };
    // handle 内的 corr 断言会在此被触发——证明 Err 回复也定向到了发起模块
    let effects = shell.step(reply, 2_000).expect("step err reply");
    assert!(effects.is_empty(), "本桩模块对 Err 回复不产出 Emit");
}

/// 确定性回放：同 Tick 序列 + 同模块初态 ⇒ 同 Effect 序列。
#[test]
fn replay_determinism_same_ticks_same_effects() {
    fn run() -> Vec<String> {
        let mut shell = ExecutionShell::new();
        shell.register(ReqReplyModule::new());
        shell.start().expect("start");

        let mut trace = Vec::new();
        let e = shell.step(ask_command(b"who"), 1_000).expect("ask");
        trace.push(format!("{e:?}"));
        let corr = match &e[0] {
            Effect::Request { corr, .. } => *corr,
            _ => unreachable!("first effect must be Request"),
        };
        let e = shell
            .step(
                Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"yes"))),
                },
                2_000,
            )
            .expect("reply");
        trace.push(format!("{e:?}"));
        trace
    }

    assert_eq!(run(), run(), "同 Tick 序列必须产出完全一致的 Effect 序列");
}
