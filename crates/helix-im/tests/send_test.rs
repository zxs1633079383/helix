//! 发送链路测试（Phase B Commit 3）
//!
//! 验证 §4.5 发送机制：
//! - im_send_message → Persist{P1} + Emit(im:post:sending)
//! - PortReply(P1 Ok) → attached surface 的真实 Frame；无 attached surface 时直接吐
//!   Http{H1} + ScheduleTimer{T1, 15s}
//! - timer 15s 超时 → UnSend + PersistFire（标记 unsend）
//! - 幂等：双重发送命令各自独立（不互相影响）

mod common;
use common::*;

use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_im::state::SendStatus;
use serde_json::Value;

fn http_corr(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(corr) = e {
                Some(*corr)
            } else {
                None
            }
        })
        .expect("P1 acknowledgement should emit Http(H1)")
}

fn p1_corr(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("ordinary send must begin with P1 persist")
}

fn acknowledge_optimistic_persist(
    harness: &mut Harness,
    initial_effects: &[EffectKind],
    now_ms: u64,
) -> Vec<EffectKind> {
    harness.step(port_reply_ok(p1_corr(initial_effects)), now_ms)
}

// Advances a retry only after its sending status update has become durable.
fn acknowledge_retry_persist(
    harness: &mut Harness,
    retry_effects: &[EffectKind],
    now_ms: u64,
) -> Vec<EffectKind> {
    assert!(
        retry_effects
            .iter()
            .all(|effect| !matches!(effect, EffectKind::Http(_))),
        "retry HTTP must wait for the sending persist barrier"
    );
    harness.step(port_reply_ok(p1_corr(retry_effects)), now_ms)
}

fn unsend_persist_fire_for_tmp(h: &Harness, tmp: &str) -> bool {
    h.sink.as_slice().iter().any(|e| match e {
        Effect::Persist { ops, .. } | Effect::PersistFire { ops } => {
            ops.iter().any(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                    spec.conflict_key == Some("temporary_id")
                        && spec.rows.iter().any(|row| {
                            let has_tmp = row.iter().any(|(k, v)| {
                                k == "temporary_id" && matches!(v, SqlValue::Text(t) if t == tmp)
                            });
                            let has_unsend = row.iter().any(|(k, v)| {
                                k == "send_status"
                                    && matches!(v, SqlValue::Text(s) if s == "unsend")
                            });
                            has_tmp && has_unsend
                        })
                }
                _ => false,
            })
        }
        _ => false,
    })
}

fn failed_projection_for_tmp(h: &Harness, tmp: &str) -> Option<Value> {
    h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Emit { event } => {
            let v: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (v["event"] == "im:post:send-failed" && v["data"]["temporaryId"] == tmp).then_some(v)
        }
        _ => None,
    })
}

fn sending_projection_for_tmp(h: &Harness, tmp: &str) -> Option<Value> {
    h.sink
        .as_slice()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (value["event"] == "im:post:sending" && value["data"]["temporaryId"] == tmp)
                    .then_some(value)
            }
            _ => None,
        })
}

fn last_sending_projection(h: &Harness) -> Option<Value> {
    h.sink
        .as_slice()
        .iter()
        .rev()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (value["event"] == "im:post:sending").then_some(value)
            }
            _ => None,
        })
}

/// 从当前 optimistic 写集中读取 Helix 分配的 temporaryId。
fn persisted_temporary_id(h: &Harness) -> Option<String> {
    h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                spec.rows.iter().find_map(|row| {
                    row.iter()
                        .find_map(|(key, value)| match (key.as_str(), value) {
                            ("temporary_id", SqlValue::Text(value)) => Some(value.clone()),
                            _ => None,
                        })
                })
            }
            _ => None,
        }),
        _ => None,
    })
}

/// Helix 分配的 temporaryId 必须贯穿 optimistic 持久化、sending 投影和 HTTP。
#[test]
fn send_message_mints_one_temporary_id_for_projection_persist_and_http() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let first_effects = h.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": cid(1),
                "text": "helix owns id"
            }),
        ),
        1_700_000_000_000,
    );

    let temporary_id =
        persisted_temporary_id(&h).expect("optimistic persist must contain Helix temporary_id");
    assert!(temporary_id.starts_with("helix_tmp_"));

    let persisted_same_id = h.sink.as_slice().iter().any(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                spec.rows.iter().any(|row| {
                    row.iter().any(|(key, value)| {
                        key == "temporary_id"
                            && matches!(value, SqlValue::Text(value) if value == temporary_id.as_str())
                    })
                })
            }
            _ => false,
        }),
        _ => false,
    });
    assert!(persisted_same_id, "optimistic row must use the minted id");

    let _ = acknowledge_optimistic_persist(&mut h, &first_effects, 1_700_000_000_001);
    let projection =
        last_sending_projection(&h).expect("PersistOk must emit optimistic projection");
    assert_eq!(projection["data"]["temporaryId"], temporary_id);

    let http_same_id = h.sink.as_slice().iter().any(|effect| match effect {
        Effect::Http { req, .. } if req.url.ends_with("/posts/create") => {
            req.body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<Value>(body).ok())
                .and_then(|body| body["temporaryId"].as_str().map(str::to_string))
                .as_deref()
                == Some(temporary_id.as_str())
        }
        _ => false,
    });
    assert!(http_same_id, "posts/create must use the minted id");

    let second_effects = h.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": cid(1),
                "temporary_id": "caller-must-not-own-this",
                "text": "second send"
            }),
        ),
        1_700_000_000_000,
    );
    let second_persisted_id =
        persisted_temporary_id(&h).expect("second optimistic persist must contain temporary_id");
    let _ = acknowledge_optimistic_persist(&mut h, &second_effects, 1_700_000_000_001);
    let second = last_sending_projection(&h).expect("second PersistOk projection");
    let second_id = second["data"]["temporaryId"].as_str().unwrap();
    assert_eq!(second_id, second_persisted_id);
    assert_ne!(second_id, temporary_id);
    assert_ne!(second_id, "caller-must-not-own-this");
}

