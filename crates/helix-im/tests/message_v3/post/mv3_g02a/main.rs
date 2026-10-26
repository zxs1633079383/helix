use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::state::SendStatus;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    command_json, enveloped, port_reply_err, port_reply_ok, port_reply_with_body, send_message_cmd,
    timer_tick, EffectKind, Harness,
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
fn http_admission_never_replaces_the_ws_echo_authority() {
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
        !http_reply.iter().any(|effect| matches!(
            effect,
            EffectKind::Http(_) | EffectKind::Persist(_) | EffectKind::Emit
        )),
        "HTTP admission must not read history, write terminal state or emit: {http_reply:?}"
    );
    assert!(
        !harness.sink.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Http { req, .. } if req.url.ends_with("/posts/getLatestPost")
        )),
        "admission must never poll history for send settlement"
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

const SEND_STATE_REGRESSION_START: u64 = 1_700_000_000_000;
const SEND_STATE_REGRESSION_TEXT: &str = "发送状态回归正文";

/// 从真实发送入口取得内核分配的 temporaryId、HTTP correlation 与实际超时 timer。
fn begin_send_state_regression(harness: &mut Harness) -> (String, u64, u64) {
    let initial = harness.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": common::cid(1),
                "text": SEND_STATE_REGRESSION_TEXT
            }),
        ),
        SEND_STATE_REGRESSION_START,
    );
    let admitted = harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        SEND_STATE_REGRESSION_START + 1,
    );
    let body = harness.last_http_body().expect("real posts/create body");
    let temporary_id = body["temporaryId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("send must allocate a temporaryId")
        .to_string();
    assert_eq!(body["channelId"], common::cid(1));
    assert_eq!(body["message"], SEND_STATE_REGRESSION_TEXT);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending)
    );
    let timer = admitted
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("actual posts/create must schedule its timeout");
    (temporary_id, correlation_of(&admitted, "http"), timer)
}

/// 严格解析生产 Emit；无效 JSON 不得被过滤后伪装成没有事件。
fn send_state_regression_events(harness: &Harness) -> Vec<serde_json::Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(
                serde_json::from_slice(event.0.as_ref()).expect("production Emit must be JSON"),
            ),
            _ => None,
        })
        .collect()
}

/// 对账写必须唯一、只更新原 temporaryId，并携带精确终态及必要的 server identity。
fn assert_send_state_regression_persist(
    harness: &Harness,
    temporary_id: &str,
    status: &str,
    server_id: Option<&str>,
) -> u64 {
    let has_text = |row: &Row, key: &str, expected: &str| {
        row.iter().any(|(column, value)| {
            column == key && matches!(value, SqlValue::Text(actual) if actual == expected)
        })
    };
    let mut writes = Vec::new();
    for effect in harness.sink.as_slice() {
        if let Effect::Persist { corr, ops } = effect {
            for op in ops {
                if let StorageOp::BatchUpsert(spec) = op {
                    if spec.table == "message"
                        && spec
                            .rows
                            .iter()
                            .any(|row| has_text(row, "temporary_id", temporary_id))
                    {
                        writes.push((corr.raw(), spec));
                    }
                }
            }
        }
    }
    assert_eq!(
        writes.len(),
        1,
        "expected one original-identity terminal write"
    );
    let (corr, spec) = writes[0];
    assert_eq!(spec.conflict_key, Some("temporary_id"));
    assert_eq!(spec.rows.len(), 1);
    let row = &spec.rows[0];
    assert_eq!(row.len(), if server_id.is_some() { 3 } else { 2 });
    assert!(has_text(row, "temporary_id", temporary_id));
    assert!(has_text(row, "send_status", status));
    if let Some(server_id) = server_id {
        assert!(has_text(row, "id", server_id));
    }
    corr
}

/// 真实失败只能发布一次精确 failed，并持久化 UnSend、撤销该 attempt 的 timer。
fn assert_send_state_regression_failed(harness: &Harness, temporary_id: &str, timer: u64) -> u64 {
    assert_eq!(
        harness.module.pending_send_status(temporary_id),
        Some(SendStatus::UnSend)
    );
    assert_eq!(
        send_state_regression_events(harness),
        vec![serde_json::json!({
            "event": "im:post:send-failed",
            "data": {
                "channelId": common::cid(1),
                "temporaryId": temporary_id,
                "sendStatus": "failed"
            }
        })]
    );
    assert_eq!(
        harness
            .sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::CancelTimer { id } if id.raw() == timer))
            .count(),
        1
    );
    assert!(!harness.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { .. } | Effect::HttpFire { .. } | Effect::ScheduleTimer { .. }
    )));
    assert_send_state_regression_persist(harness, temporary_id, "unsend", None)
}

