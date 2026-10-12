//! MV3-G01f｜发送 DOCUMENT。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g01f/`。
//!
//! 三个面（对应 T052「DOCUMENT props、三态、双 persona」）：
//!
//! 1. **DOCUMENT props**：`props.document{version,snapshot}` 由 Helix 权威构造，合法 sibling
//!    props 与 document 内扩展字段一律不删（#2..#4），并与 `outbound-sample.json` 逐字节一致（#5）。
//! 2. **三态**（INV-03）：DOCUMENT 不走上传链路，`sending` → HTTP 2xx **仍是 sending** →
//!    WS echo 才结算 `sent`，且 `props.document` 一路无损（#6）。
//! 3. **双 persona**（INV-12）：同一条 DOCUMENT 权威帧在 sender / receiver 独立回放，
//!    `props.document` 是共享消息事实、未读是 viewer-local（#7）。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use helix_im::state::SendStatus;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    enveloped, port_reply_ok, port_reply_with_body, send_message_cmd_json, EffectKind, Harness,
};

/// 参考包 `deterministic_ids`。
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const COMPANY_ID: &str = "company-phase1";
const DOCUMENT_TEXT: &str = "Phase 1 document";
const POST_ID: &str = "srvfix00000000000000000001";
const NOW_MS: u64 = 1_700_000_000_000;
const CREATE_AT: i64 = 1_700_000_000_001;

/// 返回首个指定类别 effect 的 correlation。
fn correlation_of(effects: &[EffectKind], kind: &str) -> u64 {
    effects
        .iter()
        .find_map(|effect| match (kind, effect) {
            ("persist", EffectKind::Persist(corr)) | ("http", EffectKind::Http(corr)) => {
                Some(*corr)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {kind} correlation in {effects:?}"))
}

/// 读取实际 posts/create effect 的结构化 JSON body。
fn posts_create_body(effects: &[Effect]) -> Value {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/create") => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("missing posts/create body")
}

/// 证明 MV3-G01f 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01f", "post/mv3-g01f");
}

/// 发一条 DOCUMENT 并取回 posts/create body（入参是**未规范化的原始前端意图**）。
fn document_posts_create_body(text: &str, props: Value) -> Value {
    let mut harness = Harness::new_with_identity(
        &[1],
        "user-actor-444",
        "company-phase1",
        "发送者",
        "组织",
        "部门",
    );
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "DOCUMENT",
            "text": text,
            "props": props,
            "viewers": ["all"],
            "mentions": []
        })),
        1_700_000_000_000,
    );
    let after_persist = harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    assert!(
        after_persist
            .iter()
            .any(|effect| matches!(effect, EffectKind::Http(_))),
        "DOCUMENT 不走上传链路，乐观落库回执后应直接出站 posts/create"
    );
    posts_create_body(harness.sink.as_slice())
}

/// DOCUMENT 的 `props.document` 由 Helix 权威构造：前端不传 `version`，wire body 仍必须
/// 带 `version=1`（参考包 post/mv3-g01f `outbound-sample.json`）；同时**不删除**任何合法
/// sibling props 与 document 内的扩展字段（T053）。
///
/// 纯透传实现下 `version` 会缺席，本用例即失败。
#[test]
fn document_version_is_authored_by_helix_and_siblings_survive() {
    let body = document_posts_create_body(
        "Phase 1 document",
        json!({
            "document": {
                "snapshot": {
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "正文"}]
                    }]
                },
                "extension": {"keep": true}
            },
            "legalSibling": {"keep": "unchanged"}
        }),
    );

    assert_eq!(body["type"], "DOCUMENT");
    assert_eq!(body["message"], "Phase 1 document");
    assert_eq!(
        body["props"]["document"]["version"], 1,
        "version 是 Helix 权威常量，前端不提交也必须出现在 wire"
    );
    // 前端给的快照与扩展字段一字不改地保留。
    assert_eq!(
        body["props"]["document"]["snapshot"]["content"][0]["content"][0]["text"],
        "正文"
    );
    assert_eq!(body["props"]["document"]["extension"]["keep"], true);
    assert_eq!(body["props"]["legalSibling"], json!({"keep": "unchanged"}));
    assert_eq!(body["viewers"], json!(["all"]));
    assert_eq!(body["mentions"], json!([]));
    assert!(body["temporaryId"]
        .as_str()
        .is_some_and(|temporary_id| !temporary_id.is_empty()));
    assert_eq!(body["createAt"], 1_700_000_000_000_u64);
}

