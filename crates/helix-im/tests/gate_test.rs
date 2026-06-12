//! gate 状态机测试（Phase B Commit 2）
//!
//! 验证 §4.6 核心机制：
//! - WS 路径连续帧直接放行（PersistFire），无 gate arm
//! - 断号 → buffer + arm 1s gate timer
//! - gate 触发 → Http{sync/notify}（不重复 arm）
//! - sync 成功 → Persist{corr, batch_upsert} → PortReply Ok → cursor 推进 + Emit
//! - too_long → cursor 复位 + Emit(im:sync:too_long)
//! - dup drop：seq <= cursor 的帧静默忽略

mod common;
use common::*;

use helix_core::effect::Correlation;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Tick;

// ─── WS 路径：连续帧直接放行 ──────────────────────────────────────────────────

#[test]
fn gate_contiguous_ws_applies_immediately() {
    let mut h = Harness::new(&[1]);
    // cursor = 0，喂 seq=1（连续）
    let effects = h.step(inbound_event(1, 1, 1), 1000);
    // 应产出：PersistFire（落库）+ PersistFire（advance_cursor）+ Emit
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::PersistFire)),
        "contiguous WS event should emit PersistFire, got: {:?}",
        effects
    );
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "contiguous WS event should emit domain event, got: {:?}",
        effects
    );
    // 不应 arm gate timer（连续不进 buffer）
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "contiguous WS event should NOT arm gate timer, got: {:?}",
        effects
    );
}

// ─── 断号 → buffer + arm gate ─────────────────────────────────────────────────

#[test]
fn gate_out_of_order_arms_timer() {
    let mut h = Harness::new(&[1]);
    // cursor = 0，喂 seq=3（断号：缺 1, 2）
    let effects = h.step(inbound_event(1, 3, 1), 1000);
    // 应产出 ScheduleTimer（1s gate）
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "out-of-order event should arm gate timer, got: {:?}",
        effects
    );
    // 不应立即产出 PersistFire 或 Emit（等 sync 补齐后才放行）
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::PersistFire)),
        "out-of-order event should NOT immediately PersistFire, got: {:?}",
        effects
    );
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "out-of-order event should NOT immediately Emit, got: {:?}",
        effects
    );
}

#[test]
fn gate_dup_drop_silently() {
    let mut h = Harness::new(&[1]);
    // 先让 seq=1 通过（cursor→1）
    h.step(inbound_event(1, 1, 1), 1000);
    // 再喂 seq=1（dup，seq <= cursor）
    let effects = h.step(inbound_event(1, 1, 1), 1001);
    // 应该完全静默（无 PersistFire, 无 Emit, 无 ScheduleTimer）
    assert!(
        effects.is_empty(),
        "dup event (seq <= cursor) should be silently dropped, got: {:?}",
        effects
    );
}

#[test]
fn gate_second_out_of_order_no_double_arm() {
    let mut h = Harness::new(&[1]);
    // 喂 seq=3（断号，arm gate）
    let effects1 = h.step(inbound_event(1, 3, 1), 1000);
    assert!(effects1
        .iter()
        .any(|e| matches!(e, EffectKind::ScheduleTimer(_))));

    // 再喂 seq=4（同样断号，gate 已 armed，不重复 arm）
    let effects2 = h.step(inbound_event(1, 4, 1), 1001);
    assert!(
        !effects2
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "second out-of-order event should NOT re-arm gate, got: {:?}",
        effects2
    );
}

// ─── gate 触发 → sync/notify ──────────────────────────────────────────────────

#[test]
fn gate_timer_fires_sync_http() {
    let mut h = Harness::new(&[1]);
    // arm gate by sending out-of-order
    let gate_effects = h.step(inbound_event(1, 3, 1), 1000);
    let gate_timer_id = gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("should have gate timer");

    // fire gate timer
    let sync_effects = h.step(timer_tick(gate_timer_id), 2001);
    // 应产出 Http（sync/notify）
    assert!(
        sync_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Http(_))),
        "gate timer should trigger Http sync/notify, got: {:?}",
        sync_effects
    );
    // 不应重新 arm gate（已 clear）
    assert!(
        !sync_effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "gate timer firing should NOT re-arm gate, got: {:?}",
        sync_effects
    );
}

// ─── sync 成功 → peek/commit 三阶段 ──────────────────────────────────────────