// ─── 发送命令跨持久屏障产出发送 Effects ───────────────────────────────────────

/// 初始命令只写 optimistic 事实，PersistOk 后才 Emit/Http/ScheduleTimer。
#[test]
fn send_message_emits_four_effects() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);

    // 应产出 Persist{P1}（乐观落库）
    assert!(
        initial_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "im_send_message should emit Persist(P1), got: {:?}",
        initial_effects
    );
    // P1 尚未确认时不得抢跑 Http{H1} / timer。
    assert!(
        initial_effects
            .iter()
            .all(|e| !matches!(e, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))),
        "im_send_message must wait for P1 before HTTP/timer, got: {:?}",
        initial_effects
    );
    // P1 尚未确认时不得发布 im:post:sending。
    assert!(
        initial_effects
            .iter()
            .all(|e| !matches!(e, EffectKind::Emit)),
        "im_send_message must wait for P1 before sending projection, got: {:?}",
        initial_effects
    );
    let after_p1 = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    assert!(
        after_p1.iter().any(|e| matches!(e, EffectKind::Emit)),
        "P1 acknowledgement should emit im:post:sending, got: {:?}",
        after_p1
    );
    assert!(
        after_p1.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "P1 acknowledgement should emit Http(H1), got: {:?}",
        after_p1
    );
    assert_eq!(
        after_p1
            .iter()
            .filter(|e| matches!(e, EffectKind::ScheduleTimer(_)))
            .count(),
        1,
        "P1 acknowledgement should arm exactly one send timeout, got: {:?}",
        after_p1
    );
}

#[test]
fn send_message_forwards_request_id_as_trace_header() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(
        send_message_cmd_json(serde_json::json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_trace_1",
            "text": "hello",
            "req_id": "mrc-run:G-03:req-1"
        })),
        1000,
    );
    let _ = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);

    let request = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/create") => Some(req),
            _ => None,
        })
        .expect("send should emit posts/create");
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name == "Cses-Track-Id" && value == "mrc-run:G-03:req-1"));
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name == "X-Auth-Kind" && value == "session"));
}

// ─── 两条消息各自独立 ────────────────────────────────────────────────────────

#[test]
fn send_two_messages_each_independent() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let e1 = h.step(send_message_cmd(1, "tmp_1", "msg1"), 1000);
    let e2 = h.step(send_message_cmd(1, "tmp_2", "msg2"), 1001);

    // 两条消息各有自己的 Persist corr（不同）
    let corr1 = e1
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .unwrap();
    let corr2 = e2
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .unwrap();
    assert_ne!(
        corr1, corr2,
        "two messages should have different Persist corrs"
    );

    // P1 后各自 timer 也不同。
    let after_p1 = acknowledge_optimistic_persist(&mut h, &e1, 1002);
    let t1 = after_p1
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    let after_p2 = acknowledge_optimistic_persist(&mut h, &e2, 1003);
    let t2 = after_p2
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    assert_ne!(t1, t2, "two messages should have different timeout timers");
}

// ─── P1 落库成功 → PendingSend 推进到 Sending ───────────────────────────────

#[test]
fn send_p1_persist_ok_advances_status() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
    // P1 落库成功：无 attach surface 的 Harness 直接推进 H1 + timeout。
    let reply_effects = acknowledge_optimistic_persist(&mut h, &effects, 1100);
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))),
        "P1 ok should start H1 and its timeout, got: {:?}",
        reply_effects
    );
    // pending_send 仍存在（等待 echo 对账）
    assert_eq!(
        h.module.pending_send_count(),
        1,
        "pending_send should still be 1 after P1 ok"
    );
}

// ─── 15s 超时 → UnSend + Persist ─────────────────────────────────────────────

