//! C3-core 验收：sync 驱动的 post:* mutation 事件 + im:__bus__ 信封。
//!
//! 回归锚: C3（sync-v2 完成 spec §3）
//!
//! ## 行为真源（cses-client post.rs:1146-1448 apply_event_with_messages）
//!
//! sync `events` 路径逐事件 dispatch，按 event_type + 是否命中 messages map（phantom 判定）
//! emit 对应 `im:*` 业务事件：
//! - type1 new：`msgId ∈ messages`（可见）→ im:post:received + im:channel:update-by-post（未读 +N）；
//!   `∉`（phantom）→ 不 emit（UI 不渲染，真源 1270-1280）。
//! - type2 edit：`msgId ∈ messages` → im:post:updated；phantom → noop（真源 1346）。
//! - type3 revoke：有 msgId → im:post:deleted（无须 body，真源 1402）。
//! - type6 read：有 msgId → im:channel:read_echo（A5 sync 应用路径，无须 body 恒 emit，真源 1428）。
//!
//! ## 铁律（HX-C008）
//!
//! emit 与落库同口径（可见才动 UI），但 emit **不卡 cursor**——cursor 仍由 persist_corr 的
//! PortReply Ok 独立推进到 commit_target（phantom 批也照推）。

mod common;
use common::*;

use helix_core::Effect;
use helix_im::acl::to_effect::sync_mutation_emits;
use helix_im::parser::parse_sync_response;
use helix_im::sync_session::SyncResponse;

// ─── 辅助：从 Emit Effect 取 event 名 ──────────────────────────────────────────

/// 提取 Emit Effect 的 `event` 字段（DomainEventBytes 是 `{event,data}` JSON）。
fn emit_event_name(e: &Effect) -> Option<String> {
    match e {
        Effect::Emit { event } => {
            let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            v.get("event").and_then(|s| s.as_str()).map(str::to_string)
        }
        _ => None,
    }
}

/// 从一批 Effect 取所有 emit 的 event 名（保序）。
fn emit_names(effects: &[Effect]) -> Vec<String> {
    effects.iter().filter_map(emit_event_name).collect()
}

fn parse_events_and_messages(
    bytes: &bytes::Bytes,
    channel_id: u64,
) -> (
    Vec<helix_im::sync_session::EventEnvelope>,
    std::collections::HashMap<String, helix_im::sync_session::PostFields>,
) {
    let ch = common::channel_id(channel_id);
    match parse_sync_response(bytes.as_ref(), ch).expect("parse sync response") {
        SyncResponse::Events {
            events, messages, ..
        } => (events, messages),
        other => panic!("expected Events, got {other:?}"),
    }
}

// ─── 1. type6 read：恒 emit im:channel:read_echo（A5：sync 应用路径专用事件名）──────────

#[test]
fn read_event_emits_channel_read_echo() {
    // 回归锚: A5 —— sync 应用的 type6 read 发 im:channel:read_echo（真源 post.rs:1428），
    // 与在线 post_read 的 im:post:read（post.rs:157/167）分流。
    let reply = sync_reply_events_with_messages(
        2,
        &[ev(20, 6, "msg_read")],
        &[("msg_read", "body", "1011")],
        20,
    );
    let (events, messages) = parse_events_and_messages(&reply, 2);
    let emits = sync_mutation_emits(&events, &messages);
    assert_eq!(emit_names(&emits), vec!["im:channel:read_echo"]);
}

/// type6 read 即便 msg 不在 messages map 也 emit（无须 body，真源 1413-1437 恒 emit）。
#[test]
fn read_event_emits_even_when_phantom() {
    let reply = sync_reply_events_with_messages(2, &[ev(21, 6, "msg_read_phantom")], &[], 21);
    let (events, messages) = parse_events_and_messages(&reply, 2);
    let emits = sync_mutation_emits(&events, &messages);
    assert_eq!(
        emit_names(&emits),
        vec!["im:channel:read_echo"],
        "sync read 无须 body，恒 emit im:channel:read_echo（A5）"
    );
}

// ─── 2. type2 edit：可见 emit updated；phantom noop ────────────────────────────

#[test]
fn edit_event_emits_post_updated_when_visible_noop_when_phantom() {
    let reply = sync_reply_events_with_messages(
        4,
        &[ev(40, 2, "msg_edited"), ev(41, 2, "msg_edit_phantom")],
        &[("msg_edited", "edited text", "")],
        41,
    );
    let (events, messages) = parse_events_and_messages(&reply, 4);
    let emits = sync_mutation_emits(&events, &messages);
    assert_eq!(
        emit_names(&emits),
        vec!["im:post:updated"],
        "可见 edit emit updated，phantom edit noop"
    );
}

// ─── 3. type3 revoke：恒 emit deleted（无须 body）──────────────────────────────

#[test]
fn revoke_event_emits_post_deleted_regardless_of_messages() {
    let reply = sync_reply_events_with_messages(3, &[ev(30, 3, "msg_gone")], &[], 30);
    let (events, messages) = parse_events_and_messages(&reply, 3);
    let emits = sync_mutation_emits(&events, &messages);
    assert_eq!(
        emit_names(&emits),
        vec!["im:post:deleted"],
        "revoke 无须 body，恒 emit deleted"
    );
}

// ─── 4. type1 new：可见 emit received + channel:update-by-post；phantom 不 emit ──

