//! spec06 缺陷A 回归测试 — 读族 outbound HTTP 响应回灌前端（request-response 回灌）。
//!
//! 回归锚：spec06「06-读族回灌+完整投影」缺陷A。
//!
//! ## 被测不变量
//! 1. 读命令（`is_read=true`，posts_read 17 + user-misc 读 7）dispatch 后注册
//!    `CorrelationContext::OutboundReadReply{req_id}` → HTTP 回报时 emit `im:read:result{req_id,body}`
//!    **透传后端响应体**（后端返回什么吐什么）。
//! 2. 写命令（`is_read=false`）**不**注册回灌上下文 → HTTP 回报无 corr 命中 → 静默 no-op（数据走 WS 回声）。
//! 3. HTTP 失败 / 信封畸形 → emit `im:read:result{req_id,error}`（不丢 req_id 关联，前端 reject）。
//!
//! ## 可证伪性（HX-C011）
//! `read_command_registers_relay_and_emits_result` 依赖 `module.rs` dispatch 的
//! `corr_map.insert(OutboundReadReply)`：**删该行** → PortReply 查不到上下文 → 不 emit → 本测试 FAIL
//! （正是缺陷A 现状：响应静默丢弃，前端永远拿不到数据）。

mod common;
use common::*;

use helix_core::tick::AppCommand;
use helix_core::{Effect, EffectSink, Tick};
use helix_im::commands;

// ─── 局部 helper ──────────────────────────────────────────────────────────────

/// 构造一条 outbound 读/写命令 Tick（任意命令名 + JSON payload）。
fn outbound_cmd(name: &'static str, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new(name, bytes))
}

/// 从 sink 取本批次第一条 `Effect::Http` 的 corr.raw()（读命令 dispatch 产单条 Http）。
fn http_corr(sink: &EffectSink) -> Option<u64> {
    sink.as_slice().iter().find_map(|e| match e {
        Effect::Http { corr, .. } => Some(corr.raw()),
        _ => None,
    })
}

/// 从 sink 取本批次第一条 `Effect::Emit` 的 payload（解为 JSON）。
fn emit_json(sink: &EffectSink) -> Option<serde_json::Value> {
    sink.as_slice().iter().find_map(|e| match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
        _ => None,
    })
}

// ─── 不变量 1：读命令注册回灌 + emit 透传 body ──────────────────────────────────

#[test]
fn read_command_registers_relay_and_emits_result() {
    // 前置：is_read 真源——top20 是读族。
    assert!(commands::is_read("im_top20"), "im_top20 必为读族（posts_read）");

    let mut h = Harness::new(&[]);

    // ① 喂读命令（带前端 bridge 注入的 req_id）。
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a", "req_id": "rq-1" }),
    );
    let kinds = h.step(cmd, 1000);
    // dispatch 产单条 Http；取出其 corr（不硬编码，鲁棒）。
    let corr = http_corr(&h.sink).expect("读命令 dispatch 必产 Effect::Http");
    assert!(
        kinds.iter().any(|k| matches!(k, EffectKind::Http(_))),
        "读命令产 Http"
    );

    // ② HTTP 回报：driver 回灌 ADR-007 信封 {status,headers,body:base64(raw_go_body)}。
    //    raw_go_body = 后端真实响应体（这里是 top20 的消息数组样例）。
    let raw_go_body = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status": "SUCCESS",
            "data": [ { "id": "m1", "message": "hi" }, { "id": "m2", "message": "yo" } ]
        }))
        .unwrap(),
    );
    let reply = port_reply_with_body(corr, enveloped(raw_go_body.clone()));
    h.step(reply, 1001);

    // ③ 断言 emit im:read:result{req_id, body}——body = 后端响应体**原样透传**。
    let v = emit_json(&h.sink).expect("读 HTTP Ok 必 emit im:read:result");
    assert_eq!(v["event"], "im:read:result", "channel 必为 im:read:result");
    assert_eq!(v["data"]["req_id"], "rq-1", "req_id 必原样回填");
    // 透传：body 结构 == 后端响应体（status + data 数组逐字）。
    assert_eq!(v["data"]["body"]["status"], "SUCCESS");
    assert_eq!(v["data"]["body"]["data"][0]["id"], "m1");
    assert_eq!(v["data"]["body"]["data"][1]["message"], "yo");
    // 无 error 字段（成功路径）。
    assert!(v["data"].get("error").is_none(), "成功路径不带 error");
}

// ─── 不变量 2：写命令不注册回灌（fire-and-forget）────────────────────────────────