#[test]
/// 证明 15 秒无权威回声会进入有回执的 unsend 持久化边界。
fn send_timeout_marks_unsend() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    let timeout_timer = effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("timeout timer");

    // 15s 超时触发
    let timeout_effects = h.step(timer_tick(timeout_timer), 16_001);
    // 应产出 Persist（标记 unsend）
    assert!(
        timeout_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "timeout should emit Persist to mark unsend, got: {:?}",
        timeout_effects
    );
    assert!(
        failed_projection_for_tmp(&h, "tmp_1").is_some(),
        "timeout must project the durable unsend row as retryable failed, got: {:?}",
        timeout_effects
    );
}

#[test]
/// 15s timeout 也必须携带公开发送命令保存的 request causation。
fn send_timeout_preserves_request_id_in_failed_projection() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": cid(1),
                "allocated_temporary_id": "tmp-timeout-request",
                "req_id": "send-timeout-request-1",
                "text": "request-bound timeout"
            }),
        ),
        1000,
    );
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    let timeout_timer = effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .expect("timeout timer");

    h.step(timer_tick(timeout_timer), 16_001);

    let projection = failed_projection_for_tmp(&h, "tmp-timeout-request")
        .expect("timeout must emit a request-bound send-failed projection");
    assert_eq!(projection["data"]["requestId"], "send-timeout-request-1");
}

#[test]
fn pending_send_http_error_marks_unsend_cancels_timer_and_emits_failed_projection() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(send_message_cmd(1, "tmp_fail", "hello"), 1000);
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    let h1 = http_corr(&effects);
    let timeout_timer = effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("send should schedule timeout timer");

    let failed_effects = h.step(port_reply_err(h1), 1100);

    assert!(
        unsend_persist_fire_for_tmp(&h, "tmp_fail"),
        "HTTP Err must immediately persist send_status='unsend', got: {:?}",
        h.sink.as_slice()
    );
    assert!(
        failed_effects
            .iter()
            .any(|e| *e == EffectKind::CancelTimer(timeout_timer)),
        "HTTP Err must cancel the pending send timeout, got: {failed_effects:?}"
    );
    let projection = failed_projection_for_tmp(&h, "tmp_fail")
        .expect("HTTP Err must emit im:post:send-failed anchored by temporaryId");
    assert_eq!(projection["data"]["sendStatus"], "failed");
    assert_eq!(projection["data"]["channelId"], cid(1));
    assert!(
        projection["data"].get("requestId").is_none(),
        "legacy send-failed payload without a request causation stays unchanged"
    );
    assert_eq!(
        h.module.pending_send_status("tmp_fail"),
        Some(SendStatus::UnSend),
        "failed send remains retained as terminal UnSend until any later echo/backfill"
    );
}

#[test]
/// 公开发送命令的 request causation 必须跟随失败终态，供客户端回收本轮绑定。
fn pending_send_http_error_preserves_request_id_in_failed_projection() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": cid(1),
                "allocated_temporary_id": "tmp-request-bound",
                "req_id": "send-failure-request-1",
                "text": "request-bound failure"
            }),
        ),
        1000,
    );
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    let h1 = http_corr(&effects);

    h.step(port_reply_err(h1), 1100);

    let projection = failed_projection_for_tmp(&h, "tmp-request-bound")
        .expect("HTTP Err must emit a request-bound send-failed projection");
    assert_eq!(projection["data"]["requestId"], "send-failure-request-1");
}

#[test]
/// 证明迟到的初始 P1 回包不会阻断随后经过新持久屏障的 retry。
fn late_optimistic_persist_reply_does_not_reopen_failed_send() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let effects = h.step(send_message_cmd(1, "tmp_late_p1", "hello"), 1000);
    let p1 = p1_corr(&effects);
    let after_p1 = acknowledge_optimistic_persist(&mut h, &effects, 1001);
    let h1 = http_corr(&after_p1);

    h.step(port_reply_err(h1), 1050);
    assert_eq!(
        h.module.pending_send_status("tmp_late_p1"),
        Some(SendStatus::UnSend)
    );

    h.step(port_reply_ok(p1), 1100);
    assert_eq!(
        h.module.pending_send_status("tmp_late_p1"),
        Some(SendStatus::UnSend),
        "late P1 must not overwrite the terminal retryable state"
    );

    let retry_persist = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({ "message_id": "tmp_late_p1" }),
        ),
        1200,
    );
    let retry_effects = acknowledge_retry_persist(&mut h, &retry_persist, 1201);
    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "the failed send must remain retryable after a late P1 reply"
    );
}

#[test]
/// 证明 retry 在持久化 sending 后复用原 body、temporaryId 与 posts/create。
fn retry_send_reuses_cached_body_and_temporary_id_after_real_http_error() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(send_message_cmd(1, "tmp_retry_send", "hello retry"), 1000);
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    let h1 = http_corr(&effects);
    let first_body = h.last_http_body().expect("first posts/create body");
    h.step(port_reply_err(h1), 1100);
    assert_eq!(
        h.module.pending_send_status("tmp_retry_send"),
        Some(SendStatus::UnSend)
    );
    let retry_persist = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({ "message_id": "tmp_retry_send" }),
        ),
        1200,
    );
    let retry_effects = acknowledge_retry_persist(&mut h, &retry_persist, 1201);

    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "retry must emit a real posts/create Http effect: {retry_effects:?}"
    );
    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::ScheduleTimer(_))),
        "retry must arm a fresh timeout: {retry_effects:?}"
    );
    assert_eq!(
        h.module.pending_send_status("tmp_retry_send"),
        Some(SendStatus::Sending)
    );
    let retry_body = h.last_http_body().expect("retry posts/create body");
    assert_eq!(
        retry_body, first_body,
        "retry must reuse the original Helix body"
    );
    assert_eq!(retry_body["temporaryId"], "tmp_retry_send");
}

