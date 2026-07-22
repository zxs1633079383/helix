//! C2 验收：phantom + 离线消息内容落库 + readBits（type6）+ edit/revoke。
//!
//! 回归锚: C2（sync-v2 完成 spec §2）
//!
//! ## 行为真源（cses-client post.rs:1043-1134 batch_write_events）
//!
//! sync `events` 路径事件本身只带 `eventSeq/eventType/msgId/actorId`，消息内容在分离的
//! `messages` map（key=msgId）。落库 dispatch：
//! - type1 new：`msgId ∈ messages` → 落内容行（含 read_bits）；`∉` → phantom（不落行，cursor 照推）。
//! - type2 edit：`msgId ∈ messages` → 落内容；`∉` → noop。
//! - type3 revoke：恒 UPDATE revoke=1 WHERE id（无须 body）。
//! - type6 read：读 `messages[msgId].read_bits` → UPDATE read_bits（**单调覆盖非累加**）。
//!
//! ## 铁律（HX-C008）
//!
//! cursor 推进**独立于消息是否可见**（phantom 不卡 cursor）；read_bits 单调覆盖非累加。

mod common;
use common::*;

use helix_core::effect::{SqlValue, StorageOp};
use helix_im::acl::to_effect::batch_upsert_events_with_messages;
use helix_im::parser::parse_sync_response;
use helix_im::sync_session::SyncResponse;

// ─── 辅助：从 StorageOp 取列值 / 表名 ──────────────────────────────────────────

/// SqlValue 不实现 PartialEq（helix-core 类型）→ 测试侧提取为可比较的 plain 值。
fn as_text(v: &SqlValue) -> Option<String> {
    match v {
        SqlValue::Text(s) => Some(s.clone()),
        _ => None,
    }
}

fn as_int(v: &SqlValue) -> Option<i64> {
    match v {
        SqlValue::Integer(n) => Some(*n),
        _ => None,
    }
}

/// upsert 行某列的 TEXT 值。
fn upsert_text(op: &StorageOp, name: &str) -> Option<String> {
    match op {
        StorageOp::BatchUpsert(spec) => spec
            .rows
            .first()
            .and_then(|row| row.iter().find(|(k, _)| k == name).map(|(_, v)| v))
            .and_then(as_text),
        _ => None,
    }
}

/// update patch 某列的 TEXT 值。
fn update_patch_text(op: &StorageOp, name: &str) -> Option<String> {
    match op {
        StorageOp::BatchUpdate(spec) => spec
            .patch
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
            .and_then(as_text),
        _ => None,
    }
}

/// update patch 某列的 INTEGER 值。
fn update_patch_int(op: &StorageOp, name: &str) -> Option<i64> {
    match op {
        StorageOp::BatchUpdate(spec) => spec
            .patch
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
            .and_then(as_int),
        _ => None,
    }
}

/// update WHERE key 的首个 key_val TEXT。
fn update_key_text(op: &StorageOp) -> Option<String> {
    match op {
        StorageOp::BatchUpdate(spec) => spec.key_vals.first().and_then(as_text),
        _ => None,
    }
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

// ─── 1. phantom：msgId ∉ messages → 不落 message 行 ────────────────────────────

#[test]
fn phantom_event_produces_no_upsert_but_cursor_advances() {
    // 两条 type1：msg_visible 在 map（落库），msg_phantom 不在 map（phantom）。
    let reply = sync_reply_events_with_messages(
        1,
        &[ev(10, 1, "msg_visible"), ev(11, 1, "msg_phantom")],
        &[("msg_visible", "hello", "")],
        11,
    );
    let (events, messages) = parse_events_and_messages(&reply, 1);
    assert_eq!(events.len(), 2, "解析出两条事件");
    assert_eq!(messages.len(), 1, "messages map 只含 visible");

    let ops = batch_upsert_events_with_messages(&events, &messages);
    // 仅 1 条 upsert（phantom 不产 op）——但事件计数为 2，差额即 phantom。
    assert_eq!(
        ops.len(),
        1,
        "phantom 不落行：2 事件只产 1 upsert，got {ops:?}"
    );
    assert!(
        matches!(&ops[0], StorageOp::BatchUpsert(s) if s.table == "message"),
        "唯一 op 是 message upsert"
    );
    assert_eq!(
        upsert_text(&ops[0], "message").as_deref(),
        Some("hello"),
        "落库内容取自 messages map"
    );
    assert_eq!(
        upsert_text(&ops[0], "id").as_deref(),
        Some("msg_visible"),
        "id 取 visible msgId"
    );
}

// ─── 2. phantom 不卡 cursor（端到端：handle_sync_reply → persist ok → cursor=nextSeq）──

#[test]
fn all_phantom_batch_still_advances_cursor() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // 大缺口（cursor=0 / seq=99·gap=99 > 阈值）→ **立即** backfill resync（根治 echo gate-gap·
    // fix/im-sync-cursor-catchup）。不再等 1s gate timer：sync Http 在 ingest 当步即发出，cursor
    // 经其回报 commit 追平。此处直接取当步 immediate sync 的 corr 驱动后续 phantom 回报。
    let gate_effects = h.step(inbound_event(1, 99, 1), 1000);
    let sync_corr = gate_effects
        .iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill·不等 1s timer）");

    // 服务端回报：两条事件全 phantom（messages map 空），nextSeq=5。
    let reply = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "p1"), ev(2, 1, "p2")],
        &[], // 空 messages → 全 phantom
        5,
    );
    let p_effects = h.step(port_reply_with_body(sync_corr, enveloped(reply)), 3000);
    let persist_corr = p_effects
        .iter()
        .find_map(|e| match e {
            EffectKind::PersistAtomic(c) => Some(*c),
            _ => None,
        })
        .expect("phantom 批仍发 PersistAtomic（ledger/coverage/cursor 同批，PortReply Ok 后推进）");

    // persist ok → cursor 推进到 nextSeq=5（phantom 不卡 cursor，HX-C008 §1.6）。
    h.step(port_reply_ok(persist_corr), 3100);
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(5),
        "全 phantom 批后 cursor 仍推进到服务端权威 nextSeq=5"
    );
}

