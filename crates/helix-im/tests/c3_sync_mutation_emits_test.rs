//! C3-core 验收：sync 驱动的 post:* mutation 事件 + im:__bus__ 信封。
//!
//! 回归锚: C3（sync-v2 完成 spec §3）
//!
//! ## 行为真源（cses-client post.rs:1146-1448 apply_event_with_messages）
//!
//! sync `events` 路径逐事件 dispatch，按 event_type + 是否命中 messages map（phantom 判定）
//! emit 对应 `im:*` 业务事件：
//! - type1 new：`msgId ∈ messages`（可见）→ im:post:received；左侧列表累计态由写后 InteractionFrame 承接；
//!   `∉`（phantom）→ 不 emit（UI 不渲染，真源 1270-1280）。
//! - type2 edit：`msgId ∈ messages` → im:post:updated；phantom → noop（真源 1346）。
//! - type3 revoke：有 msgId → im:post:deleted（无须 body，真源 1402）。
//! - type6 read：有 msgId → im:channel:read_echo（A5 sync 应用路径，无须 body 恒 emit，真源 1428）。
//!
//! ## 铁律（HX-C008）
//!
//! emit 与落库同口径（可见才动 UI），并与 cursor 共用相关 Persist 屏障：只有
//! PortReply Ok 才统一发布 mutation/DialogPatch/sync-state 并推进到 commit_target。

mod common;
use common::*;

use helix_core::effect::StorageOp;
use helix_core::Effect;
use helix_im::acl::to_effect::{
    batch_upsert_events_with_messages_and_auth, sync_dialog_patch_plans, sync_mutation_emits,
    sync_mutation_emits_with_auth, sync_unread_bump_ops,
};
use helix_im::parser::parse_sync_response;
use helix_im::sync_session::{EventEnvelope, EventKind, PostFields, SyncResponse};

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

fn channel_row_reply(channel_id: u64, unread_count: i64) -> bytes::Bytes {
    let row = serde_json::json!([{
        "id": cid(channel_id),
        "unread_count": unread_count,
        "mention_count": 0,
        "urgent_count": 0,
        "mention_list": [],
        "urgent_post_list": [],
        "unread_post_id": "m_a",
        "last_root_post_at": 1_700_000_001_i64,
    }]);
    bytes::Bytes::from(serde_json::to_vec(&row).unwrap())
}

fn sync_post_event(channel_num: u64, seq: u64, msg_id: &str) -> EventEnvelope {
    EventEnvelope::new(
        channel_id(channel_num),
        helix_im::state::Seq(seq),
        EventKind::PostUpsert,
        PostFields::default(),
    )
    .with_msg_id(Some(msg_id.to_string()))
}