#[test]
fn sync_success_path_peek_then_commit_cursor() {
    let mut h = Harness::new(&[1]);
    // arm gate
    let gate_effects = h.step(inbound_event(1, 3, 1), 1000);
    let gate_timer_id = gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("gate timer");

    // fire gate → Http
    let sync_effects = h.step(timer_tick(gate_timer_id), 2001);
    let sync_corr = sync_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("sync Http corr");

    // driver 回报 sync/notify 成功，带回 3 个事件（seq 1,2,3）
    let reply_body = serde_json::json!({
        "type": "events",
        "events": [
            {"channel_id": 1, "seq": 1, "event_type": 1},
            {"channel_id": 1, "seq": 2, "event_type": 1},
            {"channel_id": 1, "seq": 3, "event_type": 1},
        ],
        "needs_continuation": false
    });
    let reply_bytes = ReplyBytes(bytes::Bytes::from(serde_json::to_vec(&reply_body).unwrap()));
    let port_reply = Tick::PortReply {
        corr: Correlation::from_raw(sync_corr),
        outcome: PortOutcome::Ok(reply_bytes),
    };

    // 第一个 PortReply：sync 解析 → 吐 Persist{corr, batch_upsert}
    let peek_effects = h.step(port_reply, 2100);
    let persist_corr = peek_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("should emit Persist after sync success, meaning peek phase triggered batch write");

    // 第二个 PortReply（写库成功）→ commit cursor + PersistFire advance_cursor
    let commit_effects = h.step(port_reply_ok(persist_corr), 2200);
    // 应产出 PersistFire（advance_cursor）
    assert!(
        commit_effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistFire)),
        "persist ok should advance cursor (PersistFire), got: {:?}",
        commit_effects
    );
}

// ─── too_long → cursor 复位 + Emit ─────────────────────────────────────────────

#[test]
fn sync_too_long_resets_cursor_and_emits() {
    let mut h = Harness::new(&[1]);
    // arm gate
    let gate_effects = h.step(inbound_event(1, 3, 1), 1000);
    let gate_timer_id = gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("gate timer");
    let sync_effects = h.step(timer_tick(gate_timer_id), 2001);
    let sync_corr = sync_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("sync Http corr");

    // 服务端返回 too_long
    let reply_body = serde_json::json!({ "type": "too_long", "reset_to": 100 });
    let reply_bytes = ReplyBytes(bytes::Bytes::from(serde_json::to_vec(&reply_body).unwrap()));
    let port_reply = Tick::PortReply {
        corr: Correlation::from_raw(sync_corr),
        outcome: PortOutcome::Ok(reply_bytes),
    };

    let effects = h.step(port_reply, 2100);
    // 应产出 Emit（im:sync:too_long）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "too_long should emit domain event, got: {:?}",
        effects
    );
    // cursor 应已复位（reset_to-1 = 99）
    let cursor = h
        .module
        .cursor_for(helix_im::state::ChannelId(1))
        .unwrap_or(0);
    assert_eq!(
        cursor, 99,
        "cursor should be reset to reset_to-1 = 99, got: {}",
        cursor
    );
}

// ─── 多帧背压：buffer → flush 后连续 ─────────────────────────────────────────

#[test]
fn gate_buffer_flush_on_contiguous_arrival() {
    let mut h = Harness::new(&[1]);
    // 喂 seq=3（断号，buffer 进去，arm gate）
    h.step(inbound_event(1, 3, 1), 1000);
    // 喂 seq=2（断号，进 buffer，不重复 arm）
    h.step(inbound_event(1, 2, 1), 1001);
    // 喂 seq=1（连续！cursor=0+1=1）→ apply + 从 buffer flush 2,3
    let effects = h.step(inbound_event(1, 1, 1), 1002);

    // 应产出多个 PersistFire
    let persist_fire_count = effects
        .iter()
        .filter(|e| matches!(e, EffectKind::PersistFire))
        .count();
    // seq 1: PersistFire(upsert) + PersistFire(cursor)
    // seq 2: PersistFire(upsert) + PersistFire(cursor) [from flush]
    // seq 3: PersistFire(upsert) + PersistFire(cursor) [from flush]
    assert!(
        persist_fire_count >= 4,
        "should flush buffer after contiguous seq arrives (got {} PersistFire)",
        persist_fire_count
    );

    // cursor 最终应为 3
    let cursor = h
        .module
        .cursor_for(helix_im::state::ChannelId(1))
        .unwrap_or(0);
    assert_eq!(
        cursor, 3,
        "cursor should be 3 after flushing seq 1,2,3, got: {}",
        cursor
    );
}

// ─── cursor 不变量：无论顺序如何，cursor 永不回退 ─────────────────────────────

#[test]
fn cursor_never_decreases_under_any_input() {
    let mut h = Harness::new(&[1]);
    // 先喂 seq 1..5 连续（cursor=5）
    for seq in 1u64..=5 {
        h.step(inbound_event(1, seq, 1), seq * 1000);
    }
    let cursor_after_5 = h
        .module
        .cursor_for(helix_im::state::ChannelId(1))
        .unwrap_or(0);
    assert_eq!(cursor_after_5, 5, "cursor should be 5 after contiguous 1-5");

    // 再喂 seq=3（dup，seq < cursor）→ cursor 不变
    h.step(inbound_event(1, 3, 1), 6000);
    let cursor_after_dup = h
        .module
        .cursor_for(helix_im::state::ChannelId(1))
        .unwrap_or(0);
    assert_eq!(cursor_after_dup, 5, "cursor should not decrease on dup");

    // 喂旧 PortReply（stale，已无对应 pending_commit）→ cursor 不变
    h.step(port_reply_ok(99999), 7000);
    let cursor_after_stale = h
        .module
        .cursor_for(helix_im::state::ChannelId(1))
        .unwrap_or(0);
    assert_eq!(
        cursor_after_stale, 5,
        "cursor should not decrease on stale PortReply"
    );
}
