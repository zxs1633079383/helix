//! F1-gap1 接管收尾：`im_reconnect` handler 臂 + `im_read_channel` 悬空别名收口（回放剧本）。
//!
//! 回归锚: F1/gap-1 —— accepts_tick allowlist 含 im_reconnect/im_read_channel 但 module.rs 无臂
//! → 落 unhandled no-op。本测试冻结收口后行为：
//!   ① `im_reconnect`（连接控制，04 表 B `IMWS:reconnect`）→ handle 吐恰一条
//!      `Effect::Emit{im:net:reconnect_requested, data:{}}` 控制信号（连接生命周期归 driver，
//!      helix-im sans-IO 只声明意图）。
//!   ② `im_read_channel` = `im_post_read` 悬空别名（行为真源 cses-client/src-tauri/src/features/im/
//!      无 read_channel 概念）→ 从 accepts_tick allowlist 删除，accepts 返回 false（不再误入泵）。
//!
//! 单元级断言（emit_reconnect_requested 字段集冻结）在 `tests/projection_schema_test.rs`；
//! 本文件验证 **dispatch 接线**：Tick::Command → accepts → handle → 正确 Effect / no-op。

mod common;
use common::*;

use helix_core::tick::AppCommand;
use helix_core::{Effect, Tick};
use helix_im::acl::from_tick::accepts_tick;
use helix_im::module::ImConfig;
use helix_im::ImModule;

/// 构造任意命令 Tick。
fn cmd(name: &'static str, payload: serde_json::Value) -> Tick {
    Tick::Command(AppCommand::new(name, serde_json::to_vec(&payload).unwrap()))
}

/// 取本批次第一条 `Effect::Emit` 的解析后 JSON（无 Emit → None）。
fn first_emit_json(h: &Harness) -> Option<serde_json::Value> {
    h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
        _ => None,
    })
}

// ════════════════════════════════════════════════════════════════════════════
// ① im_reconnect → emit im:net:reconnect_requested 控制信号
// ════════════════════════════════════════════════════════════════════════════

/// im_reconnect 被 accepts_tick 放行（真连接控制命令，有 handler 臂）。
#[test]
fn im_reconnect_is_accepted() {
    let tick = cmd("im_reconnect", serde_json::json!({}));
    assert!(
        accepts_tick(&tick),
        "im_reconnect 是真连接控制命令，accepts 必放行"
    );
}

/// im_reconnect → handle 吐恰一条 Emit{im:net:reconnect_requested, data:{}}。
#[test]
fn im_reconnect_emits_control_signal() {
    let mut h = Harness::new(&[1]);
    let kinds = h.step(cmd("im_reconnect", serde_json::json!({})), 1_000);

    // 恰一条 Effect，且是 Emit（控制信号，无 Persist/Http/Send）。
    assert_eq!(
        kinds.len(),
        1,
        "im_reconnect 应吐恰一条 Effect（控制 Emit）"
    );
    assert_eq!(kinds[0], EffectKind::Emit, "唯一 Effect 必为 Emit");

    let v = first_emit_json(&h).expect("应 emit 控制信号");
    assert_eq!(
        v["event"], "im:net:reconnect_requested",
        "channel 必为 im:net:reconnect_requested（连接控制族，对齐 im:net:offline）"
    );
    // data 为空对象 {}：无参控制信号，不携业务渲染字段。
    let data = v["data"].as_object().expect("data 必为对象");
    assert!(
        data.is_empty(),
        "reconnect 控制信号 data 必空（无参）：{data:?}"
    );
}

/// im_reconnect 不解析 payload：任意/缺字段 payload 都产同一控制信号（无参语义）。
#[test]
fn im_reconnect_ignores_payload() {
    let mut h = Harness::new(&[1]);
    // 带噪声 payload —— 不应改变控制信号形态。
    let kinds = h.step(
        cmd(
            "im_reconnect",
            serde_json::json!({ "noise": "ignored", "n": 42 }),
        ),
        2_000,
    );
    assert_eq!(
        kinds,
        vec![EffectKind::Emit],
        "噪声 payload 仍只产控制 Emit"
    );
    let v = first_emit_json(&h).expect("应 emit");
    assert_eq!(v["event"], "im:net:reconnect_requested");
    assert!(v["data"].as_object().unwrap().is_empty(), "data 仍空");
}

// ════════════════════════════════════════════════════════════════════════════
// ② im_read_channel = im_post_read 悬空别名 → accepts 拒绝（不再入泵 no-op）
// ════════════════════════════════════════════════════════════════════════════

/// im_read_channel 从 accepts_tick allowlist 删除 → 不再被放行（避免悬空命令落 unhandled no-op）。
#[test]
fn im_read_channel_no_longer_accepted() {
    let tick = cmd(
        "im_read_channel",
        serde_json::json!({ "channel_id": cid(1) }),
    );
    assert!(
        !accepts_tick(&tick),
        "im_read_channel 是 im_post_read 悬空别名，已删 allowlist，accepts 必拒"
    );
}

/// 行为真源：真实已读 outbound = im_post_read（仍被放行，走 is_outbound 分支）。
/// 收口 im_read_channel 不影响真已读路径。
#[test]
fn im_post_read_still_accepted() {
    let tick = cmd("im_post_read", serde_json::json!({ "channel_id": cid(1) }));
    assert!(
        accepts_tick(&tick),
        "im_post_read 是真已读 outbound（post/read），accepts 必放行"
    );
}

/// 端到端：即便有人绕过 accepts 直接喂 im_read_channel 进 handle（防御性），
/// 它落 `_ => debug` no-op（零 Effect）——不 panic、不误产 Effect。
#[test]
fn im_read_channel_handle_is_noop() {
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: String::new(),
        company_id: String::new(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
    };
    let mut module = ImModule::new(config);
    let mut sink = helix_core::EffectSink::new();
    {
        use helix_core::Module;
        module
            .handle(
                &cmd(
                    "im_read_channel",
                    serde_json::json!({ "channel_id": cid(1) }),
                ),
                1_000,
                &mut sink,
            )
            .expect("unhandled command 应 graceful no-op，不 Err");
    }
    assert!(
        sink.as_slice().is_empty(),
        "im_read_channel 无臂 → 零 Effect（graceful no-op）"
    );
}