/// 前端连 `props.document` 都没提交（只有 sibling props）时，Helix 仍必须构造出完整
/// `props.document`（空快照 + version=1），并给空文档摘要兜底 `[文档]`。
#[test]
fn document_props_are_constructed_when_client_submits_none() {
    let body = document_posts_create_body("", json!({"legalSibling": {"keep": "unchanged"}}));

    assert_eq!(
        body["props"]["document"],
        json!({ "version": 1, "snapshot": { "type": "doc", "content": [] } }),
        "props.document 必须由 Helix 构造，而不是要求前端自造 wire 形态"
    );
    assert_eq!(
        body["props"]["legalSibling"],
        json!({"keep": "unchanged"}),
        "构造 document 不得吞掉合法 sibling props"
    );
    assert_eq!(body["simpleMessage"], "[文档]");
}

/// 非法/脏 `version` 由 Helix 归一为权威常量 1（零信任：不把前端脏值透传给 Go）。
#[test]
fn document_version_is_normalized_from_untrusted_client_value() {
    let body = document_posts_create_body(
        "Phase 1 document",
        json!({"document": {"snapshot": {"type": "doc", "content": []}, "version": "1"}}),
    );

    assert_eq!(body["props"]["document"]["version"], 1);
}

// ─────────────── #5 出站载荷 ↔ 权威样本 byte-level parity ───────────────

/// 权威 DOCUMENT props（参考包 `inbound-sample.json` 的客户端意图形态）。
fn document_props() -> Value {
    json!({ "document": { "snapshot": { "type": "doc", "content": [] }, "version": 1 } })
}

/// 读取本 Gate 的权威出站样本（线型权威：`gates/README.md`）。
fn outbound_sample() -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root().join("post/mv3-g01f/outbound-sample.json"),
    )
}

/// 取样本中指定 `(event, viewer)` 的 `rust.data`。
fn reference_data(sample: &Value, event: &str, viewer: &str) -> Value {
    sample["samples"]
        .as_array()
        .expect("outbound-sample.samples must be an array")
        .iter()
        .find(|entry| entry["rust"]["event"] == event && entry["viewer"] == viewer)
        .unwrap_or_else(|| panic!("outbound-sample must declare {event} for viewer {viewer}"))
        ["rust"]["data"]
        .clone()
}