#[test]
fn write_command_does_not_register_relay() {
    // 前置：revoke 是写族（撤回，数据更新走 WS 回声）。
    assert!(commands::is_outbound("im_revoke"), "im_revoke 必为 outbound");
    assert!(!commands::is_read("im_revoke"), "im_revoke 必非读族（写）");

    let mut h = Harness::new(&[]);

    // ① 喂写命令（即使带 req_id，写族也不注册回灌）。
    let cmd = outbound_cmd(
        "im_revoke",
        serde_json::json!({ "post_id": "p1", "req_id": "rq-w" }),
    );
    h.step(cmd, 1000);
    let corr = http_corr(&h.sink).expect("写命令 dispatch 必产 Effect::Http");

    // ② HTTP 回报：corr 未注册回灌上下文 → handle_port_reply 单次 remove 查不到 → 静默 no-op。
    let raw = bytes::Bytes::from(br#"{"status":"SUCCESS"}"#.to_vec());
    let kinds = h.step(port_reply_with_body(corr, enveloped(raw)), 1001);

    // ③ 断言：无任何 emit（写族 ack 可丢，数据走 WS 回声，不回灌）。
    assert!(
        !kinds.iter().any(|k| matches!(k, EffectKind::Emit)),
        "写命令 HTTP 回报不应 emit（fire-and-forget）"
    );
    assert!(
        emit_json(&h.sink).is_none(),
        "写命令 HTTP 回报零 Emit"
    );
}

// ─── 不变量 3：HTTP 失败 / 信封畸形 → emit error（不丢 req_id）─────────────────────

#[test]
fn read_http_error_emits_read_error() {
    let mut h = Harness::new(&[]);
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a", "req_id": "rq-e" }),
    );
    h.step(cmd, 1000);
    let corr = http_corr(&h.sink).expect("读命令产 Http");

    // PortReply Err（网络/HTTP 层失败）。
    let reply = Tick::PortReply {
        corr: helix_core::effect::Correlation::from_raw(corr),
        outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Network),
    };
    h.step(reply, 1001);

    let v = emit_json(&h.sink).expect("读 HTTP Err 必 emit im:read:result（带 error）");
    assert_eq!(v["event"], "im:read:result");
    assert_eq!(v["data"]["req_id"], "rq-e", "失败路径也回填 req_id（前端 reject）");
    assert!(v["data"].get("error").is_some(), "失败路径带 error 字段");
    assert!(v["data"].get("body").is_none(), "失败路径不带 body");
}

#[test]
fn read_malformed_envelope_emits_read_error() {
    let mut h = Harness::new(&[]);
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a", "req_id": "rq-m" }),
    );
    h.step(cmd, 1000);
    let corr = http_corr(&h.sink).expect("读命令产 Http");

    // Ok 但 body 不是合法 ADR-007 信封（缺 body 字段）→ unwrap_sync_envelope Err → read error。
    let bad = bytes::Bytes::from(br#"{"not":"an envelope"}"#.to_vec());
    h.step(port_reply_with_body(corr, bad), 1001);

    let v = emit_json(&h.sink).expect("信封畸形必 emit im:read:result（带 error，不丢 req_id）");
    assert_eq!(v["event"], "im:read:result");
    assert_eq!(v["data"]["req_id"], "rq-m");
    assert!(v["data"].get("error").is_some(), "信封畸形走 error 路径");
}

// ─── 不变量 4：缺 req_id 降级 fire-and-forget（不注册，不 panic）────────────────────

#[test]
fn read_command_without_req_id_degrades_to_fire_and_forget() {
    let mut h = Harness::new(&[]);
    // 读命令但前端未注入 req_id（非 bridge 调用）→ 仍 fire HTTP，但不注册回灌。
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a" }),
    );
    let kinds = h.step(cmd, 1000);
    assert!(
        kinds.iter().any(|k| matches!(k, EffectKind::Http(_))),
        "缺 req_id 仍 fire HTTP（命令本身不受 req_id 影响）"
    );
    let corr = http_corr(&h.sink).expect("读命令产 Http");

    // HTTP 回报：未注册 → 查不到上下文 → 无 emit（无 req_id 无从回灌，符合零信任降级）。
    let raw = bytes::Bytes::from(br#"{"status":"SUCCESS","data":[]}"#.to_vec());
    let after = h.step(port_reply_with_body(corr, enveloped(raw)), 1001);
    assert!(
        !after.iter().any(|k| matches!(k, EffectKind::Emit)),
        "缺 req_id 的读命令回报不回灌（无关联键）"
    );
}

// ─── is_read 注册表覆盖（读族标注正确性，防漂移）──────────────────────────────────

#[test]
fn is_read_marks_read_family_only() {
    // posts_read 17 全读。
    for name in [
        "im_get_schedule",
        "im_post_context",
        "im_top20",
        "im_get_posts",
        "im_get_posts_after_index",
        "im_get_replies",
        "im_get_reply_branch",
        "im_query_todo_list",
        "im_get_updated_posts",
        "im_get_latest_post",
        "im_bookmark_create",
        "im_bookmark_delete",
        "im_bookmark_load",
        "im_announcement_accept_list",
        "im_announcement_list",
        "im_announcement_detail",
        "im_post_read_list",
    ] {
        assert!(commands::is_read(name), "{name} 必为读族（posts_read）");
    }
    // user-misc 读族 7（不含 search_do）。
    for name in [
        "im_list_users",
        "im_get_user_statuses",
        "im_get_all_modules",
        "im_load_send_notifications",
        "im_search_post",
        "im_search_user",
        "im_search_channel",
    ] {
        assert!(commands::is_read(name), "{name} 必为读族（user-misc 读）");
    }
    // 写族 / 排除项 → 非读。
    for name in [
        "im_revoke",
        "im_urgent_post",
        "im_team_upsert",
        "im_team_member_add",
        "im_team_quit",
        "im_search_do", // spec06 §4：Go 未实现，唯一合法暂不切，标 false 不回灌
    ] {
        assert!(!commands::is_read(name), "{name} 必非读族");
    }
    // 未认领命令 → 非读（不 panic）。
    assert!(!commands::is_read("not_a_command"));
}