/// echo 自己的普通 Persist 才承载 sent 闸；频道 PersistAtomic 不得替代该回执。
fn observe_send_state_regression_echo(
    harness: &mut Harness,
    temporary_id: &str,
    timer: u64,
    now_ms: u64,
) -> u64 {
    let server_id = common::sid(1);
    let effects = harness.step(echo_frame(1, temporary_id, &server_id), now_ms);
    assert_eq!(harness.module.pending_send_status(temporary_id), None);
    assert!(send_state_regression_events(harness).is_empty());
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::CancelTimer(id) if *id == timer))
            .count(),
        1
    );
    let corr =
        assert_send_state_regression_persist(harness, temporary_id, "sent", Some(&server_id));
    assert_ne!(
        corr,
        atomic_correlation(&effects).expect("in-order echo must enter the channel gate")
    );
    corr
}

/// 成功必须由该 echo 的 PersistOk 发布唯一正确 sent，保留原正文与双 identity。
fn confirm_send_state_regression_echo(
    harness: &mut Harness,
    temporary_id: &str,
    corr: u64,
    now_ms: u64,
) {
    harness.step(port_reply_ok(corr), now_ms);
    let events = send_state_regression_events(harness);
    assert_eq!(events.len(), 1, "only one sent terminal may be released");
    assert_eq!(events[0].as_object().expect("event envelope").len(), 2);
    assert_eq!(events[0]["event"], "im:post:sent");
    let data = &events[0]["data"];
    assert_eq!(data["channelId"], common::cid(1));
    assert_eq!(data["temporaryId"], temporary_id);
    assert_eq!(data["id"], common::sid(1));
    assert_eq!(data["serverId"], common::sid(1));
    assert_eq!(data["sendStatus"], "sent");
    assert_eq!(data["userId"], "user-actor-444");
    assert_eq!(data["message"], SEND_STATE_REGRESSION_TEXT);
    assert_eq!(data["type"], "TEXT");
    assert_eq!(data["createAt"], SEND_STATE_REGRESSION_START);
    assert_eq!(harness.module.pending_send_status(temporary_id), None);
    harness.step(port_reply_ok(corr), now_ms + 1);
    assert!(
        harness.sink.is_empty(),
        "duplicate sent PersistOk must be inert"
    );
}

/// 晚到或重复回报不得发布任何事件、重写状态、再起 HTTP 或触碰 timer。
fn assert_send_state_regression_inert(harness: &Harness) {
    assert!(
        harness.sink.is_empty(),
        "stale response must be inert, got {:?}",
        harness.sink.as_slice()
    );
}

/// 回归锚：echo 已持久确认后，旧 HTTP Err 不能把 sent 重新投影为 failed。
#[test]
fn send_state_regression_confirmed_echo_ignores_late_http_error() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, http_corr, timer) = begin_send_state_regression(&mut harness);
    let sent_corr = observe_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        timer,
        SEND_STATE_REGRESSION_START + 2,
    );
    confirm_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        sent_corr,
        SEND_STATE_REGRESSION_START + 3,
    );
    harness.step(port_reply_err(http_corr), SEND_STATE_REGRESSION_START + 5);
    assert_send_state_regression_inert(&harness);
    assert_eq!(harness.module.pending_send_status(&temporary_id), None);
    harness.step(timer_tick(timer), SEND_STATE_REGRESSION_START + 15_001);
    assert_send_state_regression_inert(&harness);
}

/// 回归锚：echo 已移除 pending 但尚未收到自己的 PersistOk 时，HTTP Err 也不能发 failed。
#[test]
fn send_state_regression_observed_echo_ignores_error_before_sent_persist() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, http_corr, timer) = begin_send_state_regression(&mut harness);
    let sent_corr = observe_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        timer,
        SEND_STATE_REGRESSION_START + 2,
    );
    harness.step(port_reply_err(http_corr), SEND_STATE_REGRESSION_START + 3);
    assert_send_state_regression_inert(&harness);
    assert_eq!(harness.module.pending_send_status(&temporary_id), None);
    confirm_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        sent_corr,
        SEND_STATE_REGRESSION_START + 4,
    );
}

/// 回归锚：无 echo 的真实 HTTP Err 必须保留原身份的可重试失败态。
#[test]
fn send_state_regression_unconfirmed_http_error_still_fails() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, http_corr, timer) = begin_send_state_regression(&mut harness);
    harness.step(port_reply_err(http_corr), SEND_STATE_REGRESSION_START + 2);
    let failed_corr = assert_send_state_regression_failed(&harness, &temporary_id, timer);
    harness.step(port_reply_ok(failed_corr), SEND_STATE_REGRESSION_START + 3);
    assert_send_state_regression_inert(&harness);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::UnSend)
    );
}

/// 回归锚：同一 HTTP Err 重复投递及已撤销 timer 回灌，都不能重复失败。
#[test]
fn send_state_regression_duplicate_http_error_is_inert() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, http_corr, timer) = begin_send_state_regression(&mut harness);
    harness.step(port_reply_err(http_corr), SEND_STATE_REGRESSION_START + 2);
    let failed_corr = assert_send_state_regression_failed(&harness, &temporary_id, timer);
    harness.step(port_reply_ok(failed_corr), SEND_STATE_REGRESSION_START + 3);
    assert_send_state_regression_inert(&harness);
    harness.step(port_reply_err(http_corr), SEND_STATE_REGRESSION_START + 4);
    assert_send_state_regression_inert(&harness);
    harness.step(timer_tick(timer), SEND_STATE_REGRESSION_START + 15_001);
    assert_send_state_regression_inert(&harness);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::UnSend)
    );
}