/// 把样本里的确定性占位 id 替换成本 Harness 的真实 id（其余字段逐字保留）。
fn substitute_reference_ids(value: &Value, channel_id: &str, temporary_id: &str) -> Value {
    match value {
        Value::String(text) => match text.as_str() {
            "channel-phase1-001" => json!(channel_id),
            "tmp-mv3-g01f-001" => json!(temporary_id),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_reference_ids(item, channel_id, temporary_id))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.clone(),
                        substitute_reference_ids(item, channel_id, temporary_id),
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// 从当前 effect batch 解码指定 MessageV3 事件。
fn emitted_event(effects: &[Effect], name: &str) -> Option<Value> {
    effects.iter().find_map(|effect| {
        let Effect::Emit { event } = effect else {
            return None;
        };
        let value = serde_json::from_slice::<Value>(event.0.as_ref()).ok()?;
        (value["event"] == name).then_some(value)
    })
}

/// 从当前 Effect batch 解出全部 MessageV3 envelope。
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

/// 发一条 DOCUMENT 并推进到出站，返回 `(harness, temporaryId, http_corr, timer_id)`。
fn armed_document_send(sender_id: &str) -> (Harness, String, u64, u64) {
    let mut harness = Harness::new_with_identity(&[1], sender_id, COMPANY_ID, "", "", "");
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "DOCUMENT",
            "text": DOCUMENT_TEXT,
            "props": document_props(),
            "viewers": ["all"],
            "mentions": []
        })),
        NOW_MS,
    );
    let persist_corr = correlation_of(&initial, "persist");
    let armed = harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
    let http_corr = armed
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(*corr),
            _ => None,
        })
        .expect("DOCUMENT 不走上传链路，PersistOk 后应直接出站 posts/create");
    let timer_id = armed
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("posts/create must arm the echo timeout");
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic sending projection");
    let temporary_id = sending["data"]["temporaryId"]
        .as_str()
        .expect("sending projection must carry temporaryId")
        .to_string();
    (harness, temporary_id, http_corr, timer_id)
}

/// **参考包 parity 锁**：DOCUMENT 的 `im:post:sending` 载荷必须与 `outbound-sample.json`
/// 逐字节一致——含 `props.document{snapshot,version}` 整棵子树。
#[test]
fn sending_projection_matches_the_authoritative_outbound_sample() {
    let expected_raw = reference_data(&outbound_sample(), "im:post:sending", "sender");
    let expected_fields = expected_raw
        .as_object()
        .expect("sample im:post:sending data must be an object");
    // 防呆：样本被清空 / 丢掉 document 子树时，本测试会退化为 vacuous 断言。
    assert_eq!(expected_fields.get("type"), Some(&json!("DOCUMENT")));
    assert_eq!(expected_fields["props"]["document"]["version"], json!(1));
    assert!(expected_fields.len() >= 9);

    let (harness, temporary_id, _http, _timer) = armed_document_send(ACTOR_ID);
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic sending projection");
    let actual = sending["data"]
        .as_object()
        .expect("emitted im:post:sending data must be an object")
        .clone();

    let expected = substitute_reference_ids(&expected_raw, &common::cid(1), &temporary_id);
    let expected = expected
        .as_object()
        .expect("substitution preserves the object shape")
        .clone();

    let mut actual_subset = serde_json::Map::new();
    for key in expected.keys() {
        let value = actual.get(key).unwrap_or_else(|| {
            panic!("emitted im:post:sending 缺少样本声明的字段 `{key}`；实测字段 {actual:?}")
        });
        actual_subset.insert(key.clone(), value.clone());
    }
    assert_eq!(
        serde_json::to_string(&Value::Object(actual_subset)).unwrap(),
        serde_json::to_string(&Value::Object(expected)).unwrap(),
        "im:post:sending 载荷与 outbound-sample.json 不一致（参考包是权威，只能改实现）"
    );
}

// ───────────────────────── #6 三态 ─────────────────────────

/// 权威 DOCUMENT `post` 帧（真 Go wire：`action=post`，顶层 `event_seq`）。
fn authority_frame(temporary_id: &str, seq: u64) -> Tick {
    let frame = json!({
        "action": "post",
        "data": {
            "id": POST_ID,
            "temporaryId": temporary_id,
            "channelId": common::cid(1),
            "event_seq": seq,
            "userId": ACTOR_ID,
            "type": "DOCUMENT",
            "message": DOCUMENT_TEXT,
            "simpleMessage": DOCUMENT_TEXT,
            "props": document_props(),
            "viewers": ["all"],
            "mentions": [],
            "createAt": CREATE_AT,
            "updateAt": CREATE_AT
        }
    });
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&frame).expect("authority frame must serialize"),
    )))
}

