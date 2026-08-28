//! sync-v2 §S4 写顺序铁律 对齐补全回放测试。
//!
//! 回归锚: S4（SPEC-sync-v2-full-replacement.md §S4）
//!
//! 补齐既有 c2_phantom_readbits_test 未显式锁定的契约：
//! - 同批 type1+type6（同 msg）→ StorageOp 严格 type1 先 / type6 后（否则 type1 全量
//!   upsert 经 ON CONFLICT 擦掉 type6 单调覆盖的 read_bits）；
//! - 全四类混批严格 type1→type2→type3→type6（type6 殿后）；
//! - db_ok 失败 → cursor 不推进（写库成功才 commit，否则 re-pull）。

mod common;
use common::*;

use helix_core::effect::StorageOp;
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Tick};
use helix_im::acl::to_effect::batch_upsert_events_with_messages;
use helix_im::acl::to_effect::batch_upsert_events_with_messages_and_auth;
use helix_im::parser::parse_sync_response;
use helix_im::sync_session::SyncResponse;

fn parse_evs(
    bytes: &bytes::Bytes,
    channel_id: u64,
) -> (
    Vec<helix_im::sync_session::EventEnvelope>,
    std::collections::HashMap<String, helix_im::sync_session::PostFields>,
) {
    let ch = common::channel_id(channel_id);
    match parse_sync_response(bytes.as_ref(), ch).expect("parse sync") {
        SyncResponse::Events {
            events, messages, ..
        } => (events, messages),
        other => panic!("expected Events, got {other:?}"),
    }
}

/// 取 op 的「桶序号」：type1 upsert / type6 read / type3 revoke / type2 edit 各有可辨形态。
#[derive(Debug, PartialEq, Eq)]
enum OpKind {
    Type1Upsert,
    Type6Read,
    Type3Revoke,
    Type2Edit,
    Other,
}

fn classify(op: &StorageOp) -> OpKind {
    match op {
        StorageOp::BatchUpsert(s) if s.table == "message" => OpKind::Type1Upsert,
        StorageOp::BatchUpdate(s) if s.table == "message" => {
            if s.patch.iter().any(|(k, _)| k == "read_bits") {
                OpKind::Type6Read
            } else if s.patch.iter().any(|(k, _)| k == "revoke") {
                OpKind::Type3Revoke
            } else {
                OpKind::Type2Edit
            }
        }
        _ => OpKind::Other,
    }
}

/// 回归锚：Sync type=1 必须与在线 Post 共用 viewer-scoped dialog 原子写集。
#[test]
fn sync_type1_compiles_message_channel_and_viewer_member_in_one_batch() {
    let reply =
        sync_reply_events_with_messages(1, &[ev(1, 1, "m1")], &[("m1", "first unread", "")], 1);
    let (events, messages) = parse_evs(&reply, 1);

    let ops = batch_upsert_events_with_messages_and_auth(&events, &messages, "viewer-444");

    assert!(
        ops.iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
        "type=1 必须写入消息事实"
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, StorageOp::BatchUpdate(spec) if spec.table == "channel")),
        "type=1 必须在同一写集推进共享 channel lastPost"
    );
    assert!(
        ops.iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel_member")),
        "type=1 必须在同一写集确保当前 viewer 的 channel_member 行存在"
    );
    assert!(
        ops.iter().any(|op| matches!(
            op,
            StorageOp::ScopedGuardedBump(spec)
                if spec.table == "channel_member"
                    && spec.scope_col == "channel_id"
                    && spec.key_col == "user_id"
                    && spec.bump_col == "unread_count"
        )),
        "他人可见消息必须按 (channel_id,user_id) 守卫递增 viewer 未读"
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, StorageOp::GuardedBump(spec) if spec.table == "channel")),
        "共享 channel.unread_count 不得继续承载 viewer 未读"
    );
}

