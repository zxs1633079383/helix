//! EFFECT-3 回归：多模块 start() 的 Correlation 必须定向投递到**发起模块**。
//!
//! 回归锚: EFFECT-3（core/driver 基座 review wf_ba3cc60b）
//!
//! 防止 `ExecutionShell::start()` 跨模块累积 scratch 时，后注册模块的
//! `register_effects_to_maps` 把前序模块 on_start 的 Correlation 重复扫描并
//! 覆盖到自己 → PortReply 投错家。直接威胁北极星「第二个业务模块插得进」。

use bytes::Bytes;
use helix_core::effect::Correlation;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{CoreError, DomainEventBytes, Effect, EffectSink, ExecutionShell, Module, Tick};

/// 测试桩：`on_start` 吐 `Request{corr=self.corr}`；收到 `PortReply` 时吐 `Emit(自身 marker)`，
/// 供断言「该 corr 回投到了哪个模块」。
struct StartCorrModule {
    corr: u64,
    marker: &'static str,
}

impl Module for StartCorrModule {
    fn name(&self) -> &'static str {
        self.marker
    }

    fn accepts(&self, _tick: &Tick) -> bool {
        false // 只走 on_start + PortReply(corr 定向路由)，不参与 Inbound 线性路由
    }

    fn handle(&mut self, tick: &Tick, _now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        if let Tick::PortReply { .. } = tick {
            out.push(Effect::Emit {
                event: DomainEventBytes(Bytes::from_static(self.marker.as_bytes())),
            });
        }
        Ok(())
    }

    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        out.push(Effect::Request {
            corr: Correlation::from_raw(self.corr),
            kind: "init",
            payload: Bytes::new(),
        });
        Ok(())
    }
}

fn reply(corr: u64) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr),
        outcome: PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"r"))),
    }
}

/// 两个模块各自 on_start 吐 `Request{corr}`，start() 后每个 corr 必须路由回**发起它的**模块。
/// EFFECT-3 bug 下，后注册的 mod_b 会覆盖 mod_a 的 corr → corr=100 错投 mod_b（断言 mod_a 即失败）。
#[test]
fn start_corr_routes_to_originating_module() {
    let mut shell = ExecutionShell::new();
    shell.register(StartCorrModule {
        corr: 100,
        marker: "mod_a",
    });
    shell.register(StartCorrModule {
        corr: 200,
        marker: "mod_b",
    });
    let start_effects = shell.start().expect("start");
    assert_eq!(start_effects.len(), 2, "两个模块各吐一个 on_start Request");

    // corr=100 由 mod_a 发起 → 必须回投 mod_a（而非最后注册的 mod_b）
    let e = shell.step(reply(100), 1_000).expect("reply 100");
    assert_eq!(e.len(), 1, "corr=100 应定向投递恰好一个模块");
    match &e[0] {
        Effect::Emit { event } => assert_eq!(
            &event.0[..],
            b"mod_a",
            "corr=100 必须路由到发起它的 mod_a，而非最后注册的 mod_b（EFFECT-3 回归）"
        ),
        other => panic!("expected Emit, got {other:?}"),
    }

    // corr=200 由 mod_b 发起 → 必须回投 mod_b
    let e = shell.step(reply(200), 1_001).expect("reply 200");
    match &e[0] {
        Effect::Emit { event } => assert_eq!(&event.0[..], b"mod_b", "corr=200 必须路由到 mod_b"),
        other => panic!("expected Emit, got {other:?}"),
    }
}