#[test]
/// 证明 retry 的乐观投影保留原消息回复锚点。
fn retry_send_keeps_reply_anchors_in_optimistic_projection() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(
        send_message_cmd_json(serde_json::json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_retry_reply",
            "text": "回复重试",
            "replied": {
                "id": "reply-target",
                "reply_id": "parent-message",
                "reply_root_id": "root-message",
                "reply_first_level_id": "first-level"
            }
        })),
        1_000,
    );
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1_001);
    h.step(port_reply_err(http_corr(&effects)), 1_100);

    let retry_persist = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({ "message_id": "tmp_retry_reply" }),
        ),
        1_200,
    );
    let sending = sending_projection_for_tmp(&h, "tmp_retry_reply")
        .expect("retry must emit a render-ready optimistic row");
    assert_eq!(sending["data"]["replyId"], "reply-target");
    assert_eq!(sending["data"]["replyRootId"], "root-message");
    assert_eq!(sending["data"]["replyFirstLevelId"], "first-level");
    acknowledge_retry_persist(&mut h, &retry_persist, 1_201);
}

#[test]
/// 证明冷启动 durable row 经 retry 持久屏障恢复原发送合同。
fn retry_send_rehydrates_failed_message_after_native_restart() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let lookup_effects = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({
                "message_id": "tmp_durable_retry",
                "req_id": "mrc:G-03a:retry"
            }),
        ),
        2_000,
    );
    let lookup_corr = lookup_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("cold retry must query durable message");
    assert!(
        !lookup_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "posts/create must wait for the durable row"
    );
    let get = h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::Get(spec) => Some(spec),
            _ => None,
        }),
        _ => None,
    });
    assert!(
        matches!(get, Some(spec) if spec.table == "message" && spec.key_col == "temporary_id"),
        "cold retry first lookup must use the message PK"
    );

    let row = serde_json::json!([{
        "temporary_id": "tmp_durable_retry",
        "id": "",
        "channel_id": cid(1),
        "user_id": "user-before-restart",
        "type": "TEXT",
        "message": "durable hello",
        "props": "{\"vote\":{\"id\":\"v1\"}}",
        "viewers": "[\"u2\"]",
        "mentions": "[\"u2\"]",
        "user_snapshot": "{\"userId\":\"user-before-restart\",\"teamId\":\"team-1\"}",
        "topic": "topic-1",
        "reply_id": "reply-1",
        "reply_root_id": "root-1",
        "reply_first_level_id": "first-1",
        "create_at": 1234,
        "send_status": "unsend"
    }]);
    let retry_persist = h.step(
        port_reply_with_body(
            lookup_corr,
            bytes::Bytes::from(serde_json::to_vec(&row).unwrap()),
        ),
        2_010,
    );
    assert!(
        retry_persist
            .iter()
            .any(|effect| matches!(effect, EffectKind::Persist(_))),
        "rehydrated retry must persist sending behind a reply barrier"
    );
    assert!(
        retry_persist
            .iter()
            .any(|effect| matches!(effect, EffectKind::Emit)),
        "rehydrated retry must emit the optimistic sending projection"
    );
    let sending = sending_projection_for_tmp(&h, "tmp_durable_retry")
        .expect("durable retry must emit a render-ready optimistic row");
    let retry_effects = acknowledge_retry_persist(&mut h, &retry_persist, 2_011);
    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "rehydrated retry must continue to the real posts/create HTTP"
    );
    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::ScheduleTimer(_))),
        "rehydrated retry must arm a fresh timeout"
    );
    assert_eq!(
        h.module.pending_send_status("tmp_durable_retry"),
        Some(SendStatus::Sending)
    );
    let retry_body = h.last_http_body().expect("rehydrated posts/create body");
    assert_eq!(retry_body["temporaryId"], "tmp_durable_retry");
    assert_eq!(retry_body["channelId"], cid(1));
    assert_eq!(retry_body["message"], "durable hello");
    assert_eq!(retry_body["viewers"], serde_json::json!(["u2"]));
    assert_eq!(retry_body["mentions"], serde_json::json!(["u2"]));
    assert_eq!(retry_body["props"]["vote"]["id"], "v1");
    assert_eq!(retry_body["topicId"], "topic-1");
    assert_eq!(retry_body["replyId"], "reply-1");
    assert_eq!(retry_body["replyRootId"], "root-1");
    assert_eq!(retry_body["replyFirstLevelId"], "first-1");
    assert_eq!(sending["data"]["replyId"], "reply-1");
    assert_eq!(sending["data"]["replyRootId"], "root-1");
    assert_eq!(sending["data"]["replyFirstLevelId"], "first-1");
    assert_eq!(retry_body["createAt"], 1234);
    let track = h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Http { req, .. } => req
            .headers
            .iter()
            .find(|(key, _)| key == "Cses-Track-Id")
            .map(|(_, value)| value.as_str()),
        _ => None,
    });
    assert_eq!(track, Some("mrc:G-03a:retry"));
}