/// 编码当前 viewer 的 channel_member 绝对态读回。
fn member_reply(user_id: &str, unread_count: i64, temporary_id: &str) -> Bytes {
    let last_post = json!({
        "id": POST_ID,
        "temporaryId": temporary_id,
        "userId": ACTOR_ID,
        "message": DOCUMENT_TEXT,
        "createAt": CREATE_AT
    });
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(common::cid(1))),
        ("user_id".to_string(), SqlValue::Text(user_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
        ("last_post_at".to_string(), SqlValue::Integer(CREATE_AT)),
        (
            "last_root_post_at".to_string(),
            SqlValue::Integer(CREATE_AT),
        ),
    ];
    rows_to_reply_bytes(&[row])
}

/// 取本批次首个 `Effect::PersistAtomic`（MessageV3 提交事务）的写集。
fn atomic_ops(effects: &[Effect]) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, ops } => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 取指定表 upsert 行内某列的文本值。
fn upsert_text(ops: &[StorageOp], table: &str, column: &str) -> Option<String> {
    ops.iter().find_map(|op| match op {
        StorageOp::BatchUpsert(spec) if spec.table == table => spec.rows.first().and_then(|row| {
            row.iter().find_map(|(name, value)| match value {
                SqlValue::Text(text) if name == column => Some(text.clone()),
                _ => None,
            })
        }),
        _ => None,
    })
}

/// DOCUMENT 的三态：HTTP 2xx 只是 admission，WS echo 才结算 `sent`，
/// 且 `props.document` 从乐观落库到终态投影一路无损（INV-03 + AS-2.10）。
#[test]
fn document_stays_sending_through_http_admission_and_keeps_props_until_sent() {
    let (mut harness, temporary_id, http_corr, timer_id) = armed_document_send(ACTOR_ID);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending)
    );

    // ① HTTP 2xx 只承认入站。
    let admitted = harness.step(
        port_reply_with_body(
            http_corr,
            enveloped(Bytes::from_static(
                br#"{"status":"SUCCESS","data":{"id":"srvfix00000000000000000001"}}"#,
            )),
        ),
        NOW_MS + 2,
    );
    assert!(
        admitted.is_empty(),
        "DOCUMENT 的 HTTP admission 不得额外查询、写终态或取消 timer，实测 {admitted:?}"
    );
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending),
        "2xx 之后仍必须是 sending（INV-03）"
    );

    // ② WS echo 才是权威。
    let echoed = harness.step(authority_frame(&temporary_id, 1), NOW_MS + 3);
    assert!(
        echoed
            .iter()
            .any(|effect| matches!(effect, EffectKind::CancelTimer(id) if *id == timer_id)),
        "echo 必须取消 echo timeout"
    );
    let (atomic_corr, writes) =
        atomic_ops(harness.sink.as_slice()).expect("echo authority must enter atomic commit");
    let persisted_props =
        upsert_text(writes, "message", "props").expect("commit 必须把 props 落进 message 行");
    assert_eq!(
        serde_json::from_str::<Value>(&persisted_props).expect("props 列必须是合法 JSON")
            ["document"]["version"],
        json!(1),
        "props.document 必须无损落库，实测 {persisted_props}"
    );

    // ③ 读回落定才发终态，且 props.document 出现在 im:post:received。
    let after_commit = harness.step(port_reply_ok(atomic_corr.raw()), NOW_MS + 4);
    let readback_corr = after_commit
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("commit 成功后必须读回当前 viewer 的 channel_member");
    harness.step(
        port_reply_with_body(readback_corr, member_reply(ACTOR_ID, 0, &temporary_id)),
        NOW_MS + 5,
    );
    let received = emitted_event(harness.sink.as_slice(), "im:post:received")
        .expect("read-back 落定后必须发布 im:post:received");
    assert_eq!(received["data"]["sendStatus"], "sent");
    assert_eq!(received["data"]["type"], "DOCUMENT");
    assert_eq!(
        received["data"]["props"]["document"],
        document_props()["document"],
        "DOCUMENT 终态投影必须完整携带 props.document（AS-2.10）"
    );
    assert!(received["data"].get("readBits").is_none());
}