/// 回归锚：真实 Timer 已失败后，迟到 HTTP Err 不可再失败，迟到 echo 仍恢复原身份。
#[test]
fn send_state_regression_timeout_then_error_then_echo_recovers_once() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, http_corr, timer) = begin_send_state_regression(&mut harness);
    harness.step(timer_tick(timer), SEND_STATE_REGRESSION_START + 15_001);
    let failed_corr = assert_send_state_regression_failed(&harness, &temporary_id, timer);
    harness.step(
        port_reply_ok(failed_corr),
        SEND_STATE_REGRESSION_START + 15_002,
    );
    assert_send_state_regression_inert(&harness);
    harness.step(
        port_reply_err(http_corr),
        SEND_STATE_REGRESSION_START + 15_003,
    );
    assert_send_state_regression_inert(&harness);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::UnSend)
    );
    let sent_corr = observe_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        timer,
        SEND_STATE_REGRESSION_START + 15_004,
    );
    confirm_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        sent_corr,
        SEND_STATE_REGRESSION_START + 15_005,
    );
}

/// 回归锚：真实 Timer 后晚 echo 已确认成功，迟到 HTTP Err 不可再降级投影。
#[test]
fn send_state_regression_timeout_then_echo_ignores_late_http_error() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, http_corr, timer) = begin_send_state_regression(&mut harness);
    harness.step(timer_tick(timer), SEND_STATE_REGRESSION_START + 15_001);
    let failed_corr = assert_send_state_regression_failed(&harness, &temporary_id, timer);
    harness.step(
        port_reply_ok(failed_corr),
        SEND_STATE_REGRESSION_START + 15_002,
    );
    assert_send_state_regression_inert(&harness);
    let sent_corr = observe_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        timer,
        SEND_STATE_REGRESSION_START + 15_003,
    );
    confirm_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        sent_corr,
        SEND_STATE_REGRESSION_START + 15_004,
    );
    harness.step(
        port_reply_err(http_corr),
        SEND_STATE_REGRESSION_START + 15_006,
    );
    assert_send_state_regression_inert(&harness);
    assert_eq!(harness.module.pending_send_status(&temporary_id), None);
}

/// 回归锚：真实 retry 复用原身份；旧 H1 Err 不得失败新 attempt，新 H1 的真实失败仍有效。
#[test]
fn send_state_regression_retry_ignores_old_attempt_error() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let (temporary_id, old_http_corr, old_timer) = begin_send_state_regression(&mut harness);
    let original_body = harness.last_http_body().expect("original wire body");
    harness.step(timer_tick(old_timer), SEND_STATE_REGRESSION_START + 15_001);
    let failed_corr = assert_send_state_regression_failed(&harness, &temporary_id, old_timer);
    harness.step(
        port_reply_ok(failed_corr),
        SEND_STATE_REGRESSION_START + 15_002,
    );
    assert_send_state_regression_inert(&harness);

    let retry = harness.step(
        command_json(
            "im_retry_send",
            serde_json::json!({"message_id": temporary_id}),
        ),
        SEND_STATE_REGRESSION_START + 15_003,
    );
    assert!(!retry
        .iter()
        .any(|effect| matches!(effect, EffectKind::Http(_))));
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Local)
    );
    let retried = harness.step(
        port_reply_ok(correlation_of(&retry, "persist")),
        SEND_STATE_REGRESSION_START + 15_004,
    );
    let new_http_corr = correlation_of(&retried, "http");
    let new_timer = retried
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("retry must arm its own timer");
    assert_ne!(new_http_corr, old_http_corr);
    assert_ne!(new_timer, old_timer);
    assert_eq!(
        harness.last_http_body().expect("retry wire body"),
        original_body
    );
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending)
    );

    harness.step(
        port_reply_err(old_http_corr),
        SEND_STATE_REGRESSION_START + 15_005,
    );
    assert_send_state_regression_inert(&harness);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending)
    );
    harness.step(
        port_reply_err(old_http_corr),
        SEND_STATE_REGRESSION_START + 15_006,
    );
    assert_send_state_regression_inert(&harness);
    harness.step(
        port_reply_err(new_http_corr),
        SEND_STATE_REGRESSION_START + 15_007,
    );
    let failed_corr = assert_send_state_regression_failed(&harness, &temporary_id, new_timer);
    harness.step(
        port_reply_ok(failed_corr),
        SEND_STATE_REGRESSION_START + 15_008,
    );
    assert_send_state_regression_inert(&harness);

    let sent_corr = observe_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        new_timer,
        SEND_STATE_REGRESSION_START + 15_009,
    );
    confirm_send_state_regression_echo(
        &mut harness,
        &temporary_id,
        sent_corr,
        SEND_STATE_REGRESSION_START + 15_010,
    );
    harness.step(
        port_reply_err(new_http_corr),
        SEND_STATE_REGRESSION_START + 15_012,
    );
    assert_send_state_regression_inert(&harness);
    assert_eq!(harness.module.pending_send_status(&temporary_id), None);
}