#[test]
/// 证明 server id fallback 命中后仍以 durable temporaryId 继续 retry。
fn retry_send_falls_back_from_temporary_id_to_server_id_lookup() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let first = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({ "message_id": "server-id-anchor" }),
        ),
        3_000,
    );
    let first_corr = first
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("temporary_id lookup corr");
    let second = h.step(port_reply_with_body(first_corr, bytes::Bytes::new()), 3_010);
    let second_corr = second
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("id fallback lookup corr");
    let get = h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::Get(spec) => Some(spec),
            _ => None,
        }),
        _ => None,
    });
    assert!(
        matches!(get, Some(spec) if spec.key_col == "id"),
        "second durable retry lookup must fall back to indexed server id"
    );

    let row = serde_json::json!([{
        "temporary_id": "tmp-from-server-id",
        "id": "server-id-anchor",
        "channel_id": cid(1),
        "type": "TEXT",
        "message": "fallback",
        "props": "{}",
        "send_status": "unsend"
    }]);
    let retry_persist = h.step(
        port_reply_with_body(
            second_corr,
            bytes::Bytes::from(serde_json::to_vec(&row).unwrap()),
        ),
        3_020,
    );
    let retry = acknowledge_retry_persist(&mut h, &retry_persist, 3_021);
    assert!(retry
        .iter()
        .any(|effect| matches!(effect, EffectKind::Http(_))));
    assert_eq!(
        h.last_http_body().unwrap()["temporaryId"],
        "tmp-from-server-id"
    );
}

// ─── echo 先到（reconcile）→ CancelTimer + Persist + 清除 pending_send ──────

#[test]
/// 证明超时持久化只触发一次，重复 timer 不会重复失败写入。
fn duplicate_timeout_does_not_repeat_failed_persist() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    let initial_effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
    let effects = acknowledge_optimistic_persist(&mut h, &initial_effects, 1001);
    let timeout_timer = effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("timeout timer");

    // 此测试仅覆盖重复超时；真实回声由 self_echo_with_channel_gap_* 回放。
    let timeout_effects = h.step(timer_tick(timeout_timer), 16_001);
    assert!(
        timeout_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "timeout should emit Persist"
    );

    // 超时后再次触发同一 timer → 静默（timer 已从 pending_sends 中无法找到，或 status=UnSend 幂等）
    let effects2 = h.step(timer_tick(timeout_timer), 16_002);
    // 不应再次产出 Persist
    assert!(
        !effects2.iter().any(|e| matches!(e, EffectKind::Persist(_))),
        "double-fire timeout should be idempotent, got: {:?}",
        effects2
    );
}

// ─── 发送 + 接收连续事件（混合场景）─────────────────────────────────────────

#[test]
fn send_and_receive_interleaved() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    // 先发一条消息
    let _send_effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);

    // 同时收到 seq=1 的 WS 事件（其他人发的消息）
    let recv_effects = h.step(inbound_event(1, 1, 1), 1001);
    assert!(
        recv_effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistAtomic(_))),
        "inbound WS event should enter MessageV3 atomic commit while send is pending"
    );

    // 发送的消息和接收的事件应该互不影响
    assert_eq!(
        h.module.pending_send_count(),
        1,
        "pending send should still be 1 after receiving other message"
    );
}

/// 通过完整查询、缓存提交与读回建立空时间线，避免直接改内部附着状态。
fn attach_empty_send_timeline(h: &mut Harness, channel: u64) {
    let query = h.step(send_timeline_query(channel), 100);
    let remote = h.step(
        port_reply_with_body(p1_corr(&query), bytes::Bytes::from_static(b"[]")),
        101,
    );
    let cache = h.step(
        port_reply_with_body(
            http_corr(&remote),
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"SUCCESS","data":[]}"#,
            )),
        ),
        102,
    );
    let readback = h.step(port_reply_ok(p1_corr(&cache)), 103);
    let ready = h.step(
        port_reply_with_body(p1_corr(&readback), bytes::Bytes::from_static(b"[]")),
        104,
    );
    assert!(ready
        .iter()
        .any(|effect| matches!(effect, EffectKind::Emit)));
}

/// 查询同一窗口以模拟滚动刷新或连续操作推进查询代际。
fn send_timeline_query(channel: u64) -> helix_core::Tick {
    helix_core::Tick::Command(helix_core::tick::AppCommand::new(
        "im_query_messages_by_channel",
        serde_json::to_vec(&serde_json::json!({"channel_id": cid(channel), "window_token": "send-race", "limit": 20})).unwrap(),
    ))
}