fn sync_post_fields(msg_id: &str, sender: &str, viewers: &[&str]) -> PostFields {
    PostFields {
        id: msg_id.to_string(),
        user_id: sender.to_string(),
        msg_type: "TEXT".to_string(),
        message: format!("body-{msg_id}"),
        simple_message: format!("body-{msg_id}"),
        create_at: 1_700_000_000,
        viewers: viewers.iter().map(|v| (*v).to_string()).collect(),
        ..PostFields::default()
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

#[test]
fn revoke_replay_preserves_actor_event_and_time_for_viewer_ready_projection() {
    let channel = cid(3);
    let reply = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "entries": [{
                "channelId": channel,
                "kind": "events",
                "events": [{
                    "id": "event-revoke-30",
                    "channelId": channel,
                    "eventSeq": 30,
                    "eventType": 3,
                    "msgId": "msg_gone",
                    "actorId": "viewer-444",
                    "createAt": 1_700_000_030_i64,
                    "payload": {"reason": "revoke"}
                }],
                "messages": {},
                "nextSeq": 30
            }]
        }))
        .unwrap(),
    );
    let (events, messages) = parse_events_and_messages(&reply, 3);
    assert_eq!(events[0].event_id, "event-revoke-30");
    assert_eq!(events[0].actor_id, "viewer-444");
    assert_eq!(events[0].occurred_at, 1_700_000_030_i64);
    assert_eq!(events[0].event_payload, r#"{"reason":"revoke"}"#);

    let emits = sync_mutation_emits_with_auth(&events, &messages, "viewer-444");
    let Effect::Emit { event } = &emits[0] else {
        panic!("撤回重放必产 Emit")
    };
    let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(value["data"]["eventId"], format!("revoke:{}:30", cid(3)));
    assert_eq!(value["data"]["actorId"], "viewer-444");
    assert_eq!(value["data"]["displayText"], "你撤回了一条消息");
    assert_eq!(value["data"]["targetMsgId"], "msg_gone");
    assert_eq!(value["data"]["eventSeq"], 30);
    assert_eq!(value["data"]["occurredAt"], 1_700_000_030_i64);
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
        vec!["im:post:received"],
        "可见 new 先 emit received；DialogPatch 改为 Persist 写后读回累计 channel 行再 emit"
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

#[test]
fn sync_type1_body_still_rechecks_viewers_gate_locally() {
    let events = vec![sync_post_event(1, 12, "msg_private")];
    let mut messages = std::collections::HashMap::new();
    messages.insert(
        "msg_private".to_string(),
        sync_post_fields("msg_private", "sender-user", &["other-viewer"]),
    );

    let emits = sync_mutation_emits_with_auth(&events, &messages, "auth-user");
    let upserts = batch_upsert_events_with_messages_and_auth(&events, &messages, "auth-user");
    let bumps = sync_unread_bump_ops(&events, &messages, "auth-user");
    let plans = sync_dialog_patch_plans(&events, &messages, "auth-user");

    assert!(
        emit_names(&emits).is_empty(),
        "messages map 即使带 body，本地 viewers 不含 auth 也不能发 UI patch"
    );
    assert!(
        !upserts
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
        "不可见 type1 不能落可见 message"
    );
    assert!(bumps.is_empty(), "不可见 type1 不能更新 Dialog unread");
    assert!(plans.is_empty(), "不可见 type1 不能生成 DialogPatch 计划");
}

#[test]
fn sync_type1_sender_self_visible_exception_but_unread_zero() {
    let events = vec![sync_post_event(1, 13, "msg_self")];
    let mut messages = std::collections::HashMap::new();
    messages.insert(
        "msg_self".to_string(),
        sync_post_fields("msg_self", "auth-user", &["other-viewer"]),
    );

    let emits = sync_mutation_emits_with_auth(&events, &messages, "auth-user");
    let upserts = batch_upsert_events_with_messages_and_auth(&events, &messages, "auth-user");
    let bumps = sync_unread_bump_ops(&events, &messages, "auth-user");
    let plans = sync_dialog_patch_plans(&events, &messages, "auth-user");

    assert_eq!(
        emit_names(&emits),
        vec!["im:post:received"],
        "sender 自己发的消息即使 viewers 不含 auth 也可见"
    );
    assert!(
        upserts
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
        "sender 自己消息需要落可见 message"
    );
    assert_eq!(bumps.len(), 1, "own echo 仍更新 lastPost/channel row");
    match &bumps[0] {
        StorageOp::GuardedBump(spec) => {
            assert_eq!(spec.bump_delta, 0, "own echo 不增加 unread_count");
        }
        other => panic!("expected GuardedBump, got {other:?}"),
    }
    assert_eq!(
        plans.len(),
        1,
        "own echo 仍需写后读回 DialogPatch 刷新 lastPost"
    );
    assert_eq!(plans[0].update.unread_delta, 0);
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
    // type1 可见 → received；DialogPatch 由 Persist 写后读回链路发出；type1 phantom → 无；type3 → deleted；
    // type6 → channel:read_echo（A5 sync 应用路径）。
    assert_eq!(
        emit_names(&emits),
        vec![
            "im:post:received",
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

// ─── 6. 端到端：handle_sync_reply 先 Persist，ack 后统一释放 mutation emits ───

#[test]
fn handle_sync_reply_releases_mutation_events_after_persist_ack() {
    let mut h = Harness::new_with_identity(&[6], "viewer-444", "tenant-1", "", "", "");
    h.step(
        command_json("im_query_dialog_list", serde_json::json!({})),
        1,
    );
    let dialog_scan_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("initial dialog list scan correlation");
    h.step(
        port_reply_with_body(dialog_scan_corr, channel_row_reply(6, 0)),
        2,
    );

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

    // G-S2/G-S3 屏障：HTTP 回包只生成相关 Persist，mutation 与 DialogPatch 均不得抢跑。
    let names_before_persist = emit_names(h.sink.as_slice());
    assert!(
        !names_before_persist.iter().any(|n| n == "im:post:received"),
        "Persist ack 前不得 emit im:post:received，got {names_before_persist:?}"
    );
    assert!(
        !names_before_persist
            .iter()
            .any(|n| n == "im:channel:update-by-post"),
        "DialogPatch 必须等 Persist 写后 Get(channel) 回包，got {names_before_persist:?}"
    );
    assert!(
        !names_before_persist
            .iter()
            .any(|n| n == "im:channel:read_echo"),
        "Persist ack 前不得 emit im:channel:read_echo，got {names_before_persist:?}"
    );

    // Persist Ok 同时释放 mutation、sync-state 并推进 cursor。PersistAtomic 是纯写屏障，
    // 因此 attached dialog 必须在 ack 后发起目标 channel Get，不能依赖虚构的写后 row body。
    let persist_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("sync authority batch PersistAtomic corr");
    h.step(port_reply_ok(persist_corr), 3100);
    let names_after_persist = emit_names(h.sink.as_slice());
    assert!(
        names_after_persist.iter().any(|n| n == "im:post:received"),
        "Persist ack 后应 emit im:post:received，got {names_after_persist:?}"
    );
    assert!(
        names_after_persist
            .iter()
            .any(|n| n == "im:channel:read_echo"),
        "Persist ack 后 sync type6 应 emit im:channel:read_echo，got {names_after_persist:?}"
    );
    assert!(
        !names_after_persist
            .iter()
            .any(|n| n == "im:channel:update-by-post"),
        "写后 channel row 回包不得 emit raw DialogPatch，got {names_after_persist:?}"
    );
    let refresh_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel")) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("PersistOk 后必须从本地 channel 真源回读目标 dialog");
    h.step(
        port_reply_with_body(refresh_corr, channel_row_reply(6, 1)),
        3200,
    );
    let dialog_frame = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (envelope["event"] == "im:interaction:frame"
                    && envelope["data"]["mutations"][0]["surface"] == "message.left-dialog-list")
                    .then_some(envelope)
            }
            _ => None,
        })
        .expect("channel Get 回报必须发布 left-dialog-list V2 Frame");
    assert_eq!(
        dialog_frame["data"]["mutations"][0]["causationId"],
        format!("sync-persist-{persist_corr}")
    );
    assert_eq!(
        h.module.cursor_for(channel_id(6)),
        Some(2),
        "emit 不卡 cursor：persist ok 后 cursor 推进到 nextSeq=2"
    );
}
