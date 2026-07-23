//! T095：Go type-7 closed wire 的原子 tombstone 合同。
//!
//! 终态只接受密封的同步事件；它必须先成为 `PersistAtomic(channel_event_cursor)`，成功回执后
//! 才推进 cursor、记 tombstone 并投影关闭的 dialog Frame。这个文件刻意覆盖拒绝、失败、幂等
//! 与重启恢复，避免回退到旧的「未知 type-7 直接拒绝」语义。

mod common;
use common::*;

use bytes::Bytes;
use helix_core::{Effect, Module};
use serde_json::{json, Value};

const SURFACE_RESYNC: &str = "__helix_request_surface_snapshot";

fn strict_terminal_reply(channel_id: u64, seq: u64) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "entries": [{
                    "channelId": cid(channel_id),
                    "kind": "events",
                    "events": [{
                        "id": format!("event-close-{seq}"),
                        "channelId": cid(channel_id),
                        "eventSeq": seq,
                        "eventType": 7,
                        "payload": {"state": "closed"}
                    }],
                    "messages": {},
                    "nextSeq": seq,
                    "resetTo": Value::Null
                }],
                "serverTime": 1_700_000_000_i64
            }
        }))
        .unwrap(),
    )
}

fn strict_terminal_reply_with_extra_field(channel_id: u64, seq: u64) -> Bytes {
    let mut reply: Value = serde_json::from_slice(strict_terminal_reply(channel_id, seq).as_ref())
        .expect("terminal fixture must be valid JSON");
    reply["data"]["entries"][0]["events"][0]["msgId"] = Value::String("not-allowed".into());
    Bytes::from(serde_json::to_vec(&reply).unwrap())
}

fn mixed_terminal_reply(channel_id: u64) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "entries": [{
                    "channelId": cid(channel_id),
                    "kind": "events",
                    "events": [{
                        "id": "event-system-post-1",
                        "channelId": cid(channel_id),
                        "eventSeq": 1,
                        "eventType": 1,
                        "msgId": "system-post-1"
                    }, {
                        "id": "event-close-2",
                        "channelId": cid(channel_id),
                        "eventSeq": 2,
                        "eventType": 7,
                        "payload": {"state": "closed"}
                    }],
                    "messages": {
                        "system-post-1": {
                            "id": "system-post-1",
                            "temporary_id": "system-post-1",
                            "channel_id": cid(channel_id),
                            "message": "频道已关闭",
                            "userId": "system",
                            "viewers": ["viewer-1"],
                            "createAt": 1_700_000_001_i64
                        }
                    },
                    "nextSeq": 2,
                    "resetTo": Value::Null
                }],
                "serverTime": 1_700_000_002_i64
            }
        }))
        .unwrap(),
    )
}

fn first_http_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("sync must start a correlated HTTP pull")
}

fn first_atomic_persist_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => {
                assert_eq!(ops.len(), 1, "terminal must use one combined cursor write");
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("strict terminal must produce PersistAtomic")
}

fn interaction_frames(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|event| event["event"] == "im:interaction:frame")
        .collect()
}

fn attach_dialog_surface(harness: &mut Harness, now_ms: u64) {
    harness.step(
        command_json(
            SURFACE_RESYNC,
            json!({
                "surface": "message.left-dialog-list",
                "scope": {"kind":"user", "userId":"viewer-1", "teamId":"team-1"}
            }),
        ),
        now_ms,
    );
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("dialog attachment must scan local channel facts");
    harness.step(
        port_reply_with_body(
            corr,
            Bytes::from(
                serde_json::to_vec(&json!([{
                    "id": cid(1),
                    "display_name": "Terminal channel",
                    "type": "P",
                    "last_post_at": 10
                }]))
                .unwrap(),
            ),
        ),
        now_ms + 1,
    );
    assert_eq!(interaction_frames(harness.sink.as_slice()).len(), 1);
}

fn start_gap_sync(harness: &mut Harness, now_ms: u64) -> u64 {
    harness.step(inbound_event(1, 99, 1), now_ms);
    first_http_corr(harness)
}