/// 旧窗口读回可以丢弃投影，但已持久化消息仍必须发送并启动独立超时。
#[test]
fn superseded_send_readback_still_sends_and_times_out() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    attach_empty_send_timeline(&mut h, 1);
    let initial = h.step(send_message_cmd(1, "race_a", "hello"), 1000);
    let readback = acknowledge_optimistic_persist(&mut h, &initial, 1001);
    assert!(!readback
        .iter()
        .any(|effect| matches!(effect, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))));
    h.step(send_timeline_query(1), 1002);
    let completed = h.step(
        port_reply_with_body(p1_corr(&readback), bytes::Bytes::from_static(b"[]")),
        1003,
    );
    assert_eq!(
        completed
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1,
        "superseding a UI query must not cancel sending"
    );
    assert!(
        !completed
            .iter()
            .any(|effect| matches!(effect, EffectKind::Emit)),
        "stale window must not overwrite the current view"
    );
    let timer = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleTimer { id, after_ms } => {
                assert_eq!(*after_ms, 15_000);
                Some(id.raw())
            }
            _ => None,
        })
        .expect("each send needs its timeout");
    h.step(timer_tick(timer), 16003);
    assert!(failed_projection_for_tmp(&h, "race_a").is_some());
}

/// 千条消息跨四频道交错落库并逆序读回：每条发送恰好一次，各自拥有 timer。
#[test]
fn concurrent_send_readbacks_keep_independent_http_and_timers() {
    let mut h =
        Harness::new_with_identity(&[1, 2, 3, 4], "send-viewer", "send-company", "", "", "");
    for channel in 1..=4 {
        attach_empty_send_timeline(&mut h, channel);
    }
    let mut pending = Vec::new();
    for index in 0..1024_u64 {
        let channel = index % 4 + 1;
        let temporary_id = format!("burst_{index}");
        let initial = h.step(
            send_message_cmd(channel, &temporary_id, "burst"),
            1000 + index * 2,
        );
        let readback = acknowledge_optimistic_persist(&mut h, &initial, 1001 + index * 2);
        pending.push((p1_corr(&readback), channel, temporary_id));
    }
    for channel in 1..=4 {
        h.step(send_timeline_query(channel), 4000);
    }
    let mut timers = std::collections::HashSet::new();
    for (corr, channel, temporary_id) in pending.into_iter().rev() {
        let effects = h.step(
            port_reply_with_body(corr, bytes::Bytes::from_static(b"[]")),
            5000,
        );
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, EffectKind::Http(_)))
                .count(),
            1
        );
        let body = h.last_http_body().expect("matching posts/create");
        assert_eq!(body["temporaryId"], temporary_id);
        assert_eq!(body["channelId"], cid(channel));
        let timer_ids: Vec<_> = h
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::ScheduleTimer { id, after_ms } => {
                    assert_eq!(*after_ms, 15_000);
                    Some(id.raw())
                }
                _ => None,
            })
            .collect();
        assert_eq!(timer_ids.len(), 1);
        assert!(
            timers.insert(timer_ids[0]),
            "timer must belong to exactly one message"
        );
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Emit)));
        let duplicate = h.step(
            port_reply_with_body(corr, bytes::Bytes::from_static(b"[]")),
            5001,
        );
        assert!(
            duplicate.is_empty(),
            "duplicate reply must not resend or rearm"
        );
        h.step(timer_tick(timer_ids[0]), 20000);
        assert!(failed_projection_for_tmp(&h, &temporary_id).is_some());
    }
    assert_eq!(timers.len(), 1024);
}

/// WS 断线不能取消已成功落库、仍等待本地读回的 HTTP 发送。
#[test]
fn disconnected_send_readback_still_sends() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    attach_empty_send_timeline(&mut h, 1);
    let initial = h.step(send_message_cmd(1, "disconnect_a", "hello"), 1000);
    let readback = acknowledge_optimistic_persist(&mut h, &initial, 1001);
    h.step(
        helix_core::Tick::Disconnected(helix_core::effect::TransportId::from_raw(0)),
        1002,
    );
    let effects = h.step(
        port_reply_with_body(p1_corr(&readback), bytes::Bytes::from_static(b"[]")),
        1003,
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1
    );
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, EffectKind::Emit)));
}

/// 身份切换必须撤销旧账号读回，即使之前经历过 transport reset。
#[test]
fn identity_change_discards_deferred_send_readback() {
    let mut h = Harness::new_with_identity(&[1], "send-viewer", "send-company", "", "", "");
    attach_empty_send_timeline(&mut h, 1);
    let initial = h.step(send_message_cmd(1, "identity_a", "hello"), 1000);
    let readback = acknowledge_optimistic_persist(&mut h, &initial, 1001);
    h.step(
        helix_core::Tick::Disconnected(helix_core::effect::TransportId::from_raw(0)),
        1002,
    );
    h.step(
        helix_core::Tick::Command(helix_core::tick::AppCommand::new(
            helix_im::module::RUNTIME_IDENTITY_COMMAND,
            serde_json::to_vec(
                &serde_json::json!({"auth_user_id": "other-actor", "company_id": "other-tenant"}),
            )
            .unwrap(),
        )),
        1003,
    );
    let effects = h.step(
        port_reply_with_body(p1_corr(&readback), bytes::Bytes::from_static(b"[]")),
        1004,
    );
    assert!(
        effects.is_empty(),
        "old actor must not send or publish into the new session"
    );
}

