//! E3 修复验收测试：自动续拉（真 Go wire 派生续拉）
//!
//! ## 续拉信号来源（B3 真 wire）
//!
//! 真 Go `events` SyncEntry **无显式续拉标志**——续拉与否由 parser 按「满批」派生
//! （`events.len() >= SYNC_EVENTS_PER_ENTRY(500)` → 还有更多 → 续拉），拉到不足上限 /
//! `no_change` 为止（B3 验收：离线>500→续拉至 cursor==服务端 maxSeq）。
//!
//! ## 验收场景
//!
//! 1500 事件分 3 满批（500×3）+ 收尾 no_change：
//! - 第一批 sync 回报（seq 1-500，满批 → 派生续拉）→ Persist{corr1}
//! - Persist ok → cursor=500 + 吐第二次 sync Http（续拉，fromSeq=500）
//! - 第二批 sync 回报（seq 501-1000，满批 → 派生续拉）→ Persist{corr2}
//! - Persist ok → cursor=1000 + 吐第三次 sync Http（续拉，fromSeq=1000）
//! - 第三批 sync 回报（seq 1001-1500，满批 → 仍派生续拉）→ Persist{corr3}
//! - Persist ok → cursor=1500 + 吐第四次 sync Http（续拉，fromSeq=1500）
//! - 第四批 sync 回报（no_change，已追平）→ 终止，cursor 仍=1500（==maxSeq）
//!
//! ## 终止保证
//!
//! - 服务端 no_change → 终止续拉（正常收尾）
//! - 若服务端返回 events 为空 → 不落库不续拉（防服务端 bug 无限循环）
//! - 若续拉时 cursor 未前进（dup 幂等），停止续拉

mod common;
use common::*;

use helix_core::effect::TransportId;
use helix_core::Tick;

// ─── 辅助：触发一次 gate sync（进入初始 sync 状态）──────────────────────────────

fn setup_initial_sync(h: &mut Harness) -> u64 {
    // 大缺口（cursor=0 / seq=99）→ **立即** backfill resync（根治 echo gate-gap·
    // fix/im-sync-cursor-catchup）：sync Http 在 ingest 当步即发出，不再等 1s gate timer fire。
    let gate_effects = h.step(inbound_event(1, 99, 1), 1000);
    gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill·不等 1s timer）")
}

// ─── 核心验收：3 批续拉，cursor 最终=1500，第 4 次不再发 ────────────────────────

#[test]
fn continuation_three_batches_then_stops() {
    let mut h = Harness::new(&[1]);
    let sync_corr1 = setup_initial_sync(&mut h);

    // 第一批：seq 1-500（满批 500 → 派生续拉）
    let batch1_seqs: Vec<u64> = (1..=500).collect();
    let reply1 = sync_reply_events(1, &batch1_seqs);
    let p1_effects = h.step(port_reply_with_body(sync_corr1, enveloped(reply1)), 3000);

    let persist_corr1 = p1_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("batch1 should emit Persist");

    // persist ok → cursor=500 + 续拉第二次 Http
    let after_p1_effects = h.step(port_reply_ok(persist_corr1), 3100);
    let sync_corr2 = after_p1_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("after batch1 persist ok should emit continuation Http (sync_corr2)");

    // cursor 应为 500
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(500),
        "cursor should be 500 after batch1"
    );

    // 第二批：seq 501-1000（满批 500 → 派生续拉）
    let batch2_seqs: Vec<u64> = (501..=1000).collect();
    let reply2 = sync_reply_events(1, &batch2_seqs);
    let p2_effects = h.step(port_reply_with_body(sync_corr2, enveloped(reply2)), 4000);

    let persist_corr2 = p2_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("batch2 should emit Persist");

    // persist ok → cursor=1000 + 续拉第三次 Http
    let after_p2_effects = h.step(port_reply_ok(persist_corr2), 4100);
    let sync_corr3 = after_p2_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("after batch2 persist ok should emit continuation Http (sync_corr3)");

    // cursor 应为 1000
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(1000),
        "cursor should be 1000 after batch2"
    );

    // 第三批：seq 1001-1500（满批 500 → 仍派生续拉，真 wire 靠 no_change 收尾）
    let batch3_seqs: Vec<u64> = (1001..=1500).collect();
    let reply3 = sync_reply_events(1, &batch3_seqs);
    let p3_effects = h.step(port_reply_with_body(sync_corr3, enveloped(reply3)), 5000);

    let persist_corr3 = p3_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("batch3 should emit Persist");

    // persist ok → cursor=1500 + 续拉第四次 Http（满批派生续拉，等服务端 no_change 收尾）
    let after_p3_effects = h.step(port_reply_ok(persist_corr3), 5100);

    // cursor 应为 1500
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(1500),
        "cursor should be 1500 after batch3"
    );

    let sync_corr4 = after_p3_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("满批 batch3 后应续拉第四次 Http（真 wire 无续拉标志，靠 no_change 收尾）");

    // 第四批：服务端 no_change（gap=0，已追平）→ 终止续拉，cursor 不变
    let reply4 = sync_reply_no_change(1, 1500);
    let after_p4_effects = h.step(port_reply_with_body(sync_corr4, enveloped(reply4)), 5200);

    // no_change：不落库、不续拉
    assert!(
        !after_p4_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "no_change should NOT emit Persist, got: {:?}",
        after_p4_effects
    );
    let http_count = after_p4_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 0,
        "after no_change, should NOT emit further Http (continuation done), got: {:?}",
        after_p4_effects
    );

    // cursor 仍为 1500（==服务端 maxSeq）
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(1500),
        "cursor should remain 1500 (== server maxSeq) after no_change"
    );
}