/// 回归锚：Sync 提交成功后不依赖时间线 attach，必须读回 viewer member 绝对态。
#[test]
fn sync_type1_persist_ok_reads_viewer_member_for_unattached_channel() {
    use helix_core::effect::Effect;

    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let gap = h.step(inbound_event(1, 99, 1), 1_000);
    let sync_corr = gap
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(*corr),
            _ => None,
        })
        .expect("gap 必须发起 sync/notify");
    let reply =
        sync_reply_events_with_messages(1, &[ev(1, 1, "m1")], &[("m1", "first unread", "")], 1);
    let sync_effects = h.step(port_reply_with_body(sync_corr, enveloped(reply)), 2_000);
    let persist_corr = sync_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("Sync type=1 必须等待原子提交");

    h.step(port_reply_ok(persist_corr), 2_001);

    assert!(
        h.sink.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Persist { ops, .. }
                if matches!(ops.as_slice(), [StorageOp::ScopedGet(spec)]
                    if spec.table == "channel_member"
                        && spec.scope_col == "channel_id"
                        && spec.key_col == "user_id")
        )),
        "未 attach 的 A 群也必须按 (channel_id,user_id) 读回 viewer 未读绝对态"
    );
    let readback_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if matches!(ops.as_slice(), [StorageOp::ScopedGet(spec)]
                    if spec.table == "channel_member") =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("必须保留 member readback corr");
    let member_rows = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!([{
            "channel_id": cid(1),
            "user_id": "viewer-444",
            "unread_count": 1,
            "last_post": serde_json::json!({"id": "m1", "message": "first unread"}).to_string()
        }]))
        .expect("serialize member readback"),
    );

    h.step(port_reply_with_body(readback_corr, member_rows), 2_002);
    let projected: Vec<serde_json::Value> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect();
    assert_eq!(
        projected.len(),
        1,
        "Sync dialog readback 只能发布一个终态事件"
    );
    assert_eq!(projected[0]["event"], "im:channel:update");
    assert_eq!(projected[0]["data"]["unreadCount"], 1);
}

/// 回归锚: S4 —— 同批 type1 + type6（同 msg）→ type1 upsert 必须排在 type6 read 之前。
///
/// 铁律：type6 单调覆盖 read_bits 的 UPDATE 必须在 type1 全量 upsert **之后**执行；否则
/// type1 的 ON CONFLICT(temporary_id) DO UPDATE 会用 map body 的 read_bits（可能为旧值）
/// 把 type6 刚写的权威 read_bits 擦回。helix 在 parser `sort_events_by_bucket` 稳定排序
/// type1→type6，故落库 ops 顺序天然铁律对齐。本测试钉死该顺序不可漂移。
#[test]
fn s4_type1_before_type6_in_same_batch() {
    // wire 乱序投递：type6 在前、type1 在后（同 msg m1）——parser 必须分桶纠正为 type1→type6。
    let reply = sync_reply_events_with_messages(
        1,
        &[ev(10, 6, "m1"), ev(11, 1, "m1")],
        &[("m1", "body", "1111")],
        11,
    );
    let (events, messages) = parse_evs(&reply, 1);

    // parser 分桶：events 顺序 type1, type6。
    let types: Vec<u8> = events.iter().map(|e| e.kind.type_num()).collect();
    assert_eq!(types, vec![1, 6], "parser 须分桶 type1→type6（type6 最后）");

    let ops = batch_upsert_events_with_messages(&events, &messages);
    let kinds: Vec<OpKind> = ops
        .iter()
        .map(classify)
        .filter(|kind| *kind != OpKind::Other)
        .collect();
    let p1 = kinds
        .iter()
        .position(|k| *k == OpKind::Type1Upsert)
        .expect("有 type1 upsert");
    let p6 = kinds
        .iter()
        .position(|k| *k == OpKind::Type6Read)
        .expect("有 type6 read");
    assert!(
        p1 < p6,
        "S4 铁律：type1 upsert(pos={p1}) 必须早于 type6 read(pos={p6})，否则擦 read_bits。kinds={kinds:?}"
    );
}

/// 回归锚: S4 —— 全四类混批严格 type1→type2→type3→type6（type6 殿后）。
#[test]
fn s4_four_bucket_strict_order() {
    let reply = sync_reply_events_with_messages(
        2,
        &[
            ev(24, 6, "a"), // read（须排末）
            ev(23, 3, "b"), // revoke
            ev(22, 2, "c"), // edit
            ev(21, 1, "d"), // new（须排首）
        ],
        &[("a", "x", "01"), ("c", "edited", ""), ("d", "new", "10")],
        24,
    );
    let (events, messages) = parse_evs(&reply, 2);
    let ops = batch_upsert_events_with_messages(&events, &messages);
    let kinds: Vec<OpKind> = ops
        .iter()
        .map(classify)
        .filter(|kind| *kind != OpKind::Other)
        .collect();
    // d=type1 upsert, c=type2 edit, b=type3 revoke, a=type6 read → 四桶各一，顺序铁律。
    assert_eq!(
        kinds,
        vec![
            OpKind::Type1Upsert,
            OpKind::Type2Edit,
            OpKind::Type3Revoke,
            OpKind::Type6Read,
        ],
        "S4 铁律：落库 ops 须严格 type1→type2→type3→type6，got {kinds:?}"
    );
}

