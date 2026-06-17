//! 回放测试基建冒烟测试（Phase B Commit 1）
//!
//! 验证 Harness 骨架可用、on_start 行为符合预期。

mod common;
use common::*;
use helix_core::tick::InboundBytes;
use helix_core::Tick;

#[test]
fn smoke_harness_on_start_emits_schedule_timer() {
    let mut h = Harness::new(&[42]);
    let effects = h.start(0);
    // on_start 至少应包含一个 ScheduleTimer（ping 心跳）
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "on_start should emit ScheduleTimer for ping, got: {:?}",
        effects
    );
}

#[test]
fn smoke_inbound_unknown_parse_does_not_panic() {
    let mut h = Harness::new(&[1]);
    // 用 inbound_event 构造 parse 可成功的帧（JSON 格式符合）
    let ev = inbound_event(1, 1, 1);
    let effects = h.step(ev, 1000);
    // 应不 panic；对已注册 channel=1 的 seq=1，应产出 PersistFire（连续帧直接放行）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::PersistFire)),
        "valid inbound event should emit PersistFire, got: {:?}",
        effects
    );
}

#[test]
fn invalid_json_inbound_is_ignored_without_effects() {
    let mut h = Harness::new(&[1]);

    let effects = h.step(
        Tick::Inbound(InboundBytes(bytes::Bytes::from_static(b"{"))),
        1000,
    );

    assert!(
        effects.is_empty(),
        "invalid JSON inbound should be ignored without effects, got: {:?}",
        effects
    );
}

#[test]
fn smoke_port_reply_ok_does_not_panic() {
    let mut h = Harness::new(&[1]);
    // 喂一个没有对应 corr 的 PortReply，应静默忽略
    let effects = h.step(port_reply_ok(9999), 2000);
    assert!(
        effects.is_empty(),
        "unknown corr PortReply should produce no effects"
    );
}

#[test]
fn smoke_timer_fires_without_panic() {
    let mut h = Harness::new(&[1]);
    let start_effects = h.start(0);
    // 找到 ping timer id
    let timer_ids: Vec<u64> = start_effects
        .iter()
        .filter_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    assert!(
        !timer_ids.is_empty(),
        "should have at least one timer from on_start"
    );
    // fire the timer（on_start 产出的 scan Persist 和 ping timer；ping timer 在 Disconnected 不发 ping 但仍 re-arm）
    let effects = h.step(timer_tick(timer_ids[0]), 8_001);
    // ping timer 重新 arm → 应再有一个 ScheduleTimer
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "ping timer re-arm should emit another ScheduleTimer, got: {:?}",
        effects
    );
}