#[test]
fn new_visible_emits_received_and_channel_update() {
    let reply = sync_reply_events_with_messages(
        1,
        &[ev(10, 1, "msg_visible")],
        &[("msg_visible", "hello", "")],
        10,
    );
    let (events, messages) = parse_events_and_messages(&reply, 1);
    let emits = sync_mutation_emits(&events, &messages);
    assert_eq!(
        emit_names(&emits),
        vec!["im:post:received", "im:channel:update-by-post"],
        "可见 new emit received + 未读 +N 信号"
    );
}

#[test]
fn new_phantom_emits_nothing() {
    let reply = sync_reply_events_with_messages(1, &[ev(11, 1, "msg_phantom")], &[], 11);
    let (events, messages) = parse_events_and_messages(&reply, 1);
    let emits = sync_mutation_emits(&events, &messages);
    assert!(
        emit_names(&emits).is_empty(),
        "phantom new 不 emit（UI 不渲染），got {:?}",
        emit_names(&emits)
    );
}

// ─── 5. 混合批：分桶顺序 + phantom 混入 ───────────────────────────────────────

#[test]
fn mixed_batch_emits_match_dispatch_table() {
    // parser 分桶后顺序 type1, type1, type3, type6。
    let reply = sync_reply_events_with_messages(
        5,
        &[
            ev(50, 6, "m_new"),     // read（后处理）
            ev(51, 1, "m_new"),     // new 可见
            ev(52, 1, "m_phantom"), // new phantom
            ev(53, 3, "m_new"),     // revoke
        ],
        &[("m_new", "content", "0101")],
        53,
    );
    let (events, messages) = parse_events_and_messages(&reply, 5);
    // 分桶铁律：type1, type1, type3, type6。
    let types: Vec<u8> = events.iter().map(|e| e.kind.type_num()).collect();
    assert_eq!(types, vec![1, 1, 3, 6]);

    let emits = sync_mutation_emits(&events, &messages);
    // type1 可见 → received + update-by-post；type1 phantom → 无；type3 → deleted；
    // type6 → channel:read_echo（A5 sync 应用路径）。
    assert_eq!(
        emit_names(&emits),
        vec![
            "im:post:received",
            "im:channel:update-by-post",
            "im:post:deleted",
            "im:channel:read_echo",
        ]
    );
}

// ─── 5.1 在线 vs 离线 read 分流对照：同一 type6 read，两路径 emit 名必须不同 ──────────
//
// 回归锚: A5/F2 —— 钉死 read_echo 分流不变量，防再次回归。
// 离线 sync 应用路径（sync_mutation_emits）→ im:channel:read_echo；
// 在线 WS post_read 路径（gate emit_for_kind → emit_post_read）→ im:post:read。
// 两者命名分流是前端零改动契约（spec §0.3/§S9）的硬约束。
#[test]
fn online_vs_offline_read_emit_names_differ() {
    use helix_im::acl::to_effect::{emit_channel_read_echo, emit_post_read};

    let ch = channel_id(7);
    // spec06 FixB：fat 工厂签名携 &PostFields；本用例仅断言事件名分流，data 内容无关 → default 即可。
    let fields = helix_im::sync_session::PostFields::default();
    // 离线 sync 应用路径：im:channel:read_echo。
    let offline = emit_channel_read_echo(ch, 42, "m_read", &fields);
    // 在线 WS post_read 路径：im:post:read。
    let online = emit_post_read(ch, 42, "m_read", &fields);

    assert_eq!(
        emit_event_name(&offline).as_deref(),
        Some("im:channel:read_echo"),
        "离线 sync 应用 read → im:channel:read_echo"
    );
    assert_eq!(
        emit_event_name(&online).as_deref(),
        Some("im:post:read"),
        "在线 WS post_read → im:post:read"
    );
    assert_ne!(
        emit_event_name(&offline),
        emit_event_name(&online),
        "同 type6 read 在两路径必须分流为不同事件名（A5 前端零改动契约）"
    );
}

// ─── 6. 端到端：handle_sync_reply 同 step 内吐 Persist + emits（cursor 仍独立推进）──

#[test]
fn handle_sync_reply_emits_mutation_events_in_same_step() {
    let mut h = Harness::new(&[6]);
    // 大缺口（cursor=0 / seq=99）→ **立即** backfill resync（根治 echo gate-gap·
    // fix/im-sync-cursor-catchup）：sync Http 在 ingest 当步即发出，不再等 1s gate timer。
    let gate_effects = h.step(inbound_event(6, 99, 1), 1000);
    let sync_corr = gate_effects
        .iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill·不等 1s timer）");

    // 服务端回报：一条可见 type1 + 一条 type6 read，nextSeq=2。
    let reply = sync_reply_events_with_messages(
        6,
        &[ev(1, 1, "m_a"), ev(2, 6, "m_a")],
        &[("m_a", "hi", "11")],
        2,
    );
    h.step(port_reply_with_body(sync_corr, enveloped(reply)), 3000);

    // 同 step 内：sink 含 mutation emits（received + update-by-post + read）。
    let names = emit_names(h.sink.as_slice());
    assert!(
        names.iter().any(|n| n == "im:post:received"),
        "应 emit im:post:received，got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "im:channel:update-by-post"),
        "应 emit im:channel:update-by-post，got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "im:channel:read_echo"),
        "sync 应用 type6 应 emit im:channel:read_echo（A5），got {names:?}"
    );

    // emit 不卡 cursor：persist ok 后 cursor 推进到 nextSeq=2。
    let persist_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("persist corr");
    h.step(port_reply_ok(persist_corr), 3100);
    assert_eq!(
        h.module.cursor_for(channel_id(6)),
        Some(2),
        "emit 不卡 cursor：persist ok 后 cursor 推进到 nextSeq=2"
    );
}
