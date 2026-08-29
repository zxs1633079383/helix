use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::state::SendStatus;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{port_reply_ok, send_message_cmd, timer_tick, EffectKind, Harness};

/// 返回首个 Persist correlation。
fn persist_correlation(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(correlation) => Some(*correlation),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing Persist correlation in {effects:?}"))
}

/// 返回发送流程建立的 echo timeout timer。
fn timeout_timer(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer_id) => Some(*timer_id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing send timeout timer in {effects:?}"))
}

/// 构造可在超时后结算同一 temporaryId 的权威 echo。
fn echo_frame(temporary_id: &str, post_id: &str) -> Tick {
    let frame = serde_json::json!({
        "action": "post",
        "data": {
            "id": post_id,
            "temporaryId": temporary_id,
            "channelId": common::cid(1),
            "event_seq": 1,
            "userId": "user-actor-444",
            "type": "TEXT",
            "message": "迟到 echo",
            "createAt": 1_700_000_015_001_u64
        }
    });
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&frame).expect("echo frame must serialize"),
    )))
}

/// 判断 sink 中是否存在指定 temporaryId 的失败投影。
fn has_failed_projection(harness: &Harness, temporary_id: &str) -> bool {
    harness.sink.as_slice().iter().any(|effect| match effect {
        Effect::Emit { event } => serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
            .ok()
            .is_some_and(|envelope| {
                envelope["event"] == "im:post:send-failed"
                    && envelope["data"]["temporaryId"] == temporary_id
            }),
        _ => false,
    })
}

/// 证明 MV3-G02b 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G02b", "post/mv3-g02b");
}

/// 发送超时只结算本机失败态，不自动补发 HTTP，并产出可直接渲染的失败投影。
#[test]
fn timeout_marks_unsend_without_automatic_http_retry() {
    let mut sender = Harness::new(&[1]);
    let initial = sender.step(
        send_message_cmd(1, "tmp-mv3-g02b-timeout", "等待超时"),
        1_700_000_000_000,
    );
    let admitted = sender.step(
        port_reply_ok(persist_correlation(&initial)),
        1_700_000_000_001,
    );
    let timer_id = timeout_timer(&admitted);
    let timed_out = sender.step(timer_tick(timer_id), 1_700_000_015_001);

    assert_eq!(
        sender.module.pending_send_status("tmp-mv3-g02b-timeout"),
        Some(SendStatus::UnSend)
    );
    assert!(has_failed_projection(&sender, "tmp-mv3-g02b-timeout"));
    assert!(
        !timed_out
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "timeout must not retry posts/create automatically"
    );

    let mut receiver =
        Harness::new_with_identity(&[1], "user-receiver-445", "company-phase1", "", "", "");
    assert!(receiver
        .step(timer_tick(timer_id), 1_700_000_015_001)
        .is_empty());
    assert!(!has_failed_projection(&receiver, "tmp-mv3-g02b-timeout"));
}

/// 超时后的迟到权威 echo 仍按原 temporaryId 恢复 sent，并且不会生成第二条消息。
#[test]
fn late_echo_recovers_the_same_failed_identity() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let initial = harness.step(
        send_message_cmd(1, "tmp-mv3-g02b-late", "等待迟到 echo"),
        1_700_000_000_000,
    );
    let admitted = harness.step(
        port_reply_ok(persist_correlation(&initial)),
        1_700_000_000_001,
    );
    harness.step(timer_tick(timeout_timer(&admitted)), 1_700_000_015_001);
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02b-late"),
        Some(SendStatus::UnSend)
    );

    let recovered = harness.step(
        echo_frame("tmp-mv3-g02b-late", "srvfix00000000000000000021"),
        1_700_000_015_002,
    );
    assert!(
        recovered
            .iter()
            .any(|effect| matches!(effect, EffectKind::PersistAtomic(_))),
        "late echo effects: {recovered:?}"
    );
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02b-late"),
        None
    );
}
