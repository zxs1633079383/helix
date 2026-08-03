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

/// 执行一个 Tick，并逐个确认其产生的 MessageV3 原子写屏障。
fn step_durable(harness: &mut Harness, tick: Tick, now_ms: u64) -> Vec<EffectKind> {
    let mut effects = harness.step(tick, now_ms);
    let mut pending: Vec<u64> = effects
        .iter()
        .filter_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .collect();
    let mut offset = 1_u64;
    while let Some(corr) = pending.pop() {
        let next = harness.step(port_reply_ok(corr), now_ms.saturating_add(offset));
        pending.extend(next.iter().filter_map(|effect| match effect {
            EffectKind::PersistAtomic(next_corr) => Some(*next_corr),
            _ => None,
        }));
        effects.extend(next);
        offset = offset.saturating_add(1);
    }
    effects
}

// ─── WS 路径：连续帧直接放行 ──────────────────────────────────────────────────

#[test]
fn gate_contiguous_ws_applies_immediately() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // cursor = 0，喂 seq=1（连续）
    let effects = step_durable(&mut h, inbound_event(1, 1, 1), 1000);
    // MessageV3 连续帧先产原子写，写成功后才推进内存 cursor。
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistAtomic(_))),
        "contiguous WS event should emit PersistAtomic, got: {:?}",
        effects
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(1));
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
    step_durable(&mut h, inbound_event(1, 1, 1), 1000);
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
    // arm gate by sending **小缺口**（seq=2 / cursor=0 / gap=2 ≤ 阈值）→ 不触发立即 backfill resync
    // （根治 fix/im-sync-cursor-catchup 仅对 gap>2 的大缺口立即发 sync），保留 1s gate timer 作为
    // 缺口兜底触发点——本测专验「小缺口走 1s gate timer fire → sync/notify」这条路径仍成立。
    let gate_effects = h.step(inbound_event(1, 2, 1), 1000);
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
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // 大缺口（seq=3 / cursor=0 / gap=3 > 阈值）→ **立即** backfill resync（根治
    // fix/im-sync-cursor-catchup）：sync Http 在 ingest 当步即发出，不等 1s gate timer。
    let gate_effects = h.step(inbound_event(1, 3, 1), 1000);
    let sync_corr = gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill）");

    // driver 回报 sync/notify 成功，带回 3 个事件（seq 1,2,3）——真 Go wire SyncEntry
    let reply_bytes = ReplyBytes(enveloped(sync_reply_events(1, &[1, 2, 3])));
    let port_reply = Tick::PortReply {
        corr: Correlation::from_raw(sync_corr),
        outcome: PortOutcome::Ok(reply_bytes),
    };

    // 第一个 PortReply：sync 解析 → 吐 Persist{corr, batch_upsert}
    let peek_effects = h.step(port_reply, 2100);
    let persist_corr = peek_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::PersistAtomic(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("should emit Persist after sync success, meaning peek phase triggered batch write");

    // 第二个 PortReply（原子写成功）→ commit cursor；cursor op 已在同一批中。
    let commit_effects = h.step(port_reply_ok(persist_corr), 2200);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(3));
    assert!(commit_effects.iter().any(|e| matches!(e, EffectKind::Emit)));
}

// ─── cursor 锚 nextSeq（权威水位）而非 max(event.seq)——防尾部稀疏 cursor 落后 ──────
//
// 回归锚: B3 nextSeq 加固。服务端权威 nextSeq=50，但本页只下发到 seq=10
//（尾部 11..=50 是 phantom / 稀疏空洞，不进 events 数组）。cursor 必须跳到 50，
// 否则停在 10 → 下次 sync from=10 → 服务端又算 gap=40 → 稀疏 sync 风暴。
#[test]
fn sync_cursor_anchors_to_next_seq_not_max_event() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // 大缺口（喂乱序 seq=99 → buffer + 立即 backfill resync·根治 fix/im-sync-cursor-catchup）：
    // sync Http 在 ingest 当步即发出（不等 1s gate timer）。
    let gate_effects = h.step(inbound_event(1, 99, 1), 1000);
    let sync_corr = gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill）");

    // 下发 events seq 1..=10，但权威 nextSeq=50（尾部 11..=50 稀疏，不在 events 数组）
    let reply = sync_reply_events_with_next(1, &(1..=10).collect::<Vec<_>>(), 50);
    let peek_effects = h.step(port_reply_with_body(sync_corr, enveloped(reply)), 2100);
    let persist_corr = peek_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::PersistAtomic(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("should emit Persist");

    // persist ok → cursor 应跳到权威 nextSeq=50（不是 max(event.seq)=10）
    h.step(port_reply_ok(persist_corr), 2200);
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(50),
        "cursor must anchor to authoritative nextSeq=50, NOT max(event.seq)=10"
    );
}