// ───────────────────────── #7 双 persona ─────────────────────────

struct PersonaTrace {
    events: Vec<Value>,
    persisted_props: String,
    unread_bumped: bool,
}

/// 以独立本地库回放一个 persona 的 DOCUMENT 摄取（`initial_unread` = 该 persona 自己库里的
/// 发送前未读初值，终值由各自本地库独立推进，绝不从对端复制）。
fn run_persona(viewer_id: &str, initial_unread: i64) -> PersonaTrace {
    let temporary_id = "tmpfix00000000000000000001";
    let mut harness = Harness::new_with_identity(&[1], viewer_id, COMPANY_ID, "", "", "");
    harness.step(authority_frame(temporary_id, 1), CREATE_AT as u64);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "{viewer_id}: 摄取批次不得在 read-back 之前 Emit"
    );
    let (atomic_corr, writes) = atomic_ops(harness.sink.as_slice())
        .unwrap_or_else(|| panic!("{viewer_id}: 可见 DOCUMENT 必须进入 MessageV3 提交事务"));
    let persisted_props = upsert_text(writes, "message", "props")
        .unwrap_or_else(|| panic!("{viewer_id}: message 行必须落 props 列"));
    let unread_bumped = writes.iter().any(|op| {
        matches!(op, StorageOp::ScopedGuardedBump(spec)
            if spec.table == "channel_member"
                && spec.bump_col == "unread_count"
                && spec.bump_delta > 0)
    });

    let after_commit = harness.step(port_reply_ok(atomic_corr.raw()), CREATE_AT as u64 + 1);
    let readback_corr = after_commit
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{viewer_id}: commit 后必须读回本 viewer 的 channel_member"));
    let final_unread = if viewer_id == ACTOR_ID {
        0
    } else {
        initial_unread + 1
    };
    harness.step(
        port_reply_with_body(
            readback_corr,
            member_reply(viewer_id, final_unread, temporary_id),
        ),
        CREATE_AT as u64 + 2,
    );
    PersonaTrace {
        events: emitted_events(harness.sink.as_slice()),
        persisted_props,
        unread_bumped,
    }
}

/// **双 persona**（INV-12）：`props.document` 是共享消息事实，未读是 viewer-local。
///
/// sender 与 receiver 用各自的 Harness / 身份 / 初始未读独立回放同一条 DOCUMENT 帧：
/// 两侧落库 props 逐字节相同、终态投影的 `props.document` 相同，但未读写形态与绝对值不同。
#[test]
fn dual_persona_replay_shares_document_props_and_splits_unread() {
    let sender = run_persona(ACTOR_ID, 0);
    let receiver = run_persona(RECEIVER_ID, 3);

    for (who, trace) in [("sender", &sender), ("receiver", &receiver)] {
        assert_eq!(
            trace.events.len(),
            2,
            "{who}: 读回落定后恰好 received + channel:update，实测 {:?}",
            trace.events
        );
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[0]["data"]["type"], "DOCUMENT");
        assert_eq!(
            trace.events[0]["data"]["props"]["document"],
            document_props()["document"],
            "{who}: DOCUMENT 投影必须携带完整 props.document"
        );
    }

    assert_eq!(
        sender.persisted_props, receiver.persisted_props,
        "props 是消息级共享事实，不得随 viewer 变化"
    );
    assert!(!sender.unread_bumped, "发布者不得给自己涨未读");
    assert!(receiver.unread_bumped, "接收者未读必须 +1");
    assert_eq!(sender.events[1]["data"]["unreadCount"], 0);
    assert_eq!(receiver.events[1]["data"]["unreadCount"], 4);
    assert_ne!(
        sender.events[1]["data"]["unreadCount"], receiver.events[1]["data"]["unreadCount"],
        "两个视角的未读绝对值必须各算各的，禁止跨视角复制"
    );
}