/// 自端回声已经确认发送，缺少前序频道事件不能让发送终态永久依赖 gate 补齐。
#[test]
fn self_echo_with_channel_gap_delivers_durable_send_terminal() {
    let mut h = Harness::new_with_identity(&[1], "user_1", "company", "", "", "");
    let initial = h.step(send_message_cmd(1, "tmp-2", "message-2"), 1000);
    let sending = acknowledge_optimistic_persist(&mut h, &initial, 1001);
    let timer = sending
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .unwrap();
    let echo = h.step(inbound_event(1, 2, 1), 1002);
    assert!(echo.contains(&EffectKind::CancelTimer(timer)));
    assert_eq!(h.module.pending_send_count(), 0);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert!(
        !echo.contains(&EffectKind::Emit),
        "durable terminal waits for its persist"
    );
    let terminal = h.step(port_reply_ok(p1_corr(&echo)), 1003);
    assert!(terminal.contains(&EffectKind::Emit), "timer cancellation must have an independent durable terminal continuation, even while seq=1 is missing");
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(0),
        "send acknowledgement must not advance the channel cursor"
    );
}

/// 生成本端权威回声；不通过内部状态捷径模拟网络输入。
fn send_echo_frame(channel: u64, temporary_id: &str) -> helix_core::Tick {
    let frame = serde_json::json!({"action":"post","data":{
        "id":sid(channel),"temporaryId":temporary_id,"channelId":cid(channel),
        "event_seq":2,"userId":"user_1","type":"TEXT","message":"confirmed","createAt":1000
    }});
    helix_core::Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&frame).unwrap().into(),
    ))
}

/// 多频道确认逆序交付时，每条发送独立释放终态，重复回声/回执/超时均不重复结算。
#[test]
fn concurrent_gap_echo_terminals_are_independent_and_idempotent() {
    let channels: Vec<u64> = (1..=256).collect();
    let mut h = Harness::new_with_identity(&channels, "user_1", "company", "", "", "");
    let mut sends = Vec::new();
    for channel in channels {
        let temporary_id = format!("concurrent-{channel}");
        let initial = h.step(send_message_cmd(channel, &temporary_id, "confirmed"), 1000);
        let sending = acknowledge_optimistic_persist(&mut h, &initial, 1001);
        let timer = sending
            .iter()
            .find_map(|e| match e {
                EffectKind::ScheduleTimer(id) => Some(*id),
                _ => None,
            })
            .unwrap();
        sends.push((channel, temporary_id, timer));
    }
    for (channel, temporary_id, timer) in sends.into_iter().rev() {
        let echo = h.step(send_echo_frame(channel, &temporary_id), 1002);
        assert!(echo.contains(&EffectKind::CancelTimer(timer)));
        let corr = p1_corr(&echo);
        h.step(port_reply_ok(corr), 1003);
        let emitted = emit_jsons(&h.sink);
        assert_eq!(
            emitted
                .iter()
                .filter(|v| v["event"] == "im:post:sent"
                    && v["data"]["temporaryId"] == temporary_id
                    && v["data"]["sendStatus"] == "sent")
                .count(),
            1
        );
        assert!(!h
            .step(port_reply_ok(corr), 1004)
            .contains(&EffectKind::Emit));
        assert!(!h.step(timer_tick(timer), 16002).contains(&EffectKind::Emit));
        assert!(!h
            .step(send_echo_frame(channel, &temporary_id), 16003)
            .contains(&EffectKind::Emit));
        assert_eq!(h.module.cursor_for(channel_id(channel)), Some(0));
    }
    assert_eq!(h.module.pending_send_count(), 0);
}

/// 发送确认存储失败时不得发布伪造的 durable sent；失败不能被算作成功验收。
#[test]
fn gap_echo_persist_error_never_emits_sent() {
    let mut h = Harness::new_with_identity(&[1], "user_1", "company", "", "", "");
    let initial = h.step(send_message_cmd(1, "tmp-2", "hello"), 1000);
    acknowledge_optimistic_persist(&mut h, &initial, 1001);
    let echo = h.step(inbound_event(1, 2, 1), 1002);
    h.step(port_reply_err(p1_corr(&echo)), 1003);
    assert!(!emit_jsons(&h.sink)
        .iter()
        .any(|v| v["event"] == "im:post:sent"));
}

/// 只读取真实 Module 产出的事件 bytes，不在测试中构造期望业务投影。
fn emit_jsons(sink: &helix_core::EffectSink) -> Vec<Value> {
    sink.as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(serde_json::from_slice(&event.0).expect("valid event")),
            _ => None,
        })
        .collect()
}

