//! CATEGORY_CHAIN 非法 revision 的 fail-closed 回归。
//!
//! 非法 authority 不能通过任何公开入口写 cursor 或发布成功投影；在同一 seq
//! 随后收到合法 revision 时，仍须从原 cursor 正常恢复。

mod common;

use common::{cid, enveloped, hello_inbound, port_reply_ok, port_reply_with_body, sid, Harness};
use helix_core::effect::Effect;
use helix_core::{EffectSink, Module, Tick};
use serde_json::{json, Value};

/// 构造带指定 categoryChain.revision 的 CATEGORY_CHAIN/普通消息 WS 帧。
fn post_frame(
    channel_num: u64,
    seq: u64,
    msg_type: &str,
    revision: Option<Value>,
    with_seq: bool,
) -> Tick {
    let mut category_chain = json!({
        "chainId": "chain-invalid-revision",
        "status": "OPEN",
    });
    if let Some(revision) = revision {
        category_chain["revision"] = revision;
    }
    let mut data = json!({
        "id": sid(seq),
        "temporaryId": format!("tmp-invalid-revision-{seq}"),
        "channelId": cid(channel_num),
        "type": msg_type,
        "message": "revision validation",
        "props": { "categoryChain": category_chain },
    });
    if with_seq {
        data["event_seq"] = json!(seq);
    }
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({ "action": "post", "data": data }))
            .expect("post frame must serialize")
            .into(),
    ))
}

/// 构造 categoryChain type=2 编辑帧，覆盖 post_update 的专用 queue_edit 入口。
fn post_update_frame(channel_num: u64, seq: u64, revision: Option<Value>) -> Tick {
    let mut category_chain = json!({
        "chainId": "chain-invalid-revision",
        "status": "OPEN",
    });
    if let Some(revision) = revision {
        category_chain["revision"] = revision;
    }
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post_update",
            "data": {
                "id": sid(seq),
                "channelId": cid(channel_num),
                "event_seq": seq,
                "type": "CATEGORY_CHAIN",
                "message": "revision update",
                "props": { "categoryChain": category_chain },
            }
        }))
        .expect("post_update frame must serialize")
        .into(),
    ))
}

/// Construct the canonical posts_update batch used by category revoke/edit delivery.
fn posts_update_frame(channel_num: u64, seq: u64, msg_type: &str, revision: Option<Value>) -> Tick {
    let mut category_chain = json!({
        "chainId": "chain-invalid-revision",
        "status": "OPEN",
    });
    if let Some(revision) = revision {
        category_chain["revision"] = revision;
    }
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({
            "action": "posts_update",
            "data": {
                "channelId": cid(channel_num),
                "events": [{
                    "event_seq": seq,
                    "post": {
                        "id": sid(seq),
                        "channelId": cid(channel_num),
                        "type": msg_type,
                        "message": "posts update revision validation",
                        "props": { "categoryChain": category_chain },
                        "revoke": true,
                    }
                }]
            }
        }))
        .expect("posts_update frame must serialize")
        .into(),
    ))
}

/// Assert that an invalid authority frame produced no storage or renderer effect.
fn assert_silent(harness: &Harness, effects: &[common::EffectKind]) {
    assert!(
        effects.is_empty(),
        "invalid CATEGORY_CHAIN authority must be dropped before effects: {effects:?}"
    );
    assert!(
        harness.sink.as_slice().iter().all(|effect| {
            !matches!(
                effect,
                Effect::Persist { .. }
                    | Effect::PersistAtomic { .. }
                    | Effect::PersistFire { .. }
                    | Effect::Emit { .. }
            )
        }),
        "invalid CATEGORY_CHAIN authority must not write or emit: {:?}",
        harness.sink.as_slice()
    );
}

/// Construct an enveloped sync/notify response whose message map carries one revision value.
fn sync_reply(channel_num: u64, revision: Option<Value>) -> bytes::Bytes {
    let mut category_chain = json!({
        "chainId": "chain-invalid-revision",
        "status": "OPEN",
    });
    if let Some(revision) = revision {
        category_chain["revision"] = revision;
    }
    let msg_id = sid(1);
    let mut messages = serde_json::Map::new();
    messages.insert(
        msg_id.clone(),
        json!({
            "id": sid(1),
            "temporaryId": "tmp-sync-invalid-revision",
            "channelId": cid(channel_num),
            "type": "CATEGORY_CHAIN",
            "message": "sync revision validation",
            "props": { "categoryChain": category_chain },
        }),
    );
    let raw = json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": cid(channel_num),
                "kind": "events",
                "events": [{
                    "channelId": cid(channel_num),
                    "eventSeq": 1,
                    "eventType": 1,
                    "msgId": msg_id,
                }],
                "messages": Value::Object(messages),
                "nextSeq": 1,
            }],
            "serverTime": 1_700_000_000_i64,
        }
    });
    enveloped(bytes::Bytes::from(
        serde_json::to_vec(&raw).expect("sync response must serialize"),
    ))
}