// ─── too_long → 覆盖窗口原子提交 + Emit ─────────────────────────────────────────

#[test]
fn sync_too_long_resets_cursor_and_emits() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // 大缺口（seq=3 / gap=3 > 阈值）→ **立即** backfill resync（根治 fix/im-sync-cursor-catchup）。
    let gate_effects = h.step(inbound_event(1, 3, 1), 1000);
    let sync_corr = gate_effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill）");

    // 服务端返回 too_long（真 Go wire SyncEntry，resetTo=100）
    let reply_bytes = ReplyBytes(enveloped(sync_reply_too_long(1, 100)));
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
    // terminal control event 只能提交一次；旧 cursor 与本地窗口必须在成功前保留。
    let too_long_emits = h
        .sink
        .as_slice()
        .iter()
        .filter(|effect| {
            matches!(effect, helix_core::Effect::Emit { event }
                if String::from_utf8_lossy(event.0.as_ref()).contains("im:sync:too_long"))
        })
        .count();
    assert_eq!(
        too_long_emits, 1,
        "too_long terminal emit must be single-shot"
    );
    let reload_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            helix_core::Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("too_long should start authoritative latest-post reload");
    h.step(port_reply_err(reload_corr), 2200);
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(0),
        "reload failure must preserve the old cursor and local message window"
    );

    // 重拉成功后才由 PersistAtomic 的 receipt 一次性提交 resetTo；空权威窗口也要推进 cursor。
    let mut success = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let sync_corr = success
        .step(inbound_event(1, 3, 1), 3000)
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(*corr),
            _ => None,
        })
        .expect("large gap must trigger sync Http");
    success.step(
        Tick::PortReply {
            corr: Correlation::from_raw(sync_corr),
            outcome: PortOutcome::Ok(ReplyBytes(enveloped(sync_reply_too_long(1, 100)))),
        },
        3100,
    );
    let reload_corr = success
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            helix_core::Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("too_long should start authoritative latest-post reload");
    let persist_corr = success
        .step(
            port_reply_with_body(
                reload_corr,
                enveloped(bytes::Bytes::from_static(
                    br#"{"status":"SUCCESS","data":{"posts":[]}}"#,
                )),
            ),
            3200,
        )
        .into_iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(corr),
            _ => None,
        })
        .expect("successful reload must emit PersistAtomic");
    success.step(port_reply_ok(persist_corr), 3300);
    assert_eq!(
        success.module.cursor_for(channel_id(1)),
        Some(100),
        "successful authoritative window must commit resetTo atomically"
    );
}