#[test]
fn strict_go_type7_wire_requires_atomic_commit_before_closed_frame_and_is_idempotent() {
    let mut harness = Harness::new_with_identity(&[1], "viewer-1", "team-1", "", "", "");
    attach_dialog_surface(&mut harness, 1_000);
    let sync_corr = start_gap_sync(&mut harness, 1_010);

    harness.step(
        port_reply_with_body(sync_corr, enveloped(strict_terminal_reply(1, 1))),
        1_011,
    );
    let atomic_corr = first_atomic_persist_corr(&harness);
    assert!(
        interaction_frames(harness.sink.as_slice()).is_empty(),
        "terminal cannot publish a Frame before its atomic storage receipt"
    );
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(harness.module.terminal_event_seq_for(channel_id(1)), None);

    // A second sync may already have been scheduled before the persistence receipt. Once the
    // terminal commits, that late exact Go reply must be a no-op rather than reopen the channel.
    harness.step(
        command_json("im_sync_channels", json!({"channel_id": cid(1)})),
        1_012,
    );
    let late_sync_corr = first_http_corr(&harness);

    harness.step(port_reply_ok(atomic_corr), 1_013);
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(1));
    assert_eq!(
        harness.module.terminal_event_seq_for(channel_id(1)),
        Some(1)
    );
    let frames = interaction_frames(harness.sink.as_slice());
    assert_eq!(frames.len(), 1, "only the committed terminal emits a Frame");
    assert_eq!(
        frames[0]["data"]["mutations"][0]["mutation"]["kind"],
        "dialogListPatch"
    );
    assert_eq!(
        frames[0]["data"]["mutations"][0]["mutation"]["patch"]["view"]["sections"][0]["kind"],
        "closed"
    );
    assert_eq!(
        frames[0]["data"]["mutations"][0]["mutation"]["patch"]["view"]["sections"][0]["items"][0]
            ["lifecycle"]["state"],
        "closed"
    );

    harness.step(
        port_reply_with_body(late_sync_corr, enveloped(strict_terminal_reply(1, 1))),
        1_014,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "same strict terminal replay after commit must not issue a second write or Frame"
    );
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(1));
    assert_eq!(
        harness.module.terminal_event_seq_for(channel_id(1)),
        Some(1)
    );
}

#[test]
fn mixed_visible_prefix_and_terminal_commit_once_without_resync_loop() {
    let mut harness = Harness::new_with_identity(&[1], "viewer-1", "team-1", "", "", "");
    attach_dialog_surface(&mut harness, 1_100);
    let sync_corr = start_gap_sync(&mut harness, 1_110);
    harness.sink.clear();

    harness.step(
        port_reply_with_body(sync_corr, enveloped(mixed_terminal_reply(1))),
        1_111,
    );
    let (atomic_corr, op_count) = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => Some((corr.raw(), ops.len())),
            _ => None,
        })
        .expect("valid mixed terminal recovery must cross one atomic persistence barrier");
    assert!(
        op_count >= 2,
        "the atomic batch must retain the visible prefix and terminal cursor/tombstone"
    );
    assert!(
        harness
            .sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "a deterministic valid response must not immediately request itself again"
    );
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(harness.module.terminal_event_seq_for(channel_id(1)), None);

    harness.step(port_reply_ok(atomic_corr), 1_112);
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(2));
    assert_eq!(
        harness.module.terminal_event_seq_for(channel_id(1)),
        Some(2)
    );
    let emitted: Vec<Value> = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect();
    assert!(
        emitted
            .iter()
            .any(|event| event["event"] == "im:post:received"),
        "visible prefix projection releases only after the shared atomic receipt"
    );
    assert_eq!(
        interaction_frames(harness.sink.as_slice()).len(),
        1,
        "the same receipt publishes one closed dialog Frame"
    );
}