/// 回归锚: S4 —— db_ok 失败 → cursor 不推进（写库成功才 commit，否则 re-pull）。
#[test]
fn s4_persist_err_does_not_advance_cursor() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    // 大缺口（seq=99 / cursor=0）→ **立即** backfill resync（根治 fix/im-sync-cursor-catchup）：
    // sync Http 在 ingest 当步即发出，不再等 1s gate timer fire。
    let gate = h.step(inbound_event(1, 99, 1), 1000);
    let sync_corr = gate
        .iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill）");

    // 服务端回报 events（seq 1-3，nextSeq=3）→ 产 Persist{corr}。
    let reply = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "m1"), ev(2, 1, "m2"), ev(3, 1, "m3")],
        &[("m1", "a", ""), ("m2", "b", ""), ("m3", "c", "")],
        3,
    );
    // dev-zlc ADR-007：handle_sync_reply 吃 HTTP 信封（base64 body），enveloped() 复刻 driver 回灌字节。
    let p = h.step(port_reply_with_body(sync_corr, enveloped(reply)), 3000);
    let persist_corr = p
        .iter()
        .find_map(|e| match e {
            EffectKind::PersistAtomic(c) => Some(*c),
            _ => None,
        })
        .expect("Persist corr");

    // persist 失败（db_ok=false）→ cursor 不得推进（仍 0），下次 re-pull。
    h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(persist_corr),
            outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
        },
        3100,
    );
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(0),
        "S4：persist 失败时 cursor 不推进（写库成功才 commit），got {:?}",
        h.module.cursor_for(channel_id(1))
    );
}

/// 回归锚：sync type=1 已把本端 post 写入 SQLite 后，必须走 PendingSend::reconcile。
#[test]
fn sync_persist_success_reconciles_local_pending_send() {
    use helix_core::effect::Effect;
    use helix_im::state::SendStatus;
    use serde_json::json;

    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let initial = h.step(send_message_cmd(1, "tmp_sync", "from-sync"), 1_000);
    let p1_corr = initial
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("send must register an optimistic persist");
    h.step(port_reply_ok(p1_corr), 1_001);
    assert_eq!(
        h.module.pending_send_status("tmp_sync"),
        Some(SendStatus::Sending)
    );

    let _ = h.step(inbound_event(1, 99, 1), 2_000);
    let sync_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => Some(*corr),
            _ => None,
        })
        .expect("gap recovery must dispatch sync/notify");
    let server_id = sid(42);
    let mut messages = serde_json::Map::new();
    messages.insert(
        server_id.clone(),
        json!({
            "id": server_id,
            "temporaryId": "tmp_sync",
            "channelId": cid(1),
            "userId": "viewer-444",
            "type": "TEXT",
            "message": "from-sync",
            "viewers": ["viewer-444"]
        }),
    );
    let raw_reply = json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": cid(1),
                "kind": "events",
                "events": [{
                    "channelId": cid(1),
                    "eventSeq": 1,
                    "eventType": 1,
                    "msgId": sid(42)
                }],
                "messages": serde_json::Value::Object(messages),
                "nextSeq": 1,
                "resetTo": null
            }]
        }
    });
    let sync_effects = h.step(
        port_reply_with_body(
            sync_corr.raw(),
            enveloped(bytes::Bytes::from(serde_json::to_vec(&raw_reply).unwrap())),
        ),
        3_000,
    );
    let persist_corr = sync_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("sync events must wait for one atomic local persist");
    assert_eq!(
        h.module.pending_send_status("tmp_sync"),
        Some(SendStatus::Sending),
        "sync must not settle the send before SQLite commit"
    );

    let committed = h.step(port_reply_ok(persist_corr), 3_001);
    assert_eq!(
        h.module.pending_send_status("tmp_sync"),
        None,
        "a durable sync fact must remove the pending send after reconcile"
    );
    assert!(
        committed
            .iter()
            .any(|effect| matches!(effect, EffectKind::Persist(_))),
        "sync reconcile must persist server id and sent status"
    );
    assert!(
        committed
            .iter()
            .any(|effect| matches!(effect, EffectKind::CancelTimer(_))),
        "sync reconcile must cancel the original timeout"
    );
}
