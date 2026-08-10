// 回归锚: Spec 047/T034-T035，G-05 转发与合并转发双视角终态。

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

use common::{enveloped, port_reply_with_body, Harness};

const TARGET_CHANNEL: &str = "chfixx00000000000000000005";
const SOURCE_CHANNEL: &str = "chfixx00000000000000000050";
const SOURCE_A: &str = "srvfix00000000000000000051";
const SOURCE_B: &str = "srvfix00000000000000000052";
const TARGET_INDIVIDUAL: &str = "srvfix00000000000000000053";
const TARGET_MERGED: &str = "srvfix00000000000000000054";
const SENDER_ID: &str = "g05-sender";
const RECEIVER_ID: &str = "g05-receiver";
const INDIVIDUAL_REQ: &str = "g05-forward-individual";
const MERGED_REQ: &str = "g05-forward-merged";
const CREATE_AT: i64 = 1_700_000_500_000;

type TestResult<T> = Result<T, String>;

struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造 public relay command，确保测试经过真实 client API alias。
fn relay_command(mode: &str, source_ids: &[&str], req_id: &str) -> TestResult<Tick> {
    let payload = serde_json::to_vec(&json!({
        "post_ids": source_ids,
        "target_channel_ids": [TARGET_CHANNEL],
        "mode": mode,
        "req_id": req_id,
    }))
    .map_err(|error| error.to_string())?;
    helix_im::client_api::build_command("im_relay_messages", &payload)
        .map(Tick::Command)
        .map_err(|error| error.to_string())
}

/// 构造本地 durable source row；转发只允许读取这些行。
fn source_row(id: &str, user_id: &str, author: &str, message: &str, create_at: i64) -> Value {
    json!({
        "id": id,
        "temporary_id": format!("source-{id}"),
        "channel_id": SOURCE_CHANNEL,
        "user_id": user_id,
        "type": "TEXT",
        "message": message,
        "simple_message": message,
        "props": "{}",
        "mentions": "[]",
        "viewers": "[\"all\"]",
        "user_snapshot": json!({"userName": author}).to_string(),
        "create_at": create_at,
    })
}

/// 从当前 Effect batch 读取指定类型的唯一 correlation。
fn correlation(effects: &[Effect], kind: &str) -> Option<Correlation> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("http", Effect::Http { corr, .. }) => Some(*corr),
        ("persist", Effect::Persist { corr, .. }) => Some(*corr),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(*corr),
        _ => None,
    })
}

/// 解出当前 batch 的全部 MessageV3 envelopes。
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

/// 断言 source 阶段只有按 id 的 message Get，不允许改写来源行。
fn assert_source_lookup(effects: &[Effect], source_id: &str) -> TestResult<Correlation> {
    let effect = effects
        .iter()
        .find_map(|effect| {
            let Effect::Persist { corr, ops } = effect else {
                return None;
            };
            Some((*corr, ops))
        })
        .ok_or_else(|| format!("missing source lookup for {source_id}"))?;
    assert_eq!(effect.1.len(), 1);
    assert!(matches!(
        &effect.1[0],
        StorageOp::Get(spec)
            if spec.table == "message"
                && spec.key_col == "id"
                && matches!(&spec.key_val, SqlValue::Text(value) if value == source_id)
    ));
    Ok(effect.0)
}

/// 构造 createPosts 的 host HTTP 成功信封。
fn acceptance_reply() -> TestResult<Bytes> {
    let raw = serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": {
            "targets": [{
                "channelId": TARGET_CHANNEL,
                "status": "accepted"
            }]
        }
    }))
    .map_err(|error| error.to_string())?;
    Ok(enveloped(Bytes::from(raw)))
}

