use helix_im::state::SendStatus;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, port_reply_err, port_reply_ok, send_message_cmd, EffectKind, Harness};

/// 返回首个指定类别 effect 的 correlation。
fn correlation_of(effects: &[EffectKind], kind: &str) -> u64 {
    effects
        .iter()
        .find_map(|effect| match (kind, effect) {
            ("persist", EffectKind::Persist(correlation))
            | ("http", EffectKind::Http(correlation)) => Some(*correlation),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {kind} correlation in {effects:?}"))
}

/// 提交只带消息对账键的 retry 命令。
fn retry_command(temporary_id: &str) -> helix_core::Tick {
    command_json(
        "im_retry_send",
        serde_json::json!({"message_id": temporary_id}),
    )
}

/// 证明 MV3-G02c 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G02c", "post/mv3-g02c");
}

/// 普通失败重试复用原 temporaryId 和原 wire body，并在持久化闸后单次 HTTP 出站。
#[test]
fn retry_reuses_the_failed_identity_and_body() {
    let mut harness = Harness::new(&[1]);
    let initial = harness.step(
        send_message_cmd(1, "tmp-mv3-g02c-retry", "重试正文"),
        1_700_000_000_000,
    );
    let admitted = harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    let first_body = harness.last_http_body().expect("initial posts/create body");
    harness.step(
        port_reply_err(correlation_of(&admitted, "http")),
        1_700_000_000_002,
    );
    assert_eq!(
        harness.module.pending_send_status("tmp-mv3-g02c-retry"),
        Some(SendStatus::UnSend)
    );

    let retry_persist = harness.step(retry_command("tmp-mv3-g02c-retry"), 1_700_000_000_003);
    assert_eq!(
        retry_persist
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Persist(_)))
            .count(),
        1
    );
    assert!(!retry_persist
        .iter()
        .any(|effect| matches!(effect, EffectKind::Http(_))));

    let retry_started = harness.step(
        port_reply_ok(correlation_of(&retry_persist, "persist")),
        1_700_000_000_004,
    );
    assert_eq!(
        retry_started
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1
    );
    assert_eq!(
        harness.last_http_body().expect("retry posts/create body"),
        first_body
    );
    assert_eq!(first_body["temporaryId"], "tmp-mv3-g02c-retry");
}

/// 首次 retry 尚在 Persist/HTTP 进行态时重复点击必须无副作用。
#[test]
fn duplicate_retry_clicks_keep_one_inflight_attempt() {
    let mut harness = Harness::new(&[1]);
    let initial = harness.step(
        send_message_cmd(1, "tmp-mv3-g02c-singleflight", "单飞重试"),
        1_700_000_000_000,
    );
    let admitted = harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    harness.step(
        port_reply_err(correlation_of(&admitted, "http")),
        1_700_000_000_002,
    );

    let first_retry = harness.step(
        retry_command("tmp-mv3-g02c-singleflight"),
        1_700_000_000_003,
    );
    let duplicate = harness.step(
        retry_command("tmp-mv3-g02c-singleflight"),
        1_700_000_000_004,
    );

    assert_eq!(
        first_retry
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Persist(_)))
            .count(),
        1
    );
    assert!(
        duplicate.is_empty(),
        "duplicate retry must not enqueue another Persist/HTTP/timer: {duplicate:?}"
    );
}