/// 关联 ID 只随本端已持久确认的发送终态释放，失败和重复回声均不能制造成功。
#[test]
fn send_request_id_waits_for_durable_echo() {
    for persist_ok in [true, false] {
        let mut h = Harness::new_with_identity(&[1], "user_1", "company", "", "", "");
        let initial = h.step(
            send_message_cmd_json(serde_json::json!({
                "channel_id": cid(1), "temporary_id": "recent-forward-send", "text": "forwarded",
                "req_id": "recent-forward-request"
            })),
            1000,
        );
        acknowledge_optimistic_persist(&mut h, &initial, 1001);
        assert!(emit_jsons(&h.sink)
            .iter()
            .all(|v| v["data"].get("requestId").is_none()));
        let echo = h.step(send_echo_frame(1, "recent-forward-send"), 1002);
        assert!(!echo.contains(&EffectKind::Emit));
        let corr = p1_corr(&echo);
        h.step(
            if persist_ok {
                port_reply_ok(corr)
            } else {
                port_reply_err(corr)
            },
            1003,
        );
        let terminals = emit_jsons(&h.sink);
        if persist_ok {
            let sent = terminals
                .iter()
                .find(|v| v["event"] == "im:post:sent")
                .unwrap();
            assert_eq!(sent["data"]["requestId"], "recent-forward-request");
        } else {
            assert!(terminals.iter().all(|v| v["event"] != "im:post:sent"));
        }
        h.step(send_echo_frame(1, "recent-forward-send"), 1004);
        assert!(emit_jsons(&h.sink)
            .iter()
            .all(|v| v["data"].get("requestId").is_none()));
    }
}

/// 构造服务端权威 `post` 广播帧（群邀请 NOTICE / 他人消息），用于注入服务端时钟下界。
fn server_post_frame(
    channel: u64,
    server_id_fixture: u64,
    event_seq: u64,
    msg_type: &str,
    create_at: i64,
) -> helix_core::Tick {
    let frame = serde_json::json!({"action":"post","data":{
        "id":sid(server_id_fixture),
        "channelId":cid(channel),
        "event_seq":event_seq,
        "userId":"other_user",
        "type":msg_type,
        "message":"invite notice",
        "createAt":create_at
    }});
    helix_core::Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&frame).unwrap().into(),
    ))
}

/// 读取指定 temporaryId 的乐观落库行写入的 `create_at`，作为客户端排序键的直接证据。
fn optimistic_create_at(harness: &Harness, temporary_id: &str) -> i64 {
    for effect in harness.sink.as_slice() {
        let Effect::Persist { ops, .. } = effect else {
            continue;
        };
        for op in ops {
            let StorageOp::BatchUpsert(spec) = op else {
                continue;
            };
            if spec.table != "message" {
                continue;
            }
            for row in &spec.rows {
                let value = |column: &str| {
                    row.iter()
                        .find(|(name, _)| name == column)
                        .map(|(_, value)| value.clone())
                };
                let matches_temporary_id = matches!(
                    value("temporary_id"),
                    Some(SqlValue::Text(ref stored)) if stored == temporary_id
                );
                if !matches_temporary_id {
                    continue;
                }
                match value("create_at") {
                    Some(SqlValue::Integer(create_at)) => return create_at,
                    other => panic!("optimistic row must persist an integer create_at: {other:?}"),
                }
            }
        }
    }
    panic!("no optimistic message persist for {temporary_id}");
}

/// 回归锚: 客户端时钟落后服务端时，本地乐观消息的 `createAt` 必须跟随已观测的服务端
/// 时间轴，否则刚发出的消息会排到更早的入群 NOTICE 之前（“邀请通知出现在首条消息下方”），
/// 并可能被 `ORDER BY create_at DESC LIMIT` 的 latest 窗口挤掉。
#[test]
fn optimistic_send_create_at_follows_observed_server_timeline() {
    const NOTICE_AT: i64 = 1_700_000_200_000;
    // 客户端时钟比服务端慢 2500ms：真实现场量得的偏差量级。
    const LAGGING_CLIENT_AT: u64 = 1_700_000_197_500;
    let mut h = Harness::new_with_identity(&[1], "user_1", "company", "", "", "");
    // 服务端在 NOTICE_AT 落库并广播群邀请 NOTICE（客户端本地时间是 LAGGING_CLIENT_AT）。
    h.step(
        server_post_frame(1, 9, 8, "NOTICE", NOTICE_AT),
        LAGGING_CLIENT_AT,
    );
    let effects = h.step(
        send_message_cmd(1, "tmp-lagging-clock", "first"),
        LAGGING_CLIENT_AT,
    );
    assert!(
        effects.iter().any(|effect| matches!(effect, EffectKind::Persist(_))),
        "ordinary send must still persist the optimistic row first"
    );
    let create_at = optimistic_create_at(&h, "tmp-lagging-clock");
    assert!(
        create_at > NOTICE_AT,
        "optimistic createAt {create_at} must follow observed server createAt {NOTICE_AT}"
    );
}
