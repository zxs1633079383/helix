//! posts/create HTTP admission 与 WS 权威对账回归测试。
//!
//! 已确认协议：HTTP 不返回可用于业务结算的 success/post.id。任何 HTTP body 都只代表
//! transport admission；sent、post.id 与 timer 取消只由权威 WS post echo 推进。
//!
//! admission 不得发起任何历史回读：echo 缺失时保留 15s 终态（UnSend），
//! 后续 seq gap 由 sync notify 补偿，而不是让发送路径指数退避轮询 getLatestPost。

mod common;
use common::*;

use helix_core::Tick;
use helix_im::state::SendStatus;
use serde_json::json;

/// 构造 `posts/create` HTTP 响应（裸 Go body → ADR-007 信封）。
fn posts_create_reply(corr_raw: u64, raw_go_body: serde_json::Value) -> Tick {
    let raw = bytes::Bytes::from(serde_json::to_vec(&raw_go_body).unwrap());
    port_reply_with_body(corr_raw, enveloped(raw))
}

/// P1 durable acknowledgement 之后产出的 Http(H1) corr（posts/create）。
fn http_corr(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|e| {
            if let EffectKind::Http(c) = e {
                Some(*c)
            } else {
                None
            }
        })
        .expect("P1 acknowledgement should emit Http(H1)")
}

/// 普通文本的真实顺序是 Command → P1 Persist 回执 → posts/create。Harness 没有
/// attached timeline，因此 P1 回执直接启动 HTTP；带 surface 的顺序额外由 local-first
/// Frame 回放测试守住。
fn start_posts_create(harness: &mut Harness, initial_effects: &[EffectKind]) -> u64 {
    let p1_corr = initial_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("ordinary send must begin with P1 persist");
    let post_p1_effects = harness.step(port_reply_ok(p1_corr), 1_001);
    http_corr(&post_p1_effects)
}

/// 断言 HTTP admission 不发起任何读回、也不提前结算，只等待权威 WS echo。
fn assert_admission_only_waits(harness: &Harness, effects: &[EffectKind]) {
    use helix_core::effect::Effect;

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_) | EffectKind::Persist(_))),
        "admission must not start a readback or settle locally"
    );
    assert!(
        !harness.sink.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Http { req, .. } if req.url.ends_with("/posts/getLatestPost")
        )),
        "admission must never poll getLatestPost for send settlement"
    );
}

/// HTTP admission 在权威 echo 回来前必须保持 Sending，且不发起读回。
#[test]
fn send_http_success_remains_sending_until_ws_echo() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // P1 已确认并启动 H1，但 HTTP 结果尚未回来。
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending),
        "before HTTP reply, pending send must NOT be Sent"
    );

    // 即使测试注入旧式 SUCCESS/id 形状，也不得把 transport 回包当 authority。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": { "id": sid(7) } }));
    let reply_effects = h.step(reply, 1100);

    assert_admission_only_waits(&h, &reply_effects);
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending),
        "HTTP admission cannot replace the authoritative WS echo"
    );
}

/// HTTP body 中的 id 不是服务端 Post 权威主键来源。
#[test]
fn send_http_body_server_id_is_not_authoritative() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": { "id": sid(7) } }));
    let reply_effects = h.step(reply, 1100);

    assert_admission_only_waits(&h, &reply_effects);
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending)
    );
}

/// 无 id 的正常 admission 保留 pending，只等待权威 WS echo。
#[test]
fn send_http_success_without_server_id_waits_for_ws_echo() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": {} }));
    let reply_effects = h.step(reply, 1100);

    assert_admission_only_waits(&h, &reply_effects);
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending),
        "admission retains pending until the authoritative WS echo arrives"
    );
}

/// 显式 retry 的幂等 HTTP admission 同样只代表 transport admission：不发起历史回读，
/// 由权威 WS echo 结算；echo 缺失时保留 15s 终态，交给后续 sync notify 补偿。
#[test]
fn retry_http_success_without_second_echo_waits_for_ws_echo() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_retry_readback", "hello"), 1_000);
    let h1 = start_posts_create(&mut h, &initial_effects);
    let timeout_timer = h
        .effect_log
        .last()
        .and_then(|effects| {
            effects.iter().find_map(|effect| match effect {
                EffectKind::ScheduleTimer(timer) => Some(*timer),
                _ => None,
            })
        })
        .expect("initial posts/create must schedule the send timeout");

    let admission = posts_create_reply(h1, json!({ "status": "SUCCESS" }));
    let admission_effects = h.step(admission, 1_100);
    assert_admission_only_waits(&h, &admission_effects);

    h.step(timer_tick(timeout_timer), 16_100);
    assert_eq!(
        h.module.pending_send_status("tmp_retry_readback"),
        Some(SendStatus::UnSend),
        "a missing echo must fall back to the 15s terminal instead of polling history"
    );

    let retry_effects = h.step(
        command_json(
            "im_retry_send",
            json!({"message_id":"tmp_retry_readback","req_id":"retry-readback"}),
        ),
        16_200,
    );
    let retry_persist = retry_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("retry must re-enter the optimistic persist barrier");
    let retry_after_persist = h.step(port_reply_ok(retry_persist), 16_201);
    let retry_http = http_corr(&retry_after_persist);

    h.sink.clear();
    let retry_admission = posts_create_reply(retry_http, json!({ "status": "SUCCESS" }));
    let retry_admission_effects = h.step(retry_admission, 16_202);
    assert_admission_only_waits(&h, &retry_admission_effects);
    assert_eq!(
        h.module.pending_send_status("tmp_retry_readback"),
        Some(SendStatus::Sending),
        "retry admission must wait for the authoritative WS echo"
    );

    let _ = h.step(echo_frame(1, 1, "tmp_retry_readback", 77), 16_203);
    assert_eq!(
        h.module.pending_send_status("tmp_retry_readback"),
        None,
        "WS echo must clear pending instead of allowing a second timeout"
    );
    assert_eq!(
        echo_persisted_id(&h, "tmp_retry_readback"),
        Some(sid(77)),
        "WS echo must persist the canonical server id"
    );
}

