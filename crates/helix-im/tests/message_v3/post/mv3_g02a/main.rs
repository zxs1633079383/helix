use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::state::SendStatus;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    enveloped, port_reply_ok, port_reply_with_body, send_message_cmd, timer_tick, EffectKind,
    Harness,
};

/// 返回指定 effect 类别的 correlation。
fn correlation_of(effects: &[EffectKind], kind: &str) -> u64 {
    effects
        .iter()
        .find_map(|effect| match (kind, effect) {
            ("persist", EffectKind::Persist(corr)) | ("http", EffectKind::Http(corr)) => {
                Some(*corr)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {kind} correlation in {effects:?}"))
}

/// 构造携带 temporaryId 与 postId 的权威 WS echo。
fn echo_frame(sequence: u64, temporary_id: &str, post_id: &str) -> Tick {
    let frame = serde_json::json!({
        "action": "post",
        "data": {
            "id": post_id,
            "temporaryId": temporary_id,
            "channelId": common::cid(1),
            "event_seq": sequence,
            "userId": "user-actor-444",
            "type": "TEXT",
            "message": "验证发送三态",
            "createAt": 1_700_000_000_000_u64 + sequence
        }
    });
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&frame).expect("echo frame must serialize"),
    )))
}

/// 返回本批次首个原子提交 correlation（None = 该帧未进入原子提交，即被 gate 缓冲）。
fn atomic_correlation(effects: &[EffectKind]) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    })
}

/// 完成一次「发送 → 乐观落库回执 → posts/create 出站」，返回该 send 的 temporaryId。
fn arm_send(harness: &mut Harness, temporary_id: &'static str, now_ms: u64) -> &'static str {
    let initial = harness.step(send_message_cmd(1, temporary_id, "验证发送三态"), now_ms);
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        now_ms + 1,
    );
    temporary_id
}

/// 证明 MV3-G02a 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G02a", "post/mv3-g02a");
}

/// posts/create HTTP 只承认请求入站，任何响应形状都不得提前结算 sent。
#[test]
fn http_admission_never_replaces_the_ws_or_readback_authority() {
    let mut harness = Harness::new(&[1]);
    let initial = harness.step(
        send_message_cmd(1, "tmp-mv3-g02a-001", "验证发送三态"),
        1_700_000_000_000,
    );
    let persist_corr = correlation_of(&initial, "persist");
    let admitted = harness.step(port_reply_ok(persist_corr), 1_700_000_000_001);
    let http_corr = correlation_of(&admitted, "http");

    let synthetic_success = enveloped(Bytes::from_static(
        br#"{"status":"SUCCESS","data":{"id":"srvfix00000000000000000001"}}"#,
    ));
    let http_reply = harness.step(
        port_reply_with_body(http_corr, synthetic_success),
        1_700_000_000_002,
    );

    assert!(
        http_reply.is_empty(),
        "HTTP admission must not write terminal state or cancel the echo timer"
    );
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02a-001"),
        Some(SendStatus::Sending)
    );
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
}

/// echo 唯一结算 temporaryId；重复 echo 无效，超时后的迟到 echo 仍可恢复 sent。
#[test]
fn duplicate_and_late_echo_converge_to_one_terminal_send() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let initial = harness.step(
        send_message_cmd(1, "tmp-mv3-g02a-echo", "验证发送三态"),
        1_700_000_000_000,
    );
    let after_persist = harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    let timeout_timer = after_persist
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("posts/create must arm echo timeout");

    let first_echo = harness.step(
        echo_frame(1, "tmp-mv3-g02a-echo", "srvfix00000000000000000001"),
        1_700_000_000_002,
    );
    assert!(first_echo
        .iter()
        .any(|effect| matches!(effect, EffectKind::CancelTimer(timer) if *timer == timeout_timer)));
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02a-echo"),
        None
    );
    let atomic_corr = first_echo
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("echo authority must enter atomic commit");
    harness.step(port_reply_ok(atomic_corr), 1_700_000_000_003);

    let duplicate = harness.step(
        echo_frame(1, "tmp-mv3-g02a-echo", "srvfix00000000000000000001"),
        1_700_000_000_004,
    );
    assert!(duplicate.is_empty());

    let late_initial = harness.step(
        send_message_cmd(1, "tmp-mv3-g02a-late", "验证迟到 echo"),
        1_700_000_000_005,
    );
    let late_after_persist = harness.step(
        port_reply_ok(correlation_of(&late_initial, "persist")),
        1_700_000_000_006,
    );
    let late_timer = late_after_persist
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("late send must arm echo timeout");
    harness.step(timer_tick(late_timer), 1_700_000_015_006);
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02a-late"),
        Some(SendStatus::UnSend)
    );

    harness.step(
        echo_frame(2, "tmp-mv3-g02a-late", "srvfix00000000000000000002"),
        1_700_000_015_007,
    );
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02a-late"),
        None,
        "late authority must settle the same optimistic identity"
    );
}

