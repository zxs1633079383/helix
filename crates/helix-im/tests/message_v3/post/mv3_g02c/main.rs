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

/// 解码公开 seam 本批次的消息终态，不接受其他事件替代发送完成。
fn terminal_events(harness: &Harness) -> Vec<serde_json::Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| {
            let helix_core::Effect::Emit { event } = effect else {
                return None;
            };
            let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            matches!(
                value["event"].as_str(),
                Some("im:post:received" | "im:post:sent")
            )
            .then_some(value)
        })
        .collect()
}

/// 复现权威 WS echo 先于终态：echo 已带上服务端消息与 id，但 PersistOk 之前不能发终态。
///
/// retry 的 posts/create admission 只代表 transport 接收，不发起任何历史回读；
/// `temporary_id -> post.id` 结算只由权威 WS post echo 推进。
fn retry_echo_before_terminal() -> (Harness, u64) {
    let mut harness = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
    let initial = harness.step(send_message_cmd(1, "tmp-retry-authority", "重试正文"), 1000);
    let admitted = harness.step(port_reply_ok(correlation_of(&initial, "persist")), 1001);
    harness.step(port_reply_err(correlation_of(&admitted, "http")), 1002);
    let retry = harness.step(retry_command("tmp-retry-authority"), 1003);
    let admitted = harness.step(port_reply_ok(correlation_of(&retry, "persist")), 1004);
    let admission = harness.step(port_reply_ok(correlation_of(&admitted, "http")), 1005);
    assert!(
        !admission.iter().any(|effect| matches!(
            effect,
            EffectKind::Http(_) | EffectKind::Persist(_)
        )),
        "retry admission must wait for the authoritative WS echo: {admission:?}"
    );

    let frame = serde_json::json!({"action":"post", "data": {
        "id":common::sid(1), "temporaryId":"tmp-retry-authority", "channelId":common::cid(1),
        "event_seq":8, "userId":"actor", "type":"TEXT", "message":"重试正文", "createAt":1000
    }});
    harness.step(
        helix_core::Tick::Inbound(helix_core::tick::InboundBytes(bytes::Bytes::from(
            serde_json::to_vec(&frame).unwrap(),
        ))),
        1006,
    );
    assert!(
        terminal_events(&harness).is_empty(),
        "echo authority must wait for durable settlement"
    );
    let corr = harness.sink.as_slice().iter().find_map(|effect| {
        let helix_core::Effect::Persist { corr, ops } = effect else { return None; };
        ops.iter().any(|op| matches!(op, helix_core::effect::StorageOp::BatchUpsert(spec)
            if spec.table == "message" && spec.rows.iter().any(|row| row.iter().any(|(key,_)| key=="send_status"))))
            .then_some(corr.raw())
    }).expect("echo authority must persist the send reconciliation");
    (harness, corr)
}

/// 同一 echo authority 在持久化成功后只发 sender 终态，重复回报不能重复终态。
///
/// 本帧 userId == auth user，viewer 是发送者，因此终态是 `im:post:sent`；
/// `im:post:received` 只属于其它成员的权威 post（对齐 MV3-G02c outbound viewer 语义）。
#[test]
fn echo_retry_emits_sent_only_after_persist_success() {
    let (mut harness, corr) = retry_echo_before_terminal();
    harness.step(port_reply_ok(corr), 1007);
    let events = terminal_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:post:sent");
    assert_eq!(events[0]["data"]["temporaryId"], "tmp-retry-authority");
    harness.step(port_reply_ok(corr), 1008);
    assert!(terminal_events(&harness).is_empty());
}

/// 对账写失败不能把 echo authority 包装成任何消息成功终态。
#[test]
fn echo_retry_persist_failure_suppresses_terminal() {
    let (mut harness, corr) = retry_echo_before_terminal();
    harness.step(port_reply_err(corr), 1007);
    assert!(terminal_events(&harness).is_empty());
}

/// 结算已推进cursor后迟到的同一WS echo不能再次发终态或重放旧message/channel写集。
#[test]
fn late_echo_after_authority_does_not_replay_committed_stream() {
    let (mut harness, corr) = retry_echo_before_terminal();
    harness.step(port_reply_ok(corr), 1007);
    assert_eq!(terminal_events(&harness).len(), 1);
    harness.module.register_channel(common::channel_id(1), 8);
    let frame = serde_json::json!({"action":"post", "data": {
        "id":common::sid(1), "temporaryId":"tmp-retry-authority", "channelId":common::cid(1),
        "event_seq":8, "userId":"actor", "type":"TEXT", "message":"过期正文", "createAt":1000
    }});
    let effects = harness.step(
        helix_core::Tick::Inbound(helix_core::tick::InboundBytes(bytes::Bytes::from(
            serde_json::to_vec(&frame).unwrap(),
        ))),
        1008,
    );
    assert!(terminal_events(&harness).is_empty());
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        EffectKind::Persist(_) | EffectKind::PersistAtomic(_) | EffectKind::PersistFire
    )));
}
