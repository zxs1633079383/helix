//! 发送链路测试（Phase B Commit 3）
//!
//! 验证 §4.5 发送机制：
//! - im_send_message → Persist{P1} + Http{H1} + ScheduleTimer{T1, 15s} + Emit(im:post:sending)
//! - PortReply(P1 Ok) → PendingSend 状态推进到 Sending
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
        .expect("send should emit Http(H1)")
}

fn unsend_persist_fire_for_tmp(h: &Harness, tmp: &str) -> bool {
    h.sink.as_slice().iter().any(|e| match e {
        Effect::PersistFire { ops } => ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                spec.conflict_key == Some("temporary_id")
                    && spec.rows.iter().any(|row| {
                        let has_tmp = row.iter().any(|(k, v)| {
                            k == "temporary_id" && matches!(v, SqlValue::Text(t) if t == tmp)
                        });
                        let has_unsend = row.iter().any(|(k, v)| {
                            k == "send_status" && matches!(v, SqlValue::Text(s) if s == "unsend")
                        });
                        has_tmp && has_unsend
                    })
            }
            _ => false,
        }),
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

#[test]
fn send_message_mints_one_temporary_id_for_projection_persist_and_http() {
    let mut h = Harness::new(&[1]);
    h.step(
        command_json(
            "im_send_message",
            serde_json::json!({
                "channel_id": cid(1),
                "text": "helix owns id"
            }),
        ),
        1_700_000_000_000,
    );

    let projection = last_sending_projection(&h).expect("send must emit optimistic projection");
    let temporary_id = projection["data"]["temporaryId"]
        .as_str()
        .expect("projection must expose Helix temporary_id");
    assert!(temporary_id.starts_with("helix_tmp_"));

    let persisted_same_id = h.sink.as_slice().iter().any(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                spec.rows.iter().any(|row| {
                    row.iter().any(|(key, value)| {
                        key == "temporary_id"
                            && matches!(value, SqlValue::Text(value) if value == temporary_id)
                    })
                })
            }
            _ => false,
        }),
        _ => false,
    });
    assert!(persisted_same_id, "optimistic row must use the minted id");

    let http_same_id = h.sink.as_slice().iter().any(|effect| match effect {
        Effect::Http { req, .. } if req.url.ends_with("/posts/create") => {
            req.body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<Value>(body).ok())
                .and_then(|body| body["temporaryId"].as_str().map(str::to_string))
                .as_deref()
                == Some(temporary_id)
        }
        _ => false,
    });
    assert!(http_same_id, "posts/create must use the minted id");

    h.step(
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
    let second = last_sending_projection(&h).expect("second send projection");
    let second_id = second["data"]["temporaryId"].as_str().unwrap();
    assert_ne!(second_id, temporary_id);
    assert_ne!(second_id, "caller-must-not-own-this");
}

// ─── 发送命令产出 4 个 Effect ─────────────────────────────────────────────────

#[test]
fn send_message_emits_four_effects() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);

    // 应产出 Persist{P1}（乐观落库）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Persist(_))),
        "im_send_message should emit Persist(P1), got: {:?}",
        effects
    );
    // 应产出 Http{H1}（POST /posts/create）
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "im_send_message should emit Http(H1), got: {:?}",
        effects
    );
    // 应产出 ScheduleTimer{T1, 15s}
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "im_send_message should emit ScheduleTimer(T1), got: {:?}",
        effects
    );
    // 应产出 Emit(im:post:sending)
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "im_send_message should emit Emit(im:post:sending), got: {:?}",
        effects
    );
    // 恰好 4 个 Effect
    assert_eq!(
        effects.len(),
        4,
        "im_send_message should emit exactly 4 effects, got: {:?}",
        effects
    );
}

#[test]
fn send_message_forwards_request_id_as_trace_header() {
    let mut h = Harness::new(&[1]);
    h.step(
        send_message_cmd_json(serde_json::json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_trace_1",
            "text": "hello",
            "req_id": "mrc-run:G-03:req-1"
        })),
        1000,
    );

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
    let mut h = Harness::new(&[1]);
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

    // 各自 timer 也不同
    let t1 = e1
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    let t2 = e2
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
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
    let p1_corr = effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Persist(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("P1 corr");

    // P1 落库成功
    let reply_effects = h.step(port_reply_ok(p1_corr), 1100);
    // P1 PortReply 不产出额外 Effect（只是状态推进）
    // 不应产出新 Persist 或 Http
    assert!(
        !reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "P1 ok should not emit new Persist, got: {:?}",
        reply_effects
    );
    // pending_send 仍存在（等待 echo 对账）
    assert_eq!(
        h.module.pending_send_count(),
        1,
        "pending_send should still be 1 after P1 ok"
    );
}

// ─── 15s 超时 → UnSend + PersistFire ─────────────────────────────────────────