/// Extract the only HTTP correlation emitted by an explicit sync command.
fn sync_http_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("explicit sync must emit an HTTP correlation")
}

/// Build the public sync command whose payload is normalized to Helix's snake_case core form.
fn explicit_sync_command(channel_num: u64) -> Tick {
    Tick::Command(
        helix_im::client_api::build_command(
            "im_sync_channels",
            json!({ "channelId": cid(channel_num), "reason": "invalid_revision" })
                .to_string()
                .as_bytes(),
        )
        .expect("sync command must normalize"),
    )
}

#[test]
fn malformed_live_category_revisions_are_dropped_without_cursor_consumption() {
    let malformed = [
        ("missing", None),
        ("null", Some(json!(null))),
        ("number", Some(json!(7))),
        ("bad_decimal_leading_zero", Some(json!("01"))),
        ("bad_decimal_fraction", Some(json!("1.0"))),
        ("bad_decimal_text", Some(json!("revision"))),
    ];

    for (label, revision) in malformed {
        let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
        let effects = harness.step(post_frame(1, 1, "CATEGORY_CHAIN", revision, true), 1_000);
        assert_silent(&harness, &effects);
        assert_eq!(
            harness.module.cursor_for(common::channel_id(1)),
            Some(0),
            "malformed revision case {label} must not consume cursor"
        );
    }
}

#[test]
fn malformed_unsequenced_category_revision_is_not_render_only_emitted() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let effects = harness.step(
        post_frame(1, 1, "CATEGORY_CHAIN", Some(json!("01")), false),
        1_000,
    );
    assert_silent(&harness, &effects);
    assert_eq!(harness.module.cursor_for(common::channel_id(1)), Some(0));
}

#[test]
fn malformed_category_revision_is_dropped_from_post_update_queue_edit() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let effects = harness.step(post_update_frame(1, 1, Some(json!("1.0"))), 1_000);
    assert_silent(&harness, &effects);
    assert_eq!(harness.module.cursor_for(common::channel_id(1)), Some(0));
}

#[test]
fn malformed_category_revision_is_dropped_before_posts_update_gate() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let effects = harness.step(
        posts_update_frame(1, 1, "CATEGORY_CHAIN", Some(json!("01"))),
        1_000,
    );
    assert_silent(&harness, &effects);
    assert_eq!(harness.module.cursor_for(common::channel_id(1)), Some(0));
}

#[test]
fn legal_same_seq_category_revision_recovers_after_invalid_live_frame() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let invalid = harness.step(
        post_frame(1, 1, "CATEGORY_CHAIN", Some(json!("01")), true),
        1_000,
    );
    assert_silent(&harness, &invalid);

    let legal = harness.step(
        post_frame(1, 1, "CATEGORY_CHAIN", Some(json!("1")), true),
        1_001,
    );
    let corr = legal.iter().find_map(|effect| match effect {
        common::EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    assert!(
        corr.is_some(),
        "legal same-seq revision must recover: {legal:?}"
    );
    assert_eq!(harness.module.cursor_for(common::channel_id(1)), Some(0));

    harness.step(port_reply_ok(corr.expect("persist correlation")), 1_002);
    assert_eq!(harness.module.cursor_for(common::channel_id(1)), Some(1));
}

#[test]
fn noncategory_post_with_category_like_invalid_revision_keeps_existing_write_path() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    let effects = harness.step(post_frame(1, 1, "TEXT", Some(json!(7)), true), 1_000);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, common::EffectKind::PersistAtomic(_))),
        "non-category post must retain its write path: {effects:?}"
    );
}

#[test]
fn malformed_sync_message_revision_fails_before_port_reply_effects() {
    let mut harness = Harness::new_with_identity(&[1], "user-a", "tenant-a", "", "", "");
    harness.step(hello_inbound("conn-invalid-revision"), 900);
    harness.step(explicit_sync_command(1), 901);
    let corr = sync_http_corr(&harness);

    harness.sink.clear();
    let mut sink = EffectSink::new();
    let result = harness.module.handle(
        &port_reply_with_body(corr, sync_reply(1, Some(json!("1.0")))),
        902,
        &mut sink,
    );
    assert!(result.is_err(), "invalid sync revision must fail closed");
    assert!(
        sink.as_slice().is_empty(),
        "failed sync must emit no effects"
    );
    assert_eq!(harness.module.cursor_for(common::channel_id(1)), Some(0));
}
