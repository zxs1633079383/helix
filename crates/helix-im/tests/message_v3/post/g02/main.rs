// 回归锚: Spec 047/T026-T027，G-02 文档消息双 persona 与 channel 文档预览。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::Harness;

const CHANNEL_ID: &str = "chdocx00000000000000000001";
const POST_ID: &str = "srvdoc00000000000000000001";
const TEMPORARY_ID: &str = "tmpdoc00000000000000000001";
const ACTOR_ID: &str = "g02-actor";
const RECEIVER_ID: &str = "g02-receiver";
const DOCUMENT_URL: &str = "http://doc-pre.cses7.com/doc/review-g02";
const DOCUMENT_TITLE: &str = "打开文档";
const MESSAGE: &str = "【文档】 http://doc-pre.cses7.com/doc/review-g02";
const CREATE_AT: i64 = 1_700_000_100_002;

type TestResult<T> = Result<T, String>;

struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 构造含真实 document props 的 Go `post` authority。
fn authority() -> Value {
    json!({
        "action": "post",
        "data": {
            "channelId": CHANNEL_ID,
            "event_seq": 1,
            "post": {
                "id": POST_ID,
                "temporaryId": TEMPORARY_ID,
                "channelId": CHANNEL_ID,
                "userId": ACTOR_ID,
                "type": "DOCUMENT",
                "message": MESSAGE,
                "simpleMessage": MESSAGE,
                "props": {
                    "document": {
                        "version": 1,
                        "url": DOCUMENT_URL,
                        "title": DOCUMENT_TITLE,
                        "placeholder": true
                    }
                },
                "viewers": ["all"],
                "createAt": CREATE_AT,
                "updateAt": CREATE_AT
            }
        }
    })
}

/// 从当前 Effect batch 读取唯一相关持久请求。
fn correlated_persist(effects: &[Effect], atomic: bool) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match (atomic, effect) {
        (false, Effect::Persist { corr, ops }) => Some((*corr, ops.as_slice())),
        (true, Effect::PersistAtomic { corr, ops }) => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 编码当前 viewer 的 channel_member 文档预览绝对态。
fn member_reply(user_id: &str, unread_count: i64, last_post: &Value) -> ReplyBytes {
    let row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("user_id".to_string(), SqlValue::Text(user_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ];
    ReplyBytes(rows_to_reply_bytes(&[row]))
}

/// 从当前 Effect batch 解出生产 MessageV3 envelopes。
fn emitted_events(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice(event.0.as_ref()).ok()
        })
        .collect()
}

/// 回放一个独立 persona 的文档 authority、提交屏障和绝对态读回。
fn run_persona(viewer_id: &str, unread_count: i64) -> TestResult<PersonaTrace> {
    let inbound = authority();
    let bytes = serde_json::to_vec(&inbound).map_err(|error| error.to_string())?;
    // The contract channel is deliberately not `cid(1)`; seed the exact persisted
    // channel row instead of letting the numeric fixture silently authorize another id.
    let mut harness = Harness::new_with_identity(&[], viewer_id, "g02-company", "", "", "");
    harness.seed_channel_scope(CHANNEL_ID, "g02-company", viewer_id);
    harness.sink.clear();
    harness.effect_log.clear();
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(bytes))),
        CREATE_AT as u64,
    );
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    let (persist_corr, writes) = correlated_persist(harness.sink.as_slice(), true)
        .ok_or_else(|| "missing G-02 atomic persistence".to_string())?;
    assert!(writes
        .iter()
        .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")));

    harness.step(
        Tick::PortReply {
            corr: persist_corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        CREATE_AT as u64 + 1,
    );
    let (readback_corr, readback_ops) = correlated_persist(harness.sink.as_slice(), false)
        .ok_or_else(|| "missing G-02 scoped readback".to_string())?;
    assert!(readback_ops
        .iter()
        .any(|op| matches!(op, StorageOp::ScopedGet(spec) if spec.table == "channel_member")));
    let final_unread = if viewer_id == ACTOR_ID {
        0
    } else {
        unread_count + 1
    };
    let last_post = json!({
        "id": POST_ID,
        "userId": ACTOR_ID,
        "message": MESSAGE,
        "type": "DOCUMENT",
        "props": authority()["data"]["post"]["props"].clone(),
        "createAt": CREATE_AT
    });
    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, final_unread, &last_post)),
        },
        CREATE_AT as u64 + 2,
    );
    Ok(PersonaTrace {
        events: emitted_events(harness.sink.as_slice()),
        ticks: vec![
            json!({"kind": "Inbound", "value": inbound, "now_ms": CREATE_AT}),
            json!({"kind": "PortReply", "value": {"correlation": persist_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 1}),
            json!({"kind": "PortReply", "value": {"correlation": readback_corr.raw(), "outcome": {"variant": "Ok", "rows": [{"channel_id": CHANNEL_ID, "user_id": viewer_id, "unread_count": final_unread, "last_post": last_post}]}}, "now_ms": CREATE_AT + 2}),
        ],
    })
}

/// G-02 双视角都保留 document 数据，channel preview 指向同一文档消息。
#[test]
fn g02_emits_document_post_and_channel_preview_for_both_personas() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, 7)?;
    let receiver = run_persona(RECEIVER_ID, 4)?;
    for trace in [&sender, &receiver] {
        assert_eq!(trace.events.len(), 2);
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[0]["data"]["type"], "DOCUMENT");
        assert_eq!(
            trace.events[0]["data"]["props"]["document"]["url"],
            DOCUMENT_URL
        );
        assert_eq!(
            trace.events[0]["data"]["props"]["document"]["title"],
            DOCUMENT_TITLE
        );
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[1]["data"]["lastPost"]["type"], "DOCUMENT");
        assert_eq!(
            trace.events[1]["data"]["lastPost"]["props"]["document"]["url"],
            DOCUMENT_URL
        );
    }
    assert_eq!(sender.events[1]["data"]["unreadCount"], 0);
    assert_eq!(receiver.events[1]["data"]["unreadCount"], 5);
    Ok(())
}

/// Rust producer 在生成模式下写出 G-02 双视角 canonical artifact。
#[test]
fn g02_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, 7)?;
    let receiver = run_persona(RECEIVER_ID, 4)?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-02",
        "message_v3_g02",
        sender.ticks,
        sender.events,
        receiver.events,
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "send_status", "create_at", "event_seq"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "send_status", "create_at", "event_seq"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
