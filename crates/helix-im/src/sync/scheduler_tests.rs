//! SyncScheduler 单测（外提自 sync_scheduler.rs，结构闸 ≤300 行：源文件压回基线，
//! 测试入 sibling，对齐 state.rs `mod id26_tests` → state_tests.rs 同款外提范式）。

use super::*;
use crate::channel::Channel;
use crate::state::{test_channel_id, InflightSync};
use helix_core::Correlation;

fn mk_corr() -> impl FnMut() -> Correlation {
    let mut n = 1u64;
    move || {
        let c = Correlation::from_raw(n);
        n += 1;
        c
    }
}

/// enqueue_and_drain 跳过不存在 channel（line 138 `None => continue`）+ 已在途 channel（line 136）。
#[test]
fn enqueue_and_drain_skips_missing_and_inflight_channels() {
    let mut state = ImState::new();
    let present = test_channel_id(1);
    let missing = test_channel_id(2);
    let inflight = test_channel_id(3);
    state.channels.insert(present, Channel::new(present, 0));
    let mut ch_inflight = Channel::new(inflight, 0);
    ch_inflight.inflight_sync = Some(InflightSync(Correlation::from_raw(99)));
    state.channels.insert(inflight, ch_inflight);

    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    // missing 不在 channels → 跳；inflight 已在途 → 跳；只有 present 入队 + dispatch。
    enqueue_and_drain(
        &mut state,
        "http://t",
        &[present, missing, inflight],
        &mut alloc,
        &mut out,
    );
    assert_eq!(state.sync_scheduler.inflight(), 1, "只 present 应占窗");
    assert_eq!(state.sync_scheduler.pending_len(), 0);
}

/// drain 队首在出队后被注销 → 跳过并归还窗口（line 164-166 `None => release+continue`）。
#[test]
fn drain_skips_and_releases_window_for_removed_channel() {
    let mut state = ImState::new();
    let ch = test_channel_id(1);
    // 直接 enqueue 一个不在 channels 的 channel（模拟入队后被注销）。
    state.sync_scheduler.enqueue(ch);
    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    drain(&mut state, "http://t", &mut alloc, &mut out);
    // 出队 → channels.get None → release_window 归还 → 在途归 0，无 Http。
    assert_eq!(
        state.sync_scheduler.inflight(),
        0,
        "removed channel 须归还窗口"
    );
    assert!(out.as_slice().is_empty(), "removed channel 不发 sync");
}

/// drain 队首出队后发现已另起在途 → 跳过并归还窗口（line 159-161）。
#[test]
fn drain_skips_and_releases_window_for_already_inflight_channel() {
    let mut state = ImState::new();
    let ch = test_channel_id(1);
    let mut c = Channel::new(ch, 0);
    c.inflight_sync = Some(InflightSync(Correlation::from_raw(7)));
    state.channels.insert(ch, c);
    state.sync_scheduler.enqueue(ch); // 直接入队（绕过 enqueue_and_drain 的在途前置过滤）。
    let mut out = EffectSink::new();
    let mut alloc = mk_corr();
    drain(&mut state, "http://t", &mut alloc, &mut out);
    assert_eq!(
        state.sync_scheduler.inflight(),
        0,
        "已在途 channel 须归还窗口"
    );
    assert!(out.as_slice().is_empty(), "已在途 channel 不重发 sync");
}

/// 标量 API：acquire_window / release_window 饱和、has_window、reset、enqueue 去重。
#[test]
fn scheduler_scalar_api_invariants() {
    let mut s = SyncScheduler::new();
    let ch = test_channel_id(1);
    assert!(s.has_window() && s.inflight() == 0 && s.pending_len() == 0);
    assert!(s.enqueue(ch), "首次入队 true");
    assert!(!s.enqueue(ch), "重复入队去重 false");
    assert_eq!(s.pending_len(), 1);
    s.acquire_window();
    assert_eq!(s.inflight(), 1);
    s.release_window();
    s.release_window(); // 饱和减不下溢
    assert_eq!(s.inflight(), 0);
    // 占满窗 → has_window false。
    for _ in 0..MAX_INFLIGHT_SYNC {
        s.acquire_window();
    }
    assert!(
        !s.has_window() && s.next_dispatch().is_none(),
        "满窗不再 dispatch"
    );
    s.reset();
    assert_eq!(s.inflight(), 0);
    assert_eq!(s.pending_len(), 0);
}

/// pong 节流 test-and-set：5s 内同 channel 拒补、超间隔重开、reset 清节流。
#[test]
fn pong_throttle_test_and_set_and_reset() {
    let mut s = SyncScheduler::new();
    let ch = test_channel_id(1);
    // 首次：允许（并记 now=1000）。
    assert!(s.should_pong_compensate(ch, 1_000), "首次补偿允许");
    // 4s 后（< 5s）：拒。
    assert!(
        !s.should_pong_compensate(ch, 5_000),
        "5s 内同 channel 节流拒绝"
    );
    // 恰 5s 边界（5000-1000==5000，不 < 5000）：允许。
    assert!(s.should_pong_compensate(ch, 6_000), "达 5s 间隔后重开");
    // reset 清节流戳 → 立即允许（会话边界）。
    s.reset();
    assert!(s.should_pong_compensate(ch, 6_100), "reset 后节流戳清空");
}

#[test]
fn explicit_gap_moves_to_front_and_upgrades_trigger() {
    let mut scheduler = SyncScheduler::new();
    let routine_a = test_channel_id(1);
    let explicit_gap = test_channel_id(2);
    let routine_b = test_channel_id(3);
    scheduler.enqueue_with_trigger(routine_a, SyncTrigger::Routine);
    scheduler.enqueue_with_trigger(explicit_gap, SyncTrigger::Routine);
    scheduler.enqueue_with_trigger(routine_b, SyncTrigger::Routine);

    scheduler.prioritize_with_trigger(&[explicit_gap], SyncTrigger::PongGap);

    assert_eq!(
        scheduler.next_dispatch(),
        Some((explicit_gap, SyncTrigger::PongGap))
    );
    assert_eq!(
        scheduler.next_dispatch(),
        Some((routine_a, SyncTrigger::Routine))
    );
    assert_eq!(
        scheduler.next_dispatch(),
        Some((routine_b, SyncTrigger::Routine))
    );
}