#[test]
fn terminal_atomic_error_neither_advances_cursor_nor_emits_frame_then_can_retry() {
    let mut harness = Harness::new_with_identity(&[1], "viewer-1", "team-1", "", "", "");
    attach_dialog_surface(&mut harness, 2_000);
    let sync_corr = start_gap_sync(&mut harness, 2_010);
    harness.step(
        port_reply_with_body(sync_corr, enveloped(strict_terminal_reply(1, 1))),
        2_011,
    );
    let failed_atomic_corr = first_atomic_persist_corr(&harness);

    harness.step(port_reply_err(failed_atomic_corr), 2_012);
    assert!(
        interaction_frames(harness.sink.as_slice()).is_empty(),
        "failed atomic write cannot leak a closed Frame"
    );
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(harness.module.terminal_event_seq_for(channel_id(1)), None);

    harness.step(
        command_json("im_sync_channels", json!({"channel_id": cid(1)})),
        2_013,
    );
    let retry_sync_corr = first_http_corr(&harness);
    harness.step(
        port_reply_with_body(retry_sync_corr, enveloped(strict_terminal_reply(1, 1))),
        2_014,
    );
    let retry_atomic_corr = first_atomic_persist_corr(&harness);
    harness.step(port_reply_ok(retry_atomic_corr), 2_015);
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(1));
    assert_eq!(
        harness.module.terminal_event_seq_for(channel_id(1)),
        Some(1)
    );
    assert_eq!(interaction_frames(harness.sink.as_slice()).len(), 1);
}

#[test]
fn malformed_or_legacy_type7_never_reaches_atomic_storage() {
    let mut harness = Harness::new(&[1]);
    let sync_corr = start_gap_sync(&mut harness, 3_000);
    harness.sink.clear();
    let error = harness
        .module
        .handle(
            &port_reply_with_body(
                sync_corr,
                enveloped(strict_terminal_reply_with_extra_field(1, 1)),
            ),
            3_001,
            &mut harness.sink,
        )
        .expect_err("a type7 wire with msgId must fail closed");
    assert!(
        error
            .to_string()
            .contains("terminal event must contain only id,channelId,eventSeq,eventType,payload"),
        "error={error}"
    );
    assert!(
        harness
            .sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::PersistAtomic { .. } | Effect::Emit { .. })),
        "malformed terminal must not write or project"
    );
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(0));

    let mut legacy = Harness::new(&[1]);
    let effects = legacy.step(inbound_event(1, 1, 7), 3_010);
    assert!(
        effects.is_empty(),
        "legacy WS type7 is not the sealed sync wire"
    );
    assert!(legacy.sink.as_slice().is_empty());
    assert_eq!(legacy.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(legacy.module.terminal_event_seq_for(channel_id(1)), None);
}

#[test]
fn startup_tombstone_recovery_blocks_late_ws_and_sync_and_keeps_dialog_closed() {
    let mut harness = Harness::new_with_identity(&[], "viewer-1", "team-1", "", "", "");
    harness.start(4_000);
    let scan_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if matches!(
                    ops.as_slice(),
                    [helix_core::effect::StorageOp::Scan(spec)] if spec.table == "channel_event_cursor"
                ) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("startup must scan the terminal cursor projection");
    harness.step(
        port_reply_with_body(
            scan_corr,
            Bytes::from(
                serde_json::to_vec(&json!([{
                    "channel_id": cid(1),
                    "last_event_seq": 1,
                    "terminal_event_seq": 1,
                    "updated_at": 0
                }]))
                .unwrap(),
            ),
        ),
        4_001,
    );
    assert_eq!(harness.module.cursor_for(channel_id(1)), Some(1));
    assert_eq!(
        harness.module.terminal_event_seq_for(channel_id(1)),
        Some(1)
    );

    harness.step(inbound_event(1, 2, 1), 4_002);
    assert!(
        harness.sink.as_slice().is_empty(),
        "late WS cannot revive terminal channel"
    );
    harness.step(
        command_json("im_sync_channels", json!({"channel_id": cid(1)})),
        4_003,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "scheduler must not sync restored terminal channel"
    );

    attach_dialog_surface(&mut harness, 4_004);
    let frame = interaction_frames(harness.sink.as_slice())
        .into_iter()
        .next()
        .expect("restored channel still produces the typed dialog snapshot");
    assert_eq!(
        frame["data"]["mutations"][0]["mutation"]["view"]["sections"][0]["items"][0]["lifecycle"]
            ["state"],
        "closed",
        "snapshot overlays durable tombstone instead of reopening the dialog"
    );
}