// ─── HTTP admission 后由 WS echo 补 server_id ────────────────────────────────

/// 构造真 Go `action=="post"` echo 帧（带 temporary_id + server id + event_seq）。
fn echo_frame(channel_id: u64, seq: u64, temporary_id: &str, server_id: u64) -> Tick {
    use helix_core::tick::InboundBytes;
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "id": sid(server_id),
            "temporary_id": temporary_id,
            "channelId": cid(channel_id),
            "event_seq": seq,
            "message": "hello",
        },
        "broadcast": {},
        "seq": 99
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&payload).unwrap().into()))
}

/// 从 sink 找 message 表 BatchUpsert（reconcile 覆写 id 用），返回 id 列值（若有）。
fn echo_persisted_id(h: &Harness, tmp: &str) -> Option<String> {
    use helix_core::effect::{Effect, SqlValue, StorageOp};
    h.sink.as_slice().iter().rev().find_map(|e| match e {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                let row = spec.rows.first()?;
                let keyed_tmp = row.iter().any(|(k, v)| {
                    k == "temporary_id" && matches!(v, SqlValue::Text(t) if t == tmp)
                });
                if !keyed_tmp {
                    return None;
                }
                row.iter().find_map(|(k, v)| match (k.as_str(), v) {
                    ("id", SqlValue::Text(t)) => Some(t.clone()),
                    _ => None,
                })
            }
            _ => None,
        }),
        _ => None,
    })
}

/// admission 后到达的连续 echo 必须补写 id 并清理 pending。
#[test]
fn send_http_no_server_id_then_ws_echo_backfills_id() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // HTTP admission 无 server id，保持 Sending 并保留 pending。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": {} }));
    let _ = h.step(reply, 1100);
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending),
        "HTTP admission must retain Sending pending for echo reconciliation"
    );

    // 携带 server id 的 WS echo 到达（event_seq=1 == cursor+1 连续）→ reconcile_post_echo 补 id。
    h.sink.clear();
    let _ = h.step(echo_frame(1, 1, "tmp_f1", 42), 1200);

    // 断言 ①：乐观行 id 被覆写为 server_id（temporary_id→server_id），证 reconcile 真落 id。
    assert_eq!(
        echo_persisted_id(&h, "tmp_f1"),
        Some(sid(42)),
        "WS echo must overwrite optimistic row id with server_id (the 9-spec root-cause fix)"
    );
    // 断言 ②：pending_sends 最终清空无泄漏（reconcile_post_echo 落定后 remove）。
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        None,
        "after echo reconcile, pending_sends must be cleared (no leak)"
    );
    assert_eq!(h.module.pending_send_count(), 0, "no residual pending send");
}

#[test]
fn send_http_no_server_id_then_gapped_echo_still_backfills_id() {
    // 证 path2（cursor gate）不挡 id 补写：echo seq=9 ≫ cursor(0)+1（gap）→ 消息行落库进 buffer
    // 等回补，但 reconcile_post_echo 在 gate 之前**无条件**跑 → id 仍按 temporary_id 幂等补写。
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_g", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);
    let _ = h.step(
        posts_create_reply(h1, json!({ "status": "SUCCESS", "data": {} })),
        1100,
    );

    h.sink.clear();
    let _ = h.step(echo_frame(1, 9, "tmp_g", 88), 1200); // seq=9 制造 cursor gap

    assert_eq!(
        echo_persisted_id(&h, "tmp_g"),
        Some(sid(88)),
        "gapped echo must STILL backfill id (reconcile ungated, runs before channel gate)"
    );
    assert_eq!(
        h.module.pending_send_status("tmp_g"),
        None,
        "gapped echo still clears pending (reconcile independent of cursor gap)"
    );
}

// ─── 反路：status != SUCCESS → 不标 Sent ──────────────────────────────────────

#[test]
fn send_http_non_success_does_not_advance_to_sent() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // 4xx body 误判防御：HTTP 层 200 但 body status != SUCCESS（边界零信任，不当成功）。
    let reply = posts_create_reply(h1, json!({ "status": "FAIL", "message": "rejected" }));
    h.step(reply, 1100);

    assert_ne!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sent),
        "non-SUCCESS body must NOT advance PendingSend to Sent"
    );
}

// ─── 反路：PortOutcome::Err → 不标 Sent ───────────────────────────────────────

#[test]
fn send_http_err_does_not_advance_to_sent() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    h.step(port_reply_err(h1), 1100);

    assert_ne!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sent),
        "HTTP Err must NOT advance PendingSend to Sent"
    );
}

/// HTTP admission 后的匹配 echo 按 temporaryId 唯一结算。
#[test]
fn send_http_then_late_ws_echo_idempotent() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // HTTP admission 先到，仍保留 Sending pending。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": { "id": sid(7) } }));
    h.step(reply, 1100);

    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending)
    );
    let _ = h.step(echo_frame(1, 1, "tmp_f1", 7), 1200);

    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        None,
        "matching WS echo is the authority that clears pending"
    );
}