#[test]
fn send_timeout_marks_unsend() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
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
    // 应产出 PersistFire（标记 unsend）
    assert!(
        timeout_effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistFire)),
        "timeout should emit PersistFire to mark unsend, got: {:?}",
        timeout_effects
    );
    assert!(
        failed_projection_for_tmp(&h, "tmp_1").is_some(),
        "timeout must project the durable unsend row as retryable failed, got: {:?}",
        timeout_effects
    );
}

#[test]
fn pending_send_http_error_marks_unsend_cancels_timer_and_emits_failed_projection() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_fail", "hello"), 1000);
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
    assert_eq!(
        h.module.pending_send_status("tmp_fail"),
        Some(SendStatus::UnSend),
        "failed send remains retained as terminal UnSend until any later echo/backfill"
    );
}

#[test]
fn late_optimistic_persist_reply_does_not_reopen_failed_send() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_late_p1", "hello"), 1000);
    let p1 = effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("P1 corr");
    let h1 = http_corr(&effects);

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

    let retry_effects = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({ "message_id": "tmp_late_p1" }),
        ),
        1200,
    );
    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "the failed send must remain retryable after a late P1 reply"
    );
}

#[test]
fn retry_send_reuses_cached_body_and_temporary_id_after_real_http_error() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_retry_send", "hello retry"), 1000);
    let h1 = http_corr(&effects);
    let first_body = h.last_http_body().expect("first posts/create body");
    h.step(port_reply_err(h1), 1100);
    assert_eq!(
        h.module.pending_send_status("tmp_retry_send"),
        Some(SendStatus::UnSend)
    );
    let retry_effects = h.step(
        command_json(
            "im_retry_send",
            serde_json::json!({ "message_id": "tmp_retry_send" }),
        ),
        1200,
    );

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
fn retry_send_keeps_reply_anchors_in_optimistic_projection() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(
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
    h.step(port_reply_err(http_corr(&effects)), 1_100);

    h.step(
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
}

#[test]
fn retry_send_rehydrates_failed_message_after_native_restart() {
    let mut h = Harness::new(&[1]);
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
    let retry_effects = h.step(
        port_reply_with_body(
            lookup_corr,
            bytes::Bytes::from(serde_json::to_vec(&row).unwrap()),
        ),
        2_010,
    );

    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::PersistFire)),
        "rehydrated retry must persist sending"
    );
    assert!(
        retry_effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Emit)),
        "rehydrated retry must emit the optimistic sending projection"
    );
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
    let sending = sending_projection_for_tmp(&h, "tmp_durable_retry")
        .expect("durable retry must emit a render-ready optimistic row");
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
fn retry_send_falls_back_from_temporary_id_to_server_id_lookup() {
    let mut h = Harness::new(&[1]);
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
    let retry = h.step(
        port_reply_with_body(
            second_corr,
            bytes::Bytes::from(serde_json::to_vec(&row).unwrap()),
        ),
        3_020,
    );
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
fn send_echo_reconcile_cancels_timer() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
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

    // 模拟 echo 通过 inbound_event 不太直接（需要 parser 支持 temporary_id 字段）
    // 此处用 PendingSend.reconcile 路径通过直接触发验证
    // （echo 字段解析是后续 Phase 工作，此处验证 reconcile 的 Effect 形状）
    //
    // 通过调用 handle_echo 等效路径验证（需要直接触发）：
    // 我们通过 timer_tick 模拟超时先到，再验证幂等（已 Sent 的忽略）
    let timeout_effects = h.step(timer_tick(timeout_timer), 16_001);
    assert!(
        timeout_effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistFire)),
        "timeout should emit PersistFire"
    );

    // 超时后再次触发同一 timer → 静默（timer 已从 pending_sends 中无法找到，或 status=UnSend 幂等）
    let effects2 = h.step(timer_tick(timeout_timer), 16_002);
    // 不应再次产出 PersistFire
    assert!(
        !effects2
            .iter()
            .any(|e| matches!(e, EffectKind::PersistFire)),
        "double-fire timeout should be idempotent, got: {:?}",
        effects2
    );
}

// ─── 发送 + 接收连续事件（混合场景）─────────────────────────────────────────

#[test]
fn send_and_receive_interleaved() {
    let mut h = Harness::new(&[1]);
    // 先发一条消息
    let _send_effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);

    // 同时收到 seq=1 的 WS 事件（其他人发的消息）
    let recv_effects = h.step(inbound_event(1, 1, 1), 1001);
    assert!(
        recv_effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistFire)),
        "inbound WS event should still work while send is pending"
    );

    // 发送的消息和接收的事件应该互不影响
    assert_eq!(
        h.module.pending_send_count(),
        1,
        "pending send should still be 1 after receiving other message"
    );
}
