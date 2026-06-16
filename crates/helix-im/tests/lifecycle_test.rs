//! 生命周期测试（Phase B Commit 4）
//!
//! 验证：
//! - on_start：吐 Persist{Scan} + ScheduleTimer{ping}
//! - Tick::Connected：proactive resync（对所有注册 channel 各发 Http{sync/notify}）
//! - Tick::Disconnected：静默标记 offline（无 Effect）
//! - on_stop：取消所有 timer

mod common;
use common::*;

use helix_core::effect::TransportId;
use helix_core::Tick;

// ─── on_start 行为 ────────────────────────────────────────────────────────────

#[test]
fn on_start_emits_scan_persist_and_ping_timer() {
    let mut h = Harness::new(&[]);
    let effects = h.start(0);

    // 应包含 Persist（Scan cursor）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Persist(_))),
        "on_start should emit Persist(Scan), got: {:?}",
        effects
    );
    // 应包含 ScheduleTimer（ping）
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "on_start should emit ScheduleTimer(ping), got: {:?}",
        effects
    );
    // 不应产出 Http（on_start 不主动 sync，等 Connected 后才 proactive resync）
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "on_start should NOT emit Http, got: {:?}",
        effects
    );
}

#[test]
fn on_start_with_channels_still_no_http() {
    // 即使预注册了 channel，on_start 也不应发 Http（等 Connected 触发 proactive resync）
    let mut h = Harness::new(&[1, 2, 3]);
    let effects = h.start(0);
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "on_start with channels should NOT emit Http (proactive resync is in Connected), got: {:?}",
        effects
    );
}

// ─── Tick::Connected → proactive resync ──────────────────────────────────────

#[test]
fn connected_triggers_proactive_resync_for_each_channel() {
    let mut h = Harness::new(&[1, 2, 3]);
    h.start(0);

    // A4：Connected 只置 Connecting，resync 在 hello 握手后
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    let hello_effects = h.step(hello_inbound("CONN_1"), 600);

    // 应对 3 个 channel 各发一个 Http（sync/notify）
    let http_count = hello_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 3,
        "hello 握手后应对 3 个 channel 发 3 个 Http sync/notify, got: {:?}",
        hello_effects
    );
}

#[test]
fn connected_with_no_channels_emits_no_http() {
    let mut h = Harness::new(&[]);
    h.start(0);
    let connected_effects = h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    let http_count = connected_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 0,
        "Connected with no channels should emit no Http, got: {:?}",
        connected_effects
    );
}

#[test]
fn connected_each_channel_gets_different_corr() {
    let mut h = Harness::new(&[10, 20, 30]);
    h.start(0);
    // A4：resync 在 hello 握手后
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    let effects = h.step(hello_inbound("CONN_1"), 600);

    let http_corrs: Vec<u64> = effects
        .iter()
        .filter_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(http_corrs.len(), 3);
    // 所有 corr 应互不相同
    let unique: std::collections::HashSet<u64> = http_corrs.iter().copied().collect();
    assert_eq!(unique.len(), 3, "each sync Http should have a unique corr");
}

// ─── Tick::Disconnected → 静默 + 状态标记 ────────────────────────────────────

#[test]
fn disconnected_produces_no_effects() {
    let mut h = Harness::new(&[1]);
    h.start(0);
    // 先连接
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    // 再断线
    let disconnect_effects = h.step(Tick::Disconnected(TransportId::from_raw(0)), 1000);
    assert!(
        disconnect_effects.is_empty(),
        "Disconnected should produce no effects, got: {:?}",
        disconnect_effects
    );
}

#[test]
fn disconnected_then_reconnect_triggers_new_resync() {
    // 断线后重连，Connected 再次触发 proactive resync
    let mut h = Harness::new(&[1, 2]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_1"), 600); // 首次握手
    h.step(Tick::Disconnected(TransportId::from_raw(0)), 1000);

    // 第二次连接 + 重新握手（hello 带新 connectionId）
    h.step(Tick::Connected(TransportId::from_raw(0)), 2000);
    let reconnect_effects = h.step(hello_inbound("CONN_2"), 2100);
    let http_count = reconnect_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 2,
        "reconnect 后再次 hello 应对所有 channel 重新 resync"
    );
}

// ─── ping timer 跨连接状态测试 ────────────────────────────────────────────────

#[test]
fn ping_timer_fires_only_when_connected() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let ping_timer = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("should have ping timer from on_start");

    // Disconnected 状态下 ping timer 触发 → 不应吐 Send 帧（只 re-arm timer）
    let disconnect_effects = h.step(Tick::Disconnected(TransportId::from_raw(0)), 500);
    assert!(disconnect_effects.is_empty());

    let timer_effects = h.step(timer_tick(ping_timer), 8001);
    // 应 re-arm（ScheduleTimer）
    assert!(
        timer_effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "ping timer should re-arm in disconnected state"
    );
    // 不应发 Send（未连接）
    assert!(
        !timer_effects.iter().any(|e| matches!(e, EffectKind::Send)),
        "ping timer should NOT emit Send when disconnected"
    );
}

#[test]
fn ping_timer_fires_send_when_connected() {
    let mut h = Harness::new(&[]);
    let start_effects = h.start(0);
    let ping_timer = start_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("should have ping timer from on_start");

    // 连接建立 + hello 握手（A4：conn=Connected 才发 ping Send）
    h.step(Tick::Connected(TransportId::from_raw(0)), 500);
    h.step(hello_inbound("CONN_1"), 600);

    let timer_effects = h.step(timer_tick(ping_timer), 8001);
    // 应有 Send（ping 帧）
    assert!(
        timer_effects.iter().any(|e| matches!(e, EffectKind::Send)),
        "ping timer should emit Send when connected"
    );
    // 应 re-arm
    assert!(
        timer_effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "ping timer should re-arm"
    );
}