#[test]
fn proactive_too_long_requests_latest_post_reload_e6() {
    let mut h = Harness::new(&[7]);
    let mut saw_too_long = false;
    let mut saw_reload = false;

    for seq in 2..(16_384_u64 + 2 + 128) {
        let effects = h.step(inbound_event(7, seq, 1), 1_000 + seq);
        let step_saw_too_long = h.sink.as_slice().iter().any(|effect| {
            matches!(effect, helix_core::Effect::Emit { event }
                if String::from_utf8_lossy(event.0.as_ref()).contains("im:sync:too_long"))
        });
        if !step_saw_too_long {
            continue;
        }
        saw_too_long = true;
        saw_reload = h.sink.as_slice().iter().any(|effect| {
            let helix_core::Effect::Http { req, .. } = effect else {
                return false;
            };
            if !req.url.contains("posts/getLatestPost") {
                return false;
            }
            let Some(body) = req.body.as_ref() else {
                return false;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(body.as_ref()) else {
                return false;
            };
            value["channelId"] == cid(7)
        });
        assert!(
            effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
            "proactive too_long 当前 step 必须追加 getLatestPost reload Http，got {:?}",
            effects
        );
        break;
    }

    assert!(saw_too_long, "应命中 proactive too_long");
    assert!(
        saw_reload,
        "proactive too_long 后必须请求 channel 7 的 getLatestPost"
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
    let effects = step_durable(&mut h, inbound_event(1, 1, 1), 1002);

    // 每条连续消息都必须独立经过原子写屏障。
    let persist_atomic_count = effects
        .iter()
        .filter(|e| matches!(e, EffectKind::PersistAtomic(_)))
        .count();
    assert!(
        persist_atomic_count >= 3,
        "should flush buffer after contiguous seq arrives (got {} PersistAtomic)",
        persist_atomic_count
    );

    // cursor 最终应为 3
    let cursor = h.module.cursor_for(channel_id(1)).unwrap_or(0);
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
        step_durable(&mut h, inbound_event(1, seq, 1), seq * 1000);
    }
    let cursor_after_5 = h.module.cursor_for(channel_id(1)).unwrap_or(0);
    assert_eq!(cursor_after_5, 5, "cursor should be 5 after contiguous 1-5");

    // 再喂 seq=3（dup，seq < cursor）→ cursor 不变
    h.step(inbound_event(1, 3, 1), 6000);
    let cursor_after_dup = h.module.cursor_for(channel_id(1)).unwrap_or(0);
    assert_eq!(cursor_after_dup, 5, "cursor should not decrease on dup");

    // 喂旧 PortReply（stale，已无对应 pending_commit）→ cursor 不变
    h.step(port_reply_ok(99999), 7000);
    let cursor_after_stale = h.module.cursor_for(channel_id(1)).unwrap_or(0);
    assert_eq!(
        cursor_after_stale, 5,
        "cursor should not decrease on stale PortReply"
    );
}

// ─── 根治回归：大缺口 echo → 立即 backfill → cursor 追平 → buffered echo flush emit ──────
//
// 回归锚: fix/im-sync-cursor-catchup。根因——活动频道本地 cursor 落后服务端 high-water
// （cursor=2 / echo seq=5·gap=3），echo 被 gate buffer 且**不推 cursor**；旧路仅 1s gate timer
// 超时后才发 sync 追平（慢于 e2e 静默/认领窗口 → buffered echo 永不在窗口内 emit·UC-1.7/5.6w 红）。
// 根治：大缺口 ingest **当步**即触发 per-channel proactive resync → sync 回拉中间缺口（seq 3,4）
// + cursor 经 on_persist_ok commit 追平 → buffered echo（seq 5）自然 flush emit im:post:received。
// 守 HX-C008：cursor 仍由 sync commit **严格推进**，不绕 gate / 不破排序（缺口真填上，非放行）。
#[test]
fn large_gap_echo_triggers_immediate_backfill_then_flushes_buffered_echo() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // 前置：连续 seq 1,2 → cursor=2。
    step_durable(&mut h, inbound_event(1, 1, 1), 1000);
    step_durable(&mut h, inbound_event(1, 2, 1), 1001);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(2), "前置 cursor=2");

    // 大缺口 echo（seq=5 / gap=3 > 阈值）→ buffer（不 emit·不推 cursor）+ **立即** backfill sync。
    let echo = h.step(inbound_event(1, 5, 1), 1002);
    let sync_corr = echo
        .iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("大缺口 echo ingest 当步即发 backfill sync Http（立即·不等 1s timer）");
    assert!(
        !echo.iter().any(|e| matches!(e, EffectKind::Emit)),
        "大缺口 echo 当步不得 emit（仍 buffer·守 gate 排序），got {echo:?}"
    );
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(2),
        "echo 仅 buffer·不推 cursor（HX-C008·cursor 由 sync commit 推进）"
    );

    // 防风暴：同 channel 在途 sync 期间再来一帧大缺口 echo（seq=8）→ inflight_sync 守卫 → **不**重发 sync。
    let echo2 = h.step(inbound_event(1, 8, 1), 1003);
    assert!(
        !echo2.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "在途 sync 期间同 channel 不得重复发 sync（防风暴·inflight_sync 守卫），got {echo2:?}"
    );

    // 服务端 backfill 回报：中间缺口 seq 3,4（可见），nextSeq=4。
    let reply = sync_reply_events_with_messages(
        1,
        &[ev(3, 1, "msg_3"), ev(4, 1, "msg_4")],
        &[("msg_3", "a", ""), ("msg_4", "b", "")],
        4,
    );
    let p = h.step(port_reply_with_body(sync_corr, enveloped(reply)), 2000);
    let persist_corr = p
        .iter()
        .find_map(|e| match e {
            EffectKind::PersistAtomic(c) => Some(*c),
            _ => None,
        })
        .expect("backfill 回报产 Persist");

    // backfill 写成功先提交 cursor=4，再为刚变连续的 seq=5 排队 MessageV3 原子写。
    let buffered = h.step(port_reply_ok(persist_corr), 2100);
    let buffered_corr = buffered.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    let Some(buffered_corr) = buffered_corr else {
        panic!("buffered seq=5 必须进入 MessageV3 PersistAtomic");
    };
    h.step(port_reply_ok(buffered_corr), 2101);
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(5),
        "cursor 经 backfill commit(4) + buffered echo durable commit(5) 追平到 5"
    );
    assert!(
        h.sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, helix_core::Effect::Persist { .. })),
        "buffered seq=5 持久成功后必须请求 viewer channel 绝对态读回"
    );
}