// ─── 3. type6 read：读 messages[msgId].read_bits → UPDATE read_bits（单调覆盖）──────

#[test]
fn read_event_applies_read_bits_via_update() {
    let reply = sync_reply_events_with_messages(
        2,
        &[ev(20, 6, "msg_read")],
        &[("msg_read", "body", "1011")],
        20,
    );
    let (events, messages) = parse_events_and_messages(&reply, 2);
    let ops = batch_upsert_events_with_messages(&events, &messages);

    assert_eq!(ops.len(), 1, "type6 产一条 read_bits UPDATE");
    assert!(
        matches!(&ops[0], StorageOp::BatchUpdate(s) if s.table == "message" && s.key_col == "id"),
        "type6 用 BatchUpdate WHERE id（非 upsert，message.id 无 UNIQUE 约束），got {:?}",
        ops[0]
    );
    assert_eq!(
        update_patch_text(&ops[0], "read_bits").as_deref(),
        Some("1011"),
        "read_bits 直接赋值（单调覆盖非累加），取自 messages map"
    );
    assert_eq!(
        update_key_text(&ops[0]).as_deref(),
        Some("msg_read"),
        "UPDATE WHERE id = msgId"
    );
}

#[test]
fn read_event_without_read_bits_in_map_is_skipped() {
    // type6 但 map 中该 msg read_bits 空 → 无可写位 → 跳过。
    let reply = sync_reply_events_with_messages(
        2,
        &[ev(21, 6, "msg_no_rb")],
        &[("msg_no_rb", "body", "")],
        21,
    );
    let (events, messages) = parse_events_and_messages(&reply, 2);
    let ops = batch_upsert_events_with_messages(&events, &messages);
    assert!(ops.is_empty(), "read_bits 空 → 不产 op，got {ops:?}");
}

// ─── 4. type3 revoke：恒 UPDATE revoke=1 WHERE id（无须 body）──────────────────

#[test]
fn revoke_event_marks_revoke_via_update_regardless_of_messages() {
    // revoke 不在 messages map 也要标 revoke（真源：无须 body）。
    let reply = sync_reply_events_with_messages(
        3,
        &[ev(30, 3, "msg_gone")],
        &[], // 空 map
        30,
    );
    let (events, messages) = parse_events_and_messages(&reply, 3);
    let ops = batch_upsert_events_with_messages(&events, &messages);

    assert_eq!(ops.len(), 1, "revoke 恒产一条 op（无须 body）");
    assert!(
        matches!(&ops[0], StorageOp::BatchUpdate(s) if s.table == "message" && s.key_col == "id"),
        "revoke 用 BatchUpdate WHERE id，got {:?}",
        ops[0]
    );
    assert_eq!(
        update_patch_int(&ops[0], "revoke"),
        Some(1),
        "revoke 列置 1"
    );
    assert_eq!(
        update_key_text(&ops[0]).as_deref(),
        Some("msg_gone"),
        "UPDATE WHERE id = msgId"
    );
}

// ─── 5. type2 edit：msgId ∈ map → 仅覆盖内容列 UPDATE；∉ → noop ─────────────────

#[test]
fn edit_event_content_update_when_in_map_noop_when_phantom() {
    let reply = sync_reply_events_with_messages(
        4,
        &[ev(40, 2, "msg_edited"), ev(41, 2, "msg_edit_phantom")],
        &[("msg_edited", "edited text", "")],
        41,
    );
    let (events, messages) = parse_events_and_messages(&reply, 4);
    let ops = batch_upsert_events_with_messages(&events, &messages);

    assert_eq!(ops.len(), 1, "edit：命中 map 落库，phantom noop → 仅 1 op");
    // C2 BLOCKING-1：edit 走 BatchUpdate（仅内容列），**不**走全量 upsert。
    assert!(
        matches!(&ops[0], StorageOp::BatchUpdate(s) if s.table == "message" && s.key_col == "id"),
        "edit 用 BatchUpdate WHERE id（仅内容列覆盖，非全量 upsert），got {:?}",
        ops[0]
    );
    assert_eq!(
        update_patch_text(&ops[0], "message").as_deref(),
        Some("edited text"),
        "edit 内容取自 messages map"
    );
    assert_eq!(
        update_key_text(&ops[0]).as_deref(),
        Some("msg_edited"),
        "UPDATE WHERE id = msgId"
    );
}

