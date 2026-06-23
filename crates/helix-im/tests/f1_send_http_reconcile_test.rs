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

/// send 命令产出的 Http(H1) corr（posts/create）。Persist(P1) 先产，故 Http 是第二个 corr-bearing
/// effect；这里直接抠 Http corr。
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
        .expect("send should emit Http(H1)")
}

// ─── 正路：HTTP 200 {status:SUCCESS} 兜底标 Sent（无需 WS echo）────────────────

#[test]
fn send_http_success_advances_to_sent_without_ws_echo() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = http_corr(&effects);

    // 发送后乐观行 Local（P1 未回报）—— 旧 bug 下会永停于此到 15s 超时。
    assert_eq!(
        h.module.pending_send_status("tmp_f1"),
        Some(SendStatus::Local),
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
    let effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = http_corr(&effects);

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
    let effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = http_corr(&effects);

    // SUCCESS 但 data 无可解 id（防御性取 data.id/data.post.id/id 全 miss）。
    let reply = posts_create_reply(h1, json!({ "status": "SUCCESS", "data": {} }));
    let reply_effects = h.step(reply, 1100);

    // 仍兜底标 Sent：吐 Persist(status=sent，不覆写 id) + CancelTimer + 清 pending。serverId 后续
    // history/WS echo 按 temporary_id 幂等补。残留 None 反证已对账（非永停 Sending）。
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
        None,
        "SUCCESS without serverId should still reconcile + clear pending (not stuck Sending)"
    );
}

// ─── 反路：status != SUCCESS → 不标 Sent ──────────────────────────────────────

#[test]
fn send_http_non_success_does_not_advance_to_sent() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = http_corr(&effects);

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
    let effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = http_corr(&effects);

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
    let effects = h.step(send_message_cmd(1, "tmp_f1", "hello"), 1000);
    let h1 = http_corr(&effects);

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