/// 乱序 echo：超前 seq 先到不得跨越缺口结算，已结算的发送保持终态，缺口补齐后才连续提交。
///
/// 覆盖 T045 的「乱序」一路（此前只测了重复与迟到）。断言锚在**频道提交**而非发送态：
/// 按 R4-echo-gate，echo 对账与 channel gate 解耦（reconcile 只读 pending_sends，冷启动
/// 乱序下必须无条件先做），真正被乱序门控的是 cursor 推进与原子提交。
#[test]
fn out_of_order_echo_defers_commit_until_the_missing_sequence_arrives() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let channel = common::channel_id(1);

    // ① seq 1 顺序到达并提交 → 该发送进入终态，cursor = 1。
    let settled = arm_send(&mut harness, "tmp-mv3-g02a-ooo-1", 1_700_000_000_000);
    let first_echo = harness.step(echo_frame(1, settled, &common::sid(1)), 1_700_000_000_002);
    let first_atomic = atomic_correlation(&first_echo).expect("in-order echo must commit");
    harness.step(port_reply_ok(first_atomic), 1_700_000_000_003);
    assert_eq!(harness.module.cursor_for(channel), Some(1));
    assert_eq!(harness.module.pending_send_status(settled), None);

    // ② 两条在途发送；seq 3 的 echo 先于 seq 2 到达（缺口）。
    let gapped = arm_send(&mut harness, "tmp-mv3-g02a-ooo-3", 1_700_000_000_004);
    let missing = arm_send(&mut harness, "tmp-mv3-g02a-ooo-2", 1_700_000_000_006);

    let ahead = harness.step(echo_frame(3, gapped, &common::sid(3)), 1_700_000_000_008);
    assert_eq!(
        atomic_correlation(&ahead),
        None,
        "gapped echo must be buffered, never committed across the hole"
    );
    assert_eq!(
        harness.module.cursor_for(channel),
        Some(1),
        "cursor 不得跨空洞前进（否则缺失的 seq 2 永不回拉）"
    );
    assert_eq!(
        harness.module.pending_send_status(settled),
        None,
        "已结算的发送不得因后到的乱序 echo 退回非终态"
    );

    // ③ 缺口补齐：seq 2 到达并提交后，缓冲的 seq 3 才连续 flush 进原子提交。
    let filled = harness.step(echo_frame(2, missing, &common::sid(2)), 1_700_000_000_009);
    let second_atomic = atomic_correlation(&filled).expect("contiguous echo must commit");
    let flushed = harness.step(port_reply_ok(second_atomic), 1_700_000_000_010);
    assert_eq!(harness.module.cursor_for(channel), Some(2));

    let third_atomic =
        atomic_correlation(&flushed).expect("buffered seq 3 must flush once the hole is filled");
    harness.step(port_reply_ok(third_atomic), 1_700_000_000_011);
    assert_eq!(
        harness.module.cursor_for(channel),
        Some(3),
        "缺口补齐后乱序 echo 才结算到权威水位"
    );
    assert_eq!(harness.module.pending_send_status(gapped), None);
}