// ─── 终止保证：空 events + needs_continuation=true → 不续拉 ──────────────────────

#[test]
fn continuation_stops_on_empty_events_with_continuation_flag() {
    let mut h = Harness::new(&[1]);
    let sync_corr = setup_initial_sync(&mut h);

    // 服务端返回空 events（kind=events 但列表空，服务端 bug 场景）→ 防御性不落库不续拉
    let empty_reply = sync_reply_events(1, &[]);
    let effects = h.step(
        port_reply_with_body(sync_corr, enveloped(empty_reply)),
        3000,
    );

    // 不应发 Persist 也不应发续拉 Http
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "empty events + needs_continuation should NOT emit Http, got: {:?}",
        effects
    );
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Persist(_))),
        "empty events should NOT emit Persist, got: {:?}",
        effects
    );
}

// ─── 不并发自踩：inflight 守卫阻止重复续拉 ──────────────────────────────────────

#[test]
fn continuation_inflight_guard_prevents_concurrent_sync() {
    let mut h = Harness::new(&[1]);
    let sync_corr = setup_initial_sync(&mut h);

    // 第一批满批 500 → 派生续拉意图
    let batch1 = sync_reply_events(1, &(1..=500).collect::<Vec<_>>());
    let p1_effects = h.step(port_reply_with_body(sync_corr, enveloped(batch1)), 3000);
    let persist_corr1 = p1_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("batch1 Persist");

    // persist ok → 发出续拉 Http（inflight=Some）
    let continuation_effects = h.step(port_reply_ok(persist_corr1), 3100);
    let http_count = continuation_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(http_count, 1, "should emit exactly 1 continuation Http");

    // inflight 已置为 Some → 此时若再次触发 proactive resync（Connected 到来），
    // 同 channel 不应再发第二个 sync Http（gate_inflight 守卫）
    let reconnect_effects = h.step(Tick::Connected(TransportId::from_raw(0)), 3200);
    // channel=1 有 inflight，Connected 的 proactive resync 应跳过
    let reconnect_http = reconnect_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        reconnect_http, 0,
        "Connected while inflight sync active should skip channel (inflight guard), got: {:?}",
        reconnect_effects
    );
}

// ─── 续拉终止：第二次续拉时 cursor 未前进（全 dup 幂等丢弃）→ 停止 ────────────────

#[test]
fn continuation_aborts_if_cursor_not_advanced_on_second_continuation() {
    // 场景：第一批续拉正常（cursor 推进到 500），
    // 第二批服务端返回了 seq 1-500（全部重复，cursor 不能前进）→ 第三次续拉不发出
    let mut h = Harness::new(&[1]);
    let sync_corr = setup_initial_sync(&mut h);

    // 第一批：seq 1-500（满批 → 派生续拉）→ persist ok → 发出第二次 sync Http（fromSeq=500）
    let batch1 = sync_reply_events(1, &(1..=500).collect::<Vec<_>>());
    let p1_effects = h.step(port_reply_with_body(sync_corr, enveloped(batch1)), 3000);
    let persist_corr1 = p1_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("Persist corr1");
    let after_p1_effects = h.step(port_reply_ok(persist_corr1), 3100);
    let sync_corr2 = after_p1_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("continuation Http after batch1");
    // last_sync_from_seq 已置为 500

    // 第二批：seq 1-500（全 dup），满批 500 → 派生续拉意图
    // commit_target = 500，cursor.try_advance(500) → cursor 当前 500，500 <= 500 → 不推进
    let batch2_dup = sync_reply_events(1, &(1..=500).collect::<Vec<_>>());
    let p2_effects = h.step(
        port_reply_with_body(sync_corr2, enveloped(batch2_dup)),
        4000,
    );
    let persist_corr2 = p2_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("Persist corr2");

    // persist ok → cursor 仍 500（未前进）
    // maybe_continue_sync: cursor(500) <= last_sync_from_seq(500) → abort
    let after_p2_effects = h.step(port_reply_ok(persist_corr2), 4100);
    let http_count = after_p2_effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Http(_)))
        .count();
    assert_eq!(
        http_count, 0,
        "continuation should abort when cursor did not advance past prev fromSeq (all dup), got: {:?}", after_p2_effects
    );
}