/// 回归锚: C2 BLOCKING-1 —— edit(type2) 不得擦回本地 read_bits。
///
/// 真源 cses `update_content_by_message_id`（message_repo.rs:185-221）显式只 UPDATE 内容列，
/// **不含** read_bits/send_status；对齐 Telegram applyEdit「只改内容不碰已读态」。
/// 反例（修复前）：edit 走 event_to_upsert_op 全量 upsert，edit body 空 readBits 经
/// ON CONFLICT DO UPDATE `read_bits=excluded.read_bits` 把已落地行 read_bits(=1011) 擦成 ""。
///
/// 本测试断言：edit 产出的 BatchUpdate patch **不含 read_bits 列**（也不含 send_status），
/// 故无论 edit body 是否带 readBits，已落地行的本地已读位都不被触碰。
#[test]
fn edit_event_does_not_overwrite_local_read_bits() {
    // edit body 不带 readBits（map 第三元组传空串 → readBits=""，典型 edit 形态）。
    let reply = sync_reply_events_with_messages(
        7,
        &[ev(70, 2, "msg_with_local_rb")],
        &[("msg_with_local_rb", "new content after edit", "")],
        70,
    );
    let (events, messages) = parse_events_and_messages(&reply, 7);
    let ops = batch_upsert_events_with_messages(&events, &messages);

    assert_eq!(ops.len(), 1, "edit 命中 map → 1 op");
    let StorageOp::BatchUpdate(spec) = &ops[0] else {
        panic!("edit 必须是 BatchUpdate（仅内容列），got {:?}", ops[0]);
    };
    // 核心断言：patch 绝不含 read_bits / send_status（本地权威列）。
    for forbidden in ["read_bits", "send_status"] {
        assert!(
            !spec.patch.iter().any(|(k, _)| k == forbidden),
            "edit patch 不得含本地权威列 `{forbidden}`（擦回已读位/送达态），patch={:?}",
            spec.patch
        );
    }
    // 内容列照常覆盖。
    assert_eq!(
        update_patch_text(&ops[0], "message").as_deref(),
        Some("new content after edit"),
        "edit 内容列正常覆盖"
    );
    // patch 仅 3 个内容列（type/message/props），不多不少。
    assert_eq!(
        spec.patch.len(),
        3,
        "edit patch 仅 type/message/props 三内容列"
    );
}

// ─── 6. 混合批：分桶顺序 type1→type3→type6 + phantom 混入 ──────────────────────

#[test]
fn mixed_batch_buckets_and_phantom_combined() {
    // 解析后 parser 已分桶排序（type1 → type2 → type3 → type6）。
    let reply = sync_reply_events_with_messages(
        5,
        &[
            ev(50, 6, "m_new"),     // read（后处理）
            ev(51, 1, "m_new"),     // new（先处理，落内容 + read_bits）
            ev(52, 1, "m_phantom"), // new phantom
            ev(53, 3, "m_new"),     // revoke
        ],
        &[("m_new", "content", "0101")],
        53,
    );
    let (events, messages) = parse_events_and_messages(&reply, 5);

    // 分桶铁律：events 顺序应是 type1, type1, type3, type6。
    let types: Vec<u8> = events.iter().map(|e| e.kind.type_num()).collect();
    assert_eq!(
        types,
        vec![1, 1, 3, 6],
        "parser 分桶排序铁律 type1→type3→type6，got {types:?}"
    );

    let ops = batch_upsert_events_with_messages(&events, &messages);
    // 产物：1 个 type1 upsert（m_new 落内容）+ phantom 跳 + 1 个 revoke UPDATE + 1 个 read UPDATE = 3 op。
    assert_eq!(
        ops.len(),
        3,
        "1 upsert + 1 revoke + 1 read（phantom 跳），got {ops:?}"
    );
    assert!(matches!(&ops[0], StorageOp::BatchUpsert(s) if s.table == "message"));
    assert_eq!(
        upsert_text(&ops[0], "read_bits").as_deref(),
        Some("0101"),
        "type1 落库即带 read_bits（map 内容）"
    );
    assert!(
        matches!(&ops[1], StorageOp::BatchUpdate(s) if s.patch.iter().any(|(k, _)| k == "revoke"))
    );
    assert!(
        matches!(&ops[2], StorageOp::BatchUpdate(s) if s.patch.iter().any(|(k, _)| k == "read_bits"))
    );
}
