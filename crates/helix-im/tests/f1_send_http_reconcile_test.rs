//! F1 发消息核心对账 bug 回归测试（UC-1.1 阻断）。
//!
//! 回归锚: F1（send HTTP 200 兜底对账）
//!
//! ## 根因
//!
//! 真 Go 对**发送者自身连接不推 `post` WS 回声**（self-omit，实测 4/4 会话 0 echo）→ 乐观行
//! 永停 `Sending` 到 15s 超时 spinner。消息**真落库了**（HTTP 200 `{status:SUCCESS}`），但旧码
//! `handle_send_message` 分配 `h1_corr` 当 `posts/create` 的 Http effect corr 却**从不 insert 进
//! `corr_map`** → HTTP 200 回来 `corr_map.remove(h1_corr)` 返 None → **响应被静默丢弃**。
//!
//! ## 修复
//!
//! send 时把 `h1_corr` 注册 `CorrelationContext::OutboundSendHttp{temporary_id}`；HTTP 200
//! `{status:SUCCESS}` 兜底标 `Sent` + 持久行 `status=sent`（不单押 WS echo）。WS echo 路径
//! 仍存（幂等，按 temporary_id ON CONFLICT 对账，后到覆盖同值）。

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

// ─── 正路：HTTP 200 {status:SUCCESS} 兜底标 Sent（无需 WS echo）────────────────

#[test]
fn send_http_success_advances_to_sent_without_ws_echo() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // P1 已确认并启动 H1，但 HTTP 结果尚未回来。
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sending),
        "before HTTP reply, pending send must NOT be Sent"
    );

    // 真 Go self-omit：发送者自身连接无 `post` WS 回声 —— 只回 HTTP 200。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": { "id": sid(7) } }));
    let reply_effects = h.step(reply, 1100);

    // 兜底对账（不依赖 WS echo）：吐 Persist(status=sent) + CancelTimer（撤 15s spinner），并清理
    // 在途 PendingSend（已对账 → 推进 Sent → remove，对齐 echo 路径防泄漏）。残留 None = 已对账，
    // 反证旧 bug 的「永停 Sending」（旧码 h1_corr 从不入 corr_map → 此 reply 被静默丢弃 → pending 残留）。
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "HTTP 200 SUCCESS should reconcile via Persist (status=sent), got: {:?}",
        reply_effects
    );
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(_))),
        "HTTP 200 SUCCESS should cancel 15s timeout (no spinner), got: {:?}",
        reply_effects
    );
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        None,
        "after HTTP 200 reconcile, in-flight send must be cleared (not stuck Sending)"
    );
}

// ─── HTTP 200 含 server id → 吐 Persist(status=sent + id=serverId)─────────────

#[test]
fn send_http_success_with_server_id_persists_sent_row() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": { "id": sid(7) } }));
    let reply_effects = h.step(reply, 1100);

    // 含 serverId → 走 reconcile：吐 Persist（覆写 id + status=sent）+ CancelTimer。
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "HTTP 200 with server id should emit Persist (sent row), got: {:?}",
        reply_effects
    );
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(_))),
        "HTTP 200 reconcile should cancel 15s timeout timer, got: {:?}",
        reply_effects
    );
}

// ─── HTTP 200 SUCCESS 但无 serverId → 仍标 Sent（serverId 后续补）──────────────

#[test]
fn send_http_success_without_server_id_still_marks_sent() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // SUCCESS 但 data 无可解 id（防御性取 data.id/data.post.id/id 全 miss）。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": {} }));
    let reply_effects = h.step(reply, 1100);

    // 仍兜底标 Sent：吐 Persist(status=sent，不覆写 id) + CancelTimer。serverId 后续 WS echo
    // 按 temporary_id 幂等补 → **pending_sends 保留**（不清栈），待 echo 落 id 后由
    // reconcile_post_echo 清栈。（fix/im-send-echo-reconcile：旧实现此处 remove → echo 到达
    // contains_key=false → server_id 永不落 id 列；改为保留以让 echo 补 id。）
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "SUCCESS without serverId should still persist sent row, got: {:?}",
        reply_effects
    );
    assert!(
        reply_effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(_))),
        "SUCCESS without serverId should still cancel 15s timeout, got: {:?}",
        reply_effects
    );
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sent),
        "SUCCESS without serverId marks Sent but RETAINS pending (awaiting WS echo to backfill id)"
    );
}

// ─── 根因回归：HTTP 200 无 serverId → 后到 WS echo 补 server_id 到乐观行 id 列 ────
//
// fix/im-send-echo-reconcile：cses post 经 Pulsar 异步建，posts/create HTTP 200 体仅 `{status}`
// 无 server id → 旧实现 mark Sent 后**无条件 remove pending_sends** → 携带 server id 的 WS
// `post` echo 到达时 `reconcile_post_echo` 因 contains_key=false 提前 return → server_id 永不落
// 乐观行 id 列（9 个 send 类 spec 红根因）。修复后保留 pending_sends 让 echo 幂等补 id。

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

#[test]
fn send_http_no_server_id_then_ws_echo_backfills_id() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // HTTP 200 SUCCESS 但无 server id（cses Pulsar 异步建）→ 标 Sent + 保留 pending（修复点）。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": {} }));
    let _ = h.step(reply, 1100);
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Sent),
        "no-serverId SUCCESS must RETAIN pending (else echo can't backfill id)"
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

// ─── 幂等：HTTP 兜底 + 后到 WS echo 不冲突（两者同值对账）──────────────────────

#[test]
fn send_http_then_late_ws_echo_idempotent() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = start_posts_create(&mut h, &initial_effects);

    // HTTP 兜底先到 → Sent + 移除 pending（reconcile 清理）。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": { "id": sid(7) } }));
    h.step(reply, 1100);

    // 此刻 pending 已被 reconcile 清理（已对账 → 不再残留 → 15s timer 即便 fire 也幂等无害）。
    // 后到的 WS echo 命中不到在途 send → no-op，不 panic、不重复推进（幂等）。
    let echo = inbound_event(1, 1, 0); // 通用 inbound（非本端 echo 字段）→ 不应破坏已对账态
    let _ = h.step(echo, 1200);

    // 终态仍一致：不再有 tmp_f1 的在途 send（已清理），状态机不崩。
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        None,
        "after HTTP reconcile + late echo, pending send should be cleared (idempotent)"
    );
}
