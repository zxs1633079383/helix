//! 心跳 pong 逾期（ack deadline）回归。
//!
//! 事故背景：锁屏/休眠恢复后 WS 处于半开状态——ping 能写出，但 `increment_channel_end`
//! 被 Go 推到已死连接上丢失，客户端既收不到业务帧也收不到 pong；旧实现只能等 OS 给出 RST
//! （实测 15–33s）才发现连接已死。本回归锁定新增的 pong 逾期判定：
//!   ① ping 发出后无 ACK → 逾期 timer 到期即置 Connecting 并请求重建 transport（emit 控制信号）；
//!   ② pong（含无 gaps 的纯 ACK）到达 → 取消逾期 timer，不误判、不重连；
//!   ③ 已判定逾期后迟到的 pong 不复活连接，也不重复发信号；
//!   ④ 重建后的 hello 必须让心跳与新逾期 timer 重新工作；
//!   ⑤ stop 必须取消未到期的逾期 timer。

mod common;

use common::*;
use helix_core::tick::InboundBytes;
use helix_core::{Module, Tick};
use serde_json::json;

/// 构造心跳 pong ACK 帧；`with_gaps=false` 表示纯 ACK（无可补偿信息）。
fn pong_inbound(with_gaps: bool) -> Tick {
    let payload = if with_gaps {
        json!({ "status": "OK", "data": { "gaps": [] } })
    } else {
        json!({ "status": "OK", "data": {} })
    };
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&payload)
            .expect("pong fixture must serialize")
            .into(),
    ))
}

fn scheduled_timers(kinds: &[EffectKind]) -> Vec<u64> {
    kinds
        .iter()
        .filter_map(|kind| match kind {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// 一轮心跳的实测结果：ping timer 每轮自续期换新 id，因此必须一次触发出全部可断言事实。
struct Heartbeat {
    harness: Harness,
    /// pong 逾期 timer（本轮 ping 之后的第一个 ScheduleTimer）。
    deadline_timer: u64,
    /// 下一轮 ping timer（自续期产生）。
    next_ping_timer: u64,
}

/// 进入「已连接 + 已发出一次心跳」的可断言状态。
fn heartbeat_after_hello() -> Heartbeat {
    let mut harness = Harness::new(&[]);
    let started = harness.start(0);
    harness.step(hello_inbound("conn-1"), 0);

    // on_start 会臂若干 timer；只有会发 ping 帧的那个才是心跳 timer。
    for id in scheduled_timers(&started) {
        let batch = harness.step(timer_tick(id), 8_000);
        if !batch.contains(&EffectKind::Send) {
            continue;
        }
        let timers = scheduled_timers(&batch);
        assert!(
            timers.len() >= 2,
            "ping 之后必须同时臂 pong 逾期 timer 与下一轮 ping timer，实际={batch:?}"
        );
        return Heartbeat {
            harness,
            deadline_timer: timers[0],
            next_ping_timer: timers[1],
        };
    }
    panic!("on_start 必须臂一个会发 ping 的心跳 timer");
}

#[test]
fn ping_arms_deadline_and_pong_cancels_it_without_reconnect() {
    let mut heartbeat = heartbeat_after_hello();
    let deadline = heartbeat.deadline_timer;

    // 纯 ACK（无 gaps）同样必须取消逾期 timer：否则会产生假超时与无谓重连。
    let ack = heartbeat.harness.step(pong_inbound(false), 8_500);
    assert!(
        ack.contains(&EffectKind::CancelTimer(deadline)),
        "pong 到达必须取消逾期 timer，实际={ack:?}"
    );
    assert!(
        !ack.contains(&EffectKind::Emit),
        "存活连接不得请求重建 transport"
    );

    // 已取消的逾期 timer 事后被喂进来必须是 no-op。
    let late = heartbeat.harness.step(timer_tick(deadline), 9_000);
    assert!(
        !late.contains(&EffectKind::Emit),
        "已取消的逾期 timer 不得触发重建，实际={late:?}"
    );
}

#[test]
fn missing_pong_emits_reconnect_and_stops_pinging() {
    let mut heartbeat = heartbeat_after_hello();
    let deadline = heartbeat.deadline_timer;
    let next_ping = heartbeat.next_ping_timer;

    let expired = heartbeat.harness.step(timer_tick(deadline), 12_000);
    assert!(
        expired.contains(&EffectKind::Emit),
        "pong 逾期必须 emit im:net:reconnect_requested，实际={expired:?}"
    );

    // 判定后进入 Connecting：下一轮心跳不得继续对死连接写入 ping。
    let next = heartbeat.harness.step(timer_tick(next_ping), 16_000);
    assert!(
        !next.contains(&EffectKind::Send),
        "半开连接判定后不得继续发 ping，实际={next:?}"
    );
}

#[test]
fn late_pong_after_deadline_does_not_resurrect_connection() {
    let mut heartbeat = heartbeat_after_hello();
    let deadline = heartbeat.deadline_timer;
    let next_ping = heartbeat.next_ping_timer;

    assert!(heartbeat
        .harness
        .step(timer_tick(deadline), 12_000)
        .contains(&EffectKind::Emit));
    let late_ack = heartbeat.harness.step(pong_inbound(true), 12_500);
    assert!(
        !late_ack.contains(&EffectKind::Emit),
        "迟到 pong 不得产生第二次重建信号"
    );
    let next = heartbeat.harness.step(timer_tick(next_ping), 16_000);
    assert!(
        !next.contains(&EffectKind::Send),
        "迟到 pong 不得把连接恢复成 Connected，实际={next:?}"
    );
}

#[test]
fn hello_reestablishes_heartbeat_after_timeout() {
    let mut heartbeat = heartbeat_after_hello();
    let deadline = heartbeat.deadline_timer;
    let next_ping = heartbeat.next_ping_timer;

    assert!(heartbeat
        .harness
        .step(timer_tick(deadline), 12_000)
        .contains(&EffectKind::Emit));

    // 重建后服务端重新下发 hello：心跳必须恢复，且新 pong 正常取消新的逾期 timer。
    heartbeat.harness.step(hello_inbound("conn-2"), 13_000);
    let resumed = heartbeat.harness.step(timer_tick(next_ping), 16_000);
    assert!(
        resumed.contains(&EffectKind::Send),
        "hello 之后心跳必须恢复，实际={resumed:?}"
    );
    let timers = scheduled_timers(&resumed);
    assert!(timers.len() >= 2, "恢复后的 ping 同样要臂逾期 timer");
    let new_deadline = timers[0];
    let ack = heartbeat.harness.step(pong_inbound(false), 16_500);
    assert!(ack.contains(&EffectKind::CancelTimer(new_deadline)));
    assert!(!ack.contains(&EffectKind::Emit));
}

#[test]
fn stop_lifecycle_cancels_pending_deadline_timer() {
    let mut heartbeat = heartbeat_after_hello();
    let deadline = heartbeat.deadline_timer;

    heartbeat.harness.sink.clear();
    heartbeat
        .harness
        .module
        .on_stop(&mut heartbeat.harness.sink)
        .expect("stop 必须成功");
    let stopped = heartbeat
        .harness
        .sink
        .as_slice()
        .iter()
        .map(EffectKind::from_effect)
        .collect::<Vec<_>>();
    assert!(
        stopped.contains(&EffectKind::CancelTimer(deadline)),
        "stop 必须取消未到期的逾期 timer，实际={stopped:?}"
    );
}