/// 驱动一个 individual/merged command 到 HTTP admission，并返回实际发送的 post。
fn run_command(
    harness: &mut Harness,
    mode: &str,
    source_rows: &[Value],
    req_id: &str,
    now_ms: u64,
    ticks: &mut Vec<Value>,
) -> TestResult<(Value, Value)> {
    let source_ids = source_rows
        .iter()
        .filter_map(|row| row["id"].as_str())
        .collect::<Vec<_>>();
    harness.step(relay_command(mode, &source_ids, req_id)?, now_ms);
    ticks.push(json!({
        "kind": "Command",
        "value": {
            "name": "im_relay_messages",
            "post_ids": source_ids,
            "target_channel_ids": [TARGET_CHANNEL],
            "mode": mode,
            "req_id": req_id
        },
        "now_ms": now_ms
    }));

    for (index, row) in source_rows.iter().enumerate() {
        let source_id = row["id"]
            .as_str()
            .ok_or_else(|| "source row missing id".to_string())?;
        let corr = assert_source_lookup(harness.sink.as_slice(), source_id)?;
        let body = serde_json::to_vec(&json!([row])).map_err(|error| error.to_string())?;
        harness.step(
            port_reply_with_body(corr.raw(), Bytes::from(body)),
            now_ms + index as u64 + 1,
        );
        ticks.push(json!({
            "kind": "PortReply",
            "value": {
                "correlation": corr.raw(),
                "outcome": {"variant": "Ok", "rows": [row]}
            },
            "now_ms": now_ms + index as u64 + 1
        }));
    }

    if mode == "merged" {
        let corr = correlation(harness.sink.as_slice(), "persist")
            .ok_or_else(|| "merged forward missing source channel lookup".to_string())?;
        let body = serde_json::to_vec(&json!([{
            "id": SOURCE_CHANNEL,
            "display_name": "Gate Review Group"
        }]))
        .map_err(|error| error.to_string())?;
        harness.step(
            port_reply_with_body(corr.raw(), Bytes::from(body)),
            now_ms + source_rows.len() as u64 + 1,
        );
        ticks.push(json!({
            "kind": "PortReply",
            "value": {
                "correlation": corr.raw(),
                "outcome": {
                    "variant": "Ok",
                    "rows": [{"id": SOURCE_CHANNEL, "display_name": "Gate Review Group"}]
                }
            },
            "now_ms": now_ms + source_rows.len() as u64 + 1
        }));
    }

    let body = harness
        .last_http_body()
        .ok_or_else(|| format!("{mode} forward missing createPosts HTTP"))?;
    assert_eq!(body["channelIds"], json!([TARGET_CHANNEL]));
    let post = body["posts"]
        .as_array()
        .and_then(|posts| posts.first())
        .cloned()
        .ok_or_else(|| format!("{mode} forward missing target post"))?;
    let corr = correlation(harness.sink.as_slice(), "http")
        .ok_or_else(|| format!("{mode} forward missing HTTP correlation"))?;
    harness.step(
        port_reply_with_body(corr.raw(), acceptance_reply()?),
        now_ms + source_rows.len() as u64 + 2,
    );
    let batch = emitted_events(harness.sink.as_slice())
        .into_iter()
        .find(|event| event["event"] == "im:posts:batch-result")
        .ok_or_else(|| format!("{mode} forward missing batch result"))?;
    ticks.push(json!({
        "kind": "PortReply",
        "value": {
            "correlation": corr.raw(),
            "outcome": {
                "variant": "Ok",
                "authority": {
                    "targets": [{"channelId": TARGET_CHANNEL, "status": "accepted"}]
                }
            }
        },
        "now_ms": now_ms + source_rows.len() as u64 + 2
    }));
    Ok((post, batch))
}

/// 把 createPosts post 补成 Go WS `post` authority。
fn target_authority(mut post: Value, req_id: &str, id: &str, seq: u64, create_at: i64) -> Value {
    post["id"] = json!(id);
    post["channelId"] = json!(TARGET_CHANNEL);
    post["createAt"] = json!(create_at);
    post["updateAt"] = json!(create_at);
    json!({
        "action": "post",
        "tracing": {"csesTrackId": req_id},
        "data": {
            "event_seq": seq,
            "channelId": TARGET_CHANNEL,
            "post": post
        }
    })
}

/// 构造合并转发对外只读的 bounded detail，raw mergeMessage 不跨事件边界。
fn expected_forward_detail() -> Value {
    json!({
        "mode": "merged",
        "state": "ready",
        "title": "Gate Review Group的聊天记录",
        "summary": "共2条消息",
        "itemCount": 2,
        "sourceChannelId": SOURCE_CHANNEL,
        "sourceChannelName": "Gate Review Group",
        "items": [
            {
                "id": SOURCE_A,
                "userId": "source-user-a",
                "author": "Actor",
                "text": "Forwarded message A",
                "type": "TEXT",
                "createAt": CREATE_AT - 20,
                "userSnapshot": {"userName": "Actor"}
            },
            {
                "id": SOURCE_B,
                "userId": "source-user-b",
                "author": "Observer",
                "text": "Forwarded message B",
                "type": "TEXT",
                "createAt": CREATE_AT - 10,
                "userSnapshot": {"userName": "Observer"}
            }
        ],
        "failureReason": Value::Null
    })
}

/// 从 target authority 推导本地 channel/member 应持久化的 lastPost。
fn expected_last_post(authority: &Value) -> TestResult<Value> {
    let post = authority
        .pointer("/data/post")
        .ok_or_else(|| "authority missing post".to_string())?;
    let mut last_post = json!({
        "id": post["id"],
        "userId": post["userId"],
        "message": post["message"],
        "createAt": post["createAt"],
    });
    // 当前 authority 会把服务端 userSnapshot 原样带入频道 lastPost，保持渲染所需身份快照。
    if post.get("userSnapshot").is_some() {
        last_post["userSnapshot"] = post["userSnapshot"].clone();
    }
    if post["type"] == "MULTIPLY" {
        last_post["type"] = json!("MULTIPLY");
        last_post["forwardDetail"] = expected_forward_detail();
    }
    Ok(last_post)
}

/// 断言 target message 与 forward detail 进入同一原子写集，source row 未被当作目标更新。
fn assert_target_commit(
    effects: &[Effect],
    authority: &Value,
    expected_last_post: &Value,
) -> TestResult<Correlation> {
    let (corr, ops) = effects
        .iter()
        .find_map(|effect| {
            let Effect::PersistAtomic { corr, ops } = effect else {
                return None;
            };
            Some((*corr, ops))
        })
        .ok_or_else(|| "target authority missing atomic persist".to_string())?;
    let target_id = authority["data"]["post"]["id"]
        .as_str()
        .ok_or_else(|| "target authority missing id".to_string())?;
    let message_ids = ops.iter().filter_map(|op| match op {
        StorageOp::BatchUpsert(spec) if spec.table == "message" => {
            spec.rows.first().and_then(|row| {
                row.iter().find_map(|(column, value)| {
                    (column == "id")
                        .then_some(value)
                        .and_then(|value| match value {
                            SqlValue::Text(value) => Some(value.as_str()),
                            _ => None,
                        })
                })
            })
        }
        _ => None,
    });
    assert_eq!(message_ids.collect::<Vec<_>>(), [target_id]);
    let stored_last_post = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "channel" => {
            spec.patch.iter().find_map(|(column, value)| {
                (column == "last_post")
                    .then_some(value)
                    .and_then(|value| match value {
                        SqlValue::Text(value) => serde_json::from_str::<Value>(value).ok(),
                        _ => None,
                    })
            })
        }
        _ => None,
    });
    assert_eq!(stored_last_post.as_ref(), Some(expected_last_post));
    Ok(corr)
}

/// 编码当前 viewer 对 target channel 的绝对态读回。
fn member_reply(viewer_id: &str, unread_count: i64, last_post: &Value) -> ReplyBytes {
    let row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(TARGET_CHANNEL.to_string()),
        ),
        ("user_id".to_string(), SqlValue::Text(viewer_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
        (
            "last_post_at".to_string(),
            SqlValue::Integer(last_post["createAt"].as_i64().unwrap_or_default()),
        ),
    ];
    ReplyBytes(rows_to_reply_bytes(&[row]))
}

/// 对一个 persona 应用 target WS authority，严格经过 PersistAtomic→readback→Emit。
fn apply_target(
    harness: &mut Harness,
    viewer_id: &str,
    authority: &Value,
    unread_count: i64,
    now_ms: u64,
    ticks: &mut Vec<Value>,
) -> TestResult<Vec<Value>> {
    let inbound = serde_json::to_vec(authority).map_err(|error| error.to_string())?;
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound))), now_ms);
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let last_post = expected_last_post(authority)?;
    let persist_corr = assert_target_commit(harness.sink.as_slice(), authority, &last_post)?;
    ticks.push(json!({"kind": "Inbound", "value": authority, "now_ms": now_ms}));

    harness.step(
        Tick::PortReply {
            corr: persist_corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        now_ms + 1,
    );
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "target commit missing member readback".to_string())?;
    ticks.push(json!({
        "kind": "PortReply",
        "value": {"correlation": persist_corr.raw(), "outcome": {"variant": "Ok"}},
        "now_ms": now_ms + 1
    }));

    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, unread_count, &last_post)),
        },
        now_ms + 2,
    );
    ticks.push(json!({
        "kind": "PortReply",
        "value": {
            "correlation": readback_corr.raw(),
            "outcome": {
                "variant": "Ok",
                "rows": [{
                    "channel_id": TARGET_CHANNEL,
                    "user_id": viewer_id,
                    "unread_count": unread_count,
                    "last_post": last_post
                }]
            }
        },
        "now_ms": now_ms + 2
    }));
    Ok(emitted_events(harness.sink.as_slice()))
}

/// 回放 sender 的两个 command 及双方各自的 target authority。
fn run_gate() -> TestResult<GateTrace> {
    let row_a = source_row(
        SOURCE_A,
        "source-user-a",
        "Actor",
        "Forwarded message A",
        CREATE_AT - 20,
    );
    let row_b = source_row(
        SOURCE_B,
        "source-user-b",
        "Observer",
        "Forwarded message B",
        CREATE_AT - 10,
    );
    let mut sender = Harness::new_with_identity(&[5], SENDER_ID, "g05-company", "Sender", "", "");
    let mut sender_ticks = Vec::new();
    let mut sender_events = Vec::new();

    let (individual_post, individual_batch) = run_command(
        &mut sender,
        "individual",
        std::slice::from_ref(&row_a),
        INDIVIDUAL_REQ,
        CREATE_AT as u64,
        &mut sender_ticks,
    )?;
    sender_events.push(individual_batch);
    let individual_authority = target_authority(
        individual_post,
        INDIVIDUAL_REQ,
        TARGET_INDIVIDUAL,
        1,
        CREATE_AT + 1,
    );
    sender_events.extend(apply_target(
        &mut sender,
        SENDER_ID,
        &individual_authority,
        0,
        CREATE_AT as u64 + 10,
        &mut sender_ticks,
    )?);

    let (merged_post, merged_batch) = run_command(
        &mut sender,
        "merged",
        &[row_a, row_b],
        MERGED_REQ,
        CREATE_AT as u64 + 20,
        &mut sender_ticks,
    )?;
    assert_eq!(merged_post["type"], "MULTIPLY");
    sender_events.push(merged_batch);
    let merged_authority =
        target_authority(merged_post, MERGED_REQ, TARGET_MERGED, 2, CREATE_AT + 2);
    sender_events.extend(apply_target(
        &mut sender,
        SENDER_ID,
        &merged_authority,
        0,
        CREATE_AT as u64 + 30,
        &mut sender_ticks,
    )?);

    let mut receiver =
        Harness::new_with_identity(&[5], RECEIVER_ID, "g05-company", "Receiver", "", "");
    let mut receiver_ticks = Vec::new();
    let mut receiver_events = apply_target(
        &mut receiver,
        RECEIVER_ID,
        &individual_authority,
        1,
        CREATE_AT as u64 + 40,
        &mut receiver_ticks,
    )?;
    receiver_events.extend(apply_target(
        &mut receiver,
        RECEIVER_ID,
        &merged_authority,
        2,
        CREATE_AT as u64 + 50,
        &mut receiver_ticks,
    )?);

    Ok(GateTrace {
        sender: PersonaTrace {
            events: sender_events,
            ticks: sender_ticks,
        },
        receiver: PersonaTrace {
            events: receiver_events,
            ticks: receiver_ticks,
        },
    })
}

/// G-05 source 不变，两个 target identity 与双 persona channel 绝对态独立收敛。
#[test]
fn g05_forwards_individual_and_merged_posts_without_rewriting_sources() -> TestResult<()> {
    let trace = run_gate()?;
    assert_eq!(
        trace
            .sender
            .events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        [
            "im:posts:batch-result",
            "im:post:received",
            "im:channel:update",
            "im:posts:batch-result",
            "im:post:received",
            "im:channel:update",
        ]
    );
    assert_eq!(
        trace
            .receiver
            .events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        [
            "im:post:received",
            "im:channel:update",
            "im:post:received",
            "im:channel:update",
        ]
    );
    for batch in [&trace.sender.events[0], &trace.sender.events[3]] {
        assert_eq!(batch["data"]["batchStatus"], "success");
        assert_eq!(batch["data"]["targets"][0]["acceptanceStatus"], "accepted");
        assert_eq!(batch["data"]["targets"][0]["deliveryStatus"], "pending");
    }
    assert_eq!(trace.sender.events[1]["data"]["id"], TARGET_INDIVIDUAL);
    assert_eq!(trace.sender.events[4]["data"]["id"], TARGET_MERGED);
    assert_eq!(trace.receiver.events[0]["data"]["id"], TARGET_INDIVIDUAL);
    assert_eq!(trace.receiver.events[2]["data"]["id"], TARGET_MERGED);
    assert_eq!(
        trace.sender.events[4]["data"]["forwardDetail"],
        expected_forward_detail()
    );
    assert!(trace.sender.events[4]["data"].get("props").is_none());
    assert_eq!(
        trace.sender.events[5]["data"]["lastPost"]["forwardDetail"],
        expected_forward_detail()
    );
    assert_eq!(trace.sender.events[2]["data"]["unreadCount"], 0);
    assert_eq!(trace.sender.events[5]["data"]["unreadCount"], 0);
    assert_eq!(trace.receiver.events[1]["data"]["unreadCount"], 1);
    assert_eq!(trace.receiver.events[3]["data"]["unreadCount"], 2);
    let encoded = serde_json::to_string(&trace.sender.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "actionCapabilities",
        "InteractionFrame",
        "Surface",
        "temporary_id",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-05 sender/receiver canonical artifact。
#[test]
fn g05_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-05",
        "message_v3_g05",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "viewers", "create_at"]},
            {"table": "channel_event", "columns": ["channel_id", "event_seq", "event_type", "msg_id", "actor_id", "create_at", "payload"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "send_status", "create_at", "event_seq"]},
            {"table": "channel", "columns": ["id", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "send_status", "create_at", "event_seq"]},
            {"table": "channel", "columns": ["id", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
