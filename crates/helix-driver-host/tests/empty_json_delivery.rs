//! 空 JSON 字段的真实 `ImModule` → HostStorage SQLite 回归。
//!
//! 每个用例都经过公开 `Tick`/`PortReply` seam；HTTP 只回灌隔离的 ACK 信封，不访问网络。

#[path = "../../helix-im/tests/common.rs"]
mod common;

use bytes::Bytes;
use common::{enveloped, sid};
use helix_core::effect::{GetSpec, Row, SqlValue, UpsertSpec};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::ports::Storage;
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde_json::{json, Value};

const CHANNEL: &str = "chfixx00000000000000000001";
const RECEIVER: &str = "receiver";
const OUTSIDER: &str = "outsider";
const COMPANY: &str = "company";

struct Fixture {
    db: HostStorage,
    module: ImModule,
    sink: EffectSink,
}

#[derive(Default)]
struct Trace {
    events: Vec<Value>,
    acks: Vec<Value>,
}

struct PumpOutcome {
    trace: Trace,
    error: Option<String>,
}

/// 建立产品 schema、频道 authority 和成员投影；启动查询通过真实 storage reply 建立本地视图。
async fn fixture(last_post: &str, projection_revision: i64, cursor: u64) -> Fixture {
    let db = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .expect("open isolated sqlite");
    for sql in IM_SCHEMA.iter().chain(IM_SCHEMA_MIGRATIONS) {
        if let Err(error) = db.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "schema/migration failed: {error}"
            );
        }
    }
    db.batch_upsert(UpsertSpec::new(
        "channel",
        vec![vec![
            ("id".into(), SqlValue::Text(CHANNEL.into())),
            ("user_id".into(), SqlValue::Text(RECEIVER.into())),
            ("team_id".into(), SqlValue::Text(COMPANY.into())),
            ("display_name".into(), SqlValue::Text("before".into())),
            ("last_post".into(), SqlValue::Text(last_post.into())),
        ]],
        Some("id"),
    ))
    .await
    .expect("seed channel authority");
    db.batch_upsert(UpsertSpec::new(
        "channel_member",
        vec![
            vec![
                ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                ("user_id".into(), SqlValue::Text(RECEIVER.into())),
                ("team_id".into(), SqlValue::Text(COMPANY.into())),
                ("last_post".into(), SqlValue::Text(last_post.into())),
                ("last_post_at".into(), SqlValue::Integer(123)),
                ("last_root_post_at".into(), SqlValue::Integer(123)),
                ("unread_count".into(), SqlValue::Integer(4)),
                (
                    "projection_revision".into(),
                    SqlValue::Integer(projection_revision),
                ),
                (
                    "effect_id".into(),
                    SqlValue::Text("member-projection-13".into()),
                ),
            ],
            vec![
                ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                ("user_id".into(), SqlValue::Text(OUTSIDER.into())),
                ("team_id".into(), SqlValue::Text(COMPANY.into())),
                (
                    "last_post".into(),
                    SqlValue::Text(r#"{"id":"outsider"}"#.into()),
                ),
                ("last_post_at".into(), SqlValue::Integer(321)),
                ("last_root_post_at".into(), SqlValue::Integer(321)),
                ("projection_revision".into(), SqlValue::Integer(77)),
                (
                    "effect_id".into(),
                    SqlValue::Text("outsider-projection".into()),
                ),
                ("unread_count".into(), SqlValue::Integer(9)),
            ],
        ],
        Some("channel_id,user_id"),
    ))
    .await
    .expect("seed member authority");
    if cursor > 0 {
        db.batch_upsert(UpsertSpec::new(
            "channel_event_cursor",
            vec![vec![
                ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                ("last_event_seq".into(), SqlValue::Integer(cursor as i64)),
            ]],
            Some("channel_id"),
        ))
        .await
        .expect("seed event cursor");
    }

    let mut module = ImModule::new(ImConfig {
        auth_user_id: RECEIVER.into(),
        company_id: COMPANY.into(),
        api_base_url: "http://test".into(),
        ..Default::default()
    });
    module.register_channel(common::channel_id(1), cursor);
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_query_dialog_list",
                br#"{"req_id":"empty-json-fixture"}"#.to_vec(),
            )),
            0,
            &mut sink,
        )
        .expect("startup query must be accepted");
    let mut fixture = Fixture { db, module, sink };
    let startup = pump(&mut fixture, false).await;
    assert!(
        startup.error.is_none(),
        "startup storage lifecycle failed: {:?}",
        startup.error
    );
    fixture.sink.clear();
    fixture
}

/// 仅兑现存储与 clientAck；任何真实网络、Send 或未声明 Effect 都让测试失败。
async fn pump(fixture: &mut Fixture, fail_storage: bool) -> PumpOutcome {
    pump_with_options(fixture, fail_storage, false, false).await
}

async fn pump_with_mismatched_member_viewer(fixture: &mut Fixture) -> PumpOutcome {
    pump_with_options(fixture, false, true, false).await
}

async fn pump_with_sync_backfill(fixture: &mut Fixture) -> PumpOutcome {
    pump_with_options(fixture, false, false, true).await
}

/// Only the public member readback reply is replaced; the preceding real SQLite commit remains.
async fn pump_with_options(
    fixture: &mut Fixture,
    fail_storage: bool,
    mut override_member_viewer: bool,
    allow_sync_backfill: bool,
) -> PumpOutcome {
    let mut trace = Trace::default();
    for _ in 0..32 {
        if fixture.sink.is_empty() {
            return PumpOutcome { trace, error: None };
        }
        let batch = std::mem::take(&mut fixture.sink);
        let mut replies = Vec::new();
        for effect in batch.as_slice() {
            match effect {
                Effect::Persist { corr, ops } => {
                    let outcome = if override_member_viewer {
                        override_member_viewer = false;
                        let row = vec![
                            ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                            ("user_id".into(), SqlValue::Text(OUTSIDER.into())),
                            ("unread_count".into(), SqlValue::Integer(4)),
                            ("last_post".into(), SqlValue::Text(String::new())),
                            ("projection_revision".into(), SqlValue::Integer(13)),
                            ("mention_list".into(), SqlValue::Text("[]".into())),
                            ("urgent_post_list".into(), SqlValue::Text("[]".into())),
                        ];
                        PortOutcome::Ok(helix_core::tick::ReplyBytes(rows_to_reply_bytes(&[row])))
                    } else if fail_storage {
                        PortOutcome::Err(PortError::Storage(500))
                    } else {
                        execute_storage_ops(&fixture.db, ops).await
                    };
                    replies.push((*corr, outcome));
                }
                Effect::PersistAtomic { corr, ops } => {
                    let outcome = if fail_storage {
                        PortOutcome::Err(PortError::Storage(500))
                    } else {
                        execute_storage_ops_atomic(&fixture.db, ops).await
                    };
                    replies.push((*corr, outcome));
                }
                Effect::PersistFire { ops } => {
                    let outcome = execute_storage_ops(&fixture.db, ops).await;
                    assert!(
                        matches!(outcome, PortOutcome::Ok(_)),
                        "fire persist: {outcome:?}"
                    );
                }
                Effect::Emit { event } => {
                    trace.events.push(
                        serde_json::from_slice(event.0.as_ref()).expect("emit JSON envelope"),
                    );
                }
                Effect::Http { corr, req } if req.url.ends_with("/post/clientAck") => {
                    trace.acks.push(
                        serde_json::from_slice(req.body.as_ref().expect("clientAck body"))
                            .expect("clientAck JSON body"),
                    );
                    replies.push((
                        *corr,
                        PortOutcome::Ok(helix_core::tick::ReplyBytes(enveloped(
                            Bytes::from_static(br#"{"status":"SUCCESS","data":{"recorded":true}}"#),
                        ))),
                    ));
                }
                Effect::Http { corr, req }
                    if allow_sync_backfill && req.url.ends_with("/channel/sync/notify") =>
                {
                    // A gap legitimately schedules sync/notify; use a no-change Go envelope
                    // locally so the gate can release without contacting an external service.
                    replies.push((
                        *corr,
                        PortOutcome::Ok(helix_core::tick::ReplyBytes(enveloped(
                            common::sync_reply_no_change(1, 1),
                        ))),
                    ));
                }
                Effect::Http { req, .. } => {
                    panic!("offline test saw unexpected HTTP: {}", req.url)
                }
                Effect::HttpFire { .. } | Effect::Send { .. } | Effect::Request { .. } => {
                    panic!("offline regression must not issue external I/O: {effect:?}")
                }
                // Gate timeout cleanup is an in-memory intent; this test does not advance time.
                Effect::ScheduleTimer { .. } | Effect::CancelTimer { .. } => {}
                Effect::UploadFile { .. } => {
                    panic!("unexpected effect in bounded regression: {effect:?}")
                }
            }
        }
        for (corr, outcome) in replies {
            if let Err(error) =
                fixture
                    .module
                    .handle(&Tick::PortReply { corr, outcome }, 1_001, &mut fixture.sink)
            {
                return PumpOutcome {
                    trace,
                    error: Some(error.to_string()),
                };
            }
        }
    }
    PumpOutcome {
        trace,
        error: Some("bounded lifecycle did not settle".into()),
    }
}

async fn seed_message(fixture: &Fixture, id: &str, message: &str, expedite_map: &str) {
    fixture
        .db
        .batch_upsert(UpsertSpec::new(
            "message",
            vec![vec![
                ("temporary_id".into(), SqlValue::Text(id.into())),
                ("id".into(), SqlValue::Text(id.into())),
                ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                ("user_id".into(), SqlValue::Text("sender".into())),
                ("type".into(), SqlValue::Text("TEXT".into())),
                ("message".into(), SqlValue::Text(message.into())),
                ("simple_message".into(), SqlValue::Text(message.into())),
                ("props".into(), SqlValue::Text("{}".into())),
                ("viewers".into(), SqlValue::Text(r#"["all"]"#.into())),
                ("expedite_map".into(), SqlValue::Text(expedite_map.into())),
                ("create_at".into(), SqlValue::Integer(1)),
                ("update_at".into(), SqlValue::Integer(1)),
            ]],
            Some("temporary_id"),
        ))
        .await
        .expect("seed message");
}

fn post_frame(seq: u64, id: &str, message: &str) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post",
            "data": {
                "channelId": CHANNEL,
                "event_seq": seq,
                "post": {
                    "id": id,
                    "temporaryId": id,
                    "channelId": CHANNEL,
                    "userId": "sender",
                    "type": "TEXT",
                    "message": message,
                    "simpleMessage": message,
                    "props": {},
                    "viewers": ["all"],
                    "createAt": seq,
                    "updateAt": seq
                }
            }
        }))
        .expect("post frame")
        .into(),
    ))
}

fn post_update_frame(seq: u64, id: &str, message: &str, expedite_map: Value) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post_update",
            "data": {
                "event_seq": seq,
                "post": {
                    "id": id,
                    "temporaryId": id,
                    "channelId": CHANNEL,
                    "userId": "sender",
                    "type": "TEXT",
                    "message": message,
                    "simpleMessage": message,
                    "props": {},
                    "viewers": ["all"],
                    "createAt": 1,
                    "updateAt": seq,
                    "expediteMap": expedite_map
                }
            }
        }))
        .expect("post_update frame")
        .into(),
    ))
}

fn update_channel_frame(last_post: Value, display_name: &str) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "update_channel",
            "data": {
                "id": CHANNEL,
                "userId": RECEIVER,
                "displayName": display_name,
                "lastPost": last_post
            }
        }))
        .expect("update_channel frame")
        .into(),
    ))
}

async fn get_row(fixture: &Fixture, table: &'static str, key_col: &'static str, key: &str) -> Row {
    fixture
        .db
        .get(GetSpec {
            table,
            key_col,
            key_val: SqlValue::Text(key.into()),
        })
        .await
        .expect("read real sqlite row")
        .expect("expected real sqlite row")
}

fn column<'a>(row: &'a Row, key: &str) -> &'a SqlValue {
    row.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .unwrap_or_else(|| panic!("missing column {key} in row {row:?}"))
}

fn text<'a>(row: &'a Row, key: &str) -> &'a str {
    match column(row, key) {
        SqlValue::Text(value) => value,
        value => panic!("{key} expected text, got {value:?}"),
    }
}

fn integer(row: &Row, key: &str) -> i64 {
    match column(row, key) {
        SqlValue::Integer(value) => *value,
        value => panic!("{key} expected integer, got {value:?}"),
    }
}

fn named_events<'a>(trace: &'a Trace, name: &str) -> Vec<&'a Value> {
    trace
        .events
        .iter()
        .filter(|event| event["event"] == name)
        .collect()
}

/// 精确空串必须形成 JSON string 终态，并保留 revision/计数/authority；重复 seq 不再终结一次。
#[tokio::test]
async fn post_empty_member_last_post_preserves_versioned_authority_and_is_idempotent() {
    let mut fixture = fixture("", 13, 0).await;
    fixture
        .module
        .handle(
            &post_frame(1, &sid(1), "member empty regression"),
            1_000,
            &mut fixture.sink,
        )
        .expect("post frame accepted");
    let outcome = pump(&mut fixture, false).await;
    assert!(
        outcome.error.is_none(),
        "post lifecycle: {:?}",
        outcome.error
    );
    assert_eq!(outcome.trace.acks.len(), 1);
    assert_eq!(outcome.trace.acks[0]["postId"], sid(1));
    assert_eq!(outcome.trace.acks[0]["ackId"], format!("{}:1", sid(1)));
    assert_eq!(named_events(&outcome.trace, "im:post:received").len(), 1);
    assert_eq!(named_events(&outcome.trace, "im:channel:update").len(), 1);
    assert_eq!(
        named_events(&outcome.trace, "im:post:client-ack-succeeded").len(),
        1
    );
    let channel_update = named_events(&outcome.trace, "im:channel:update")[0];
    assert_eq!(channel_update["data"]["userId"], RECEIVER);
    assert_eq!(channel_update["data"]["lastPost"], "");

    let member = get_row(&fixture, "channel_member", "channel_id", CHANNEL).await;
    assert_eq!(text(&member, "last_post"), "");
    assert_eq!(integer(&member, "projection_revision"), 13);
    assert_eq!(integer(&member, "unread_count"), 4);
    assert_eq!(integer(&member, "last_post_at"), 123);
    let outsider = fixture
        .db
        .get(GetSpec {
            table: "channel_member",
            key_col: "user_id",
            key_val: SqlValue::Text(OUTSIDER.into()),
        })
        .await
        .expect("read member rows")
        .expect("outsider member row");
    assert_eq!(text(&outsider, "last_post"), r#"{"id":"outsider"}"#);
    let cursor = get_row(&fixture, "channel_event_cursor", "channel_id", CHANNEL).await;
    assert_eq!(integer(&cursor, "last_event_seq"), 1);

    fixture.sink.clear();
    fixture
        .module
        .handle(
            &post_frame(1, &sid(1), "member empty regression"),
            1_002,
            &mut fixture.sink,
        )
        .expect("duplicate frame is a handled no-op");
    assert!(
        fixture.sink.is_empty(),
        "duplicate seq must not emit terminal effects"
    );
}

/// A readback row for another viewer must terminate the lifecycle with no ACK or business Emit.
#[tokio::test]
async fn post_member_readback_viewer_mismatch_is_fail_closed() {
    let mut fixture = fixture("", 13, 0).await;
    fixture
        .module
        .handle(
            &post_frame(1, &sid(9), "viewer mismatch"),
            1_000,
            &mut fixture.sink,
        )
        .expect("post frame accepted");
    let result = pump_with_mismatched_member_viewer(&mut fixture).await;
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("viewer mismatch")),
        "wrong viewer readback must be rejected: {:?}",
        result.error
    );
    assert!(result.trace.acks.is_empty());
    assert!(result.trace.events.is_empty());
}

/// 非空合法 JSON 继续按原值投影；非空非法 JSON 在成员读回处显式失败且不发 ACK/事件。
#[tokio::test]
async fn member_last_post_object_is_preserved_and_invalid_json_fails_closed() {
    let mut valid = fixture(r#"{"id":"old","message":"old"}"#, 13, 0).await;
    valid
        .module
        .handle(
            &post_frame(1, &sid(2), "valid object"),
            1_000,
            &mut valid.sink,
        )
        .expect("post frame accepted");
    let result = pump(&mut valid, false).await;
    assert!(
        result.error.is_none(),
        "valid member object: {:?}",
        result.error
    );
    let event = named_events(&result.trace, "im:channel:update");
    assert_eq!(event.len(), 1);
    assert_eq!(event[0]["data"]["lastPost"]["id"], "old");
    assert_eq!(
        integer(
            &get_row(&valid, "channel_member", "channel_id", CHANNEL).await,
            "projection_revision"
        ),
        13
    );

    let mut invalid = fixture("not-json", 13, 0).await;
    invalid
        .module
        .handle(
            &post_frame(1, &sid(3), "invalid member summary"),
            1_000,
            &mut invalid.sink,
        )
        .expect("post admission still succeeds before member readback");
    let result = pump(&mut invalid, false).await;
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("last_post")),
        "invalid member summary must fail explicitly: {:?}",
        result.error
    );
    assert!(result.trace.acks.is_empty());
    assert!(result.trace.events.is_empty());
    let member = get_row(&invalid, "channel_member", "channel_id", CHANNEL).await;
    assert_eq!(text(&member, "last_post"), "not-json");
    assert_eq!(integer(&member, "projection_revision"), 13);
    assert_eq!(integer(&member, "unread_count"), 4);
}

/// 首个原子持久化失败时，消息游标与终态事件均不得伪造成功。
#[tokio::test]
async fn post_persist_error_does_not_emit_or_ack() {
    let mut fixture = fixture("", 13, 0).await;
    fixture
        .module
        .handle(
            &post_frame(1, &sid(4), "persist failure"),
            1_000,
            &mut fixture.sink,
        )
        .expect("post frame accepted");
    let result = pump(&mut fixture, true).await;
    assert!(result.trace.acks.is_empty());
    assert!(result.trace.events.is_empty());
    let message = fixture
        .db
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text(sid(4)),
        })
        .await
        .expect("read failed post");
    assert!(message.is_none());
    let cursor = fixture
        .db
        .get(GetSpec {
            table: "channel_event_cursor",
            key_col: "channel_id",
            key_val: SqlValue::Text(CHANNEL.into()),
        })
        .await
        .expect("read failed cursor");
    assert!(cursor.is_none());
}

/// update_channel 的空串进入普通 PATCH；对象也应真落 channel/member 行并在 PersistOk 后 Emit。
#[tokio::test]
async fn update_channel_empty_and_object_last_post_patch_real_sqlite() {
    let mut fixture = fixture("", 0, 0).await;
    fixture
        .module
        .handle(
            &update_channel_frame(Value::String(String::new()), "empty-patched"),
            1_000,
            &mut fixture.sink,
        )
        .expect("empty lastPost must use ordinary channel patch");
    let empty = pump(&mut fixture, false).await;
    assert!(
        empty.error.is_none(),
        "empty channel patch: {:?}",
        empty.error
    );
    let channel = get_row(&fixture, "channel", "id", CHANNEL).await;
    assert_eq!(text(&channel, "display_name"), "empty-patched");
    assert_eq!(text(&channel, "last_post"), "");
    let member = get_row(&fixture, "channel_member", "channel_id", CHANNEL).await;
    assert_eq!(text(&member, "last_post"), "");
    let update = named_events(&empty.trace, "im:channel:update");
    assert_eq!(update.len(), 1);
    assert_eq!(update[0]["data"]["lastPost"], "");

    fixture.sink.clear();
    fixture
        .module
        .handle(
            &update_channel_frame(json!({"id":"normal","message":"normal"}), "object-patched"),
            1_001,
            &mut fixture.sink,
        )
        .expect("object lastPost must remain a normal patch");
    let object = pump(&mut fixture, false).await;
    assert!(
        object.error.is_none(),
        "object channel patch: {:?}",
        object.error
    );
    let channel = get_row(&fixture, "channel", "id", CHANNEL).await;
    assert_eq!(text(&channel, "display_name"), "object-patched");
    assert_eq!(
        serde_json::from_str::<Value>(text(&channel, "last_post")).expect("stored object"),
        json!({"id":"normal","message":"normal","simpleMessage":"normal"})
    );
    assert_eq!(named_events(&object.trace, "im:channel:update").len(), 1);
    assert_eq!(object.trace.events[0]["data"]["lastPost"]["id"], "normal");

    for invalid in [Value::String("{bad".into()), json!([])] {
        fixture.sink.clear();
        let result = fixture.module.handle(
            &update_channel_frame(invalid, "must-not-write"),
            1_002,
            &mut fixture.sink,
        );
        assert!(result.is_err(), "invalid lastPost must be rejected");
        assert!(fixture.sink.is_empty(), "invalid lastPost must not persist");
    }
}

/// expediteMap 精确空串走普通完整编辑；`{}` 仍走 urgent 且落库/发更新事件。
#[tokio::test]
async fn post_update_empty_expedite_is_full_edit_and_object_is_urgent() {
    let mut ordinary = fixture("", 0, 1).await;
    seed_message(&ordinary, &sid(5), "before", "").await;
    ordinary
        .module
        .handle(
            &post_update_frame(2, &sid(5), "after-empty", Value::String(String::new())),
            1_000,
            &mut ordinary.sink,
        )
        .expect("empty expediteMap edit accepted");
    let ordinary_result = pump(&mut ordinary, false).await;
    assert!(
        ordinary_result.error.is_none(),
        "ordinary edit lifecycle: {:?}",
        ordinary_result.error
    );
    let message = get_row(&ordinary, "message", "temporary_id", &sid(5)).await;
    assert_eq!(text(&message, "message"), "after-empty");
    assert_eq!(text(&message, "expedite_map"), "");
    assert_eq!(
        integer(
            &get_row(&ordinary, "channel_event_cursor", "channel_id", CHANNEL).await,
            "last_event_seq"
        ),
        2
    );
    assert_eq!(
        named_events(&ordinary_result.trace, "im:post:updated").len(),
        1
    );

    // Go's post DTO emits `expediteMap: null` for an absent map; it is the same
    // ordinary edit as the exact empty string and must not enter urgent storage.
    let mut null_map = fixture("", 0, 1).await;
    seed_message(&null_map, &sid(8), "before", "").await;
    null_map
        .module
        .handle(
            &post_update_frame(2, &sid(8), "after-null", Value::Null),
            1_000,
            &mut null_map.sink,
        )
        .expect("null expediteMap is a valid ordinary edit");
    let null_result = pump(&mut null_map, false).await;
    assert!(
        null_result.error.is_none(),
        "null ordinary edit lifecycle: {:?}",
        null_result.error
    );
    let message = get_row(&null_map, "message", "temporary_id", &sid(8)).await;
    assert_eq!(text(&message, "message"), "after-null");
    assert_eq!(text(&message, "expedite_map"), "");
    assert_eq!(
        integer(
            &get_row(&null_map, "channel_event_cursor", "channel_id", CHANNEL).await,
            "last_event_seq"
        ),
        2
    );
    assert_eq!(named_events(&null_result.trace, "im:post:updated").len(), 1);

    let mut urgent = fixture("", 0, 1).await;
    seed_message(&urgent, &sid(6), "before", "").await;
    urgent
        .module
        .handle(
            &post_update_frame(2, &sid(6), "after-urgent", json!({})),
            1_000,
            &mut urgent.sink,
        )
        .expect("empty object expediteMap remains urgent");
    let urgent_result = pump(&mut urgent, false).await;
    assert!(
        urgent_result.error.is_none(),
        "urgent lifecycle: {:?}",
        urgent_result.error
    );
    let message = get_row(&urgent, "message", "temporary_id", &sid(6)).await;
    assert_eq!(text(&message, "message"), "before");
    assert_eq!(text(&message, "expedite_map"), "{}");
    assert_eq!(
        integer(
            &get_row(&urgent, "channel_event_cursor", "channel_id", CHANNEL).await,
            "last_event_seq"
        ),
        2
    );
    assert_eq!(
        named_events(&urgent_result.trace, "im:post:updated").len(),
        1
    );

    for invalid in [
        Value::String("not-json".into()),
        json!("nonempty"),
        json!([]),
    ] {
        let mut rejected = fixture("", 0, 1).await;
        seed_message(&rejected, &sid(7), "before", "").await;
        rejected.sink.clear();
        let result = rejected.module.handle(
            &post_update_frame(2, &sid(7), "must-not-write", invalid),
            1_000,
            &mut rejected.sink,
        );
        assert!(result.is_err(), "invalid expediteMap must fail closed");
        assert!(
            rejected.sink.is_empty(),
            "invalid expediteMap must not persist"
        );
        let cursor = get_row(&rejected, "channel_event_cursor", "channel_id", CHANNEL).await;
        assert_eq!(integer(&cursor, "last_event_seq"), 1);
    }
}

/// A buffered empty edit must flush in sequence order and preserve an existing urgent map.
#[tokio::test]
async fn buffered_empty_edits_preserve_existing_urgent_data() {
    let mut fixture = fixture("", 0, 1).await;
    let id = sid(10);
    let existing_urgent = r#"{"member":{"status":1}}"#;
    seed_message(&fixture, &id, "before", existing_urgent).await;

    fixture
        .module
        .handle(
            &post_update_frame(3, &id, "buffered-3", Value::String(String::new())),
            1_000,
            &mut fixture.sink,
        )
        .expect("out-of-order edit is buffered");
    assert!(
        fixture.sink.as_slice().iter().all(|effect| {
            matches!(
                effect,
                Effect::ScheduleTimer { .. } | Effect::CancelTimer { .. }
            ) || matches!(
                effect,
                Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify")
            )
        }),
        "seq3 may schedule a gate timer but must not write or emit while buffered"
    );

    fixture
        .module
        .handle(
            &post_update_frame(2, &id, "buffered-2", Value::String(String::new())),
            1_001,
            &mut fixture.sink,
        )
        .expect("seq2 admits and flushes seq3");
    let result = pump_with_sync_backfill(&mut fixture).await;
    assert!(
        result.error.is_none(),
        "buffered edit lifecycle: {:?}",
        result.error
    );
    assert_eq!(named_events(&result.trace, "im:post:updated").len(), 2);
    let message = get_row(&fixture, "message", "temporary_id", &id).await;
    assert_eq!(text(&message, "message"), "buffered-3");
    assert_eq!(text(&message, "expedite_map"), existing_urgent);
    let cursor = get_row(&fixture, "channel_event_cursor", "channel_id", CHANNEL).await;
    assert_eq!(integer(&cursor, "last_event_seq"), 3);
}

/// @ 受众在真实落库和 received 中一致；重复回声不重发，卡片 props 不受影响。
#[tokio::test]
async fn mentions_survive_persist_and_received_for_footer_notifications() {
    let mut fixture = fixture("", 13, 0).await;
    let mut received = Vec::new();
    for (index, mentions, props) in [
        (1, json!([RECEIVER]), json!({})),
        (2, json!([RECEIVER]), json!({})),
        (
            3,
            json!([]),
            json!({"meetingCardBlock":{"detail":{"title":"会议"}}}),
        ),
        (
            4,
            json!([]),
            json!({"taskCardBlock":{"detail":{"title":"任务"}}}),
        ),
        (5, json!([OUTSIDER]), json!({})),
    ] {
        let Tick::Inbound(frame) = post_frame(index, &sid(index), "notification") else {
            panic!("expected inbound");
        };
        let mut wire: Value = serde_json::from_slice(&frame.0).unwrap();
        wire["data"]["post"]["mentions"] = mentions.clone();
        wire["data"]["post"]["props"] = props.clone();
        let tick = Tick::Inbound(InboundBytes(serde_json::to_vec(&wire).unwrap().into()));
        fixture
            .module
            .handle(&tick, 1_000, &mut fixture.sink)
            .unwrap();
        assert!(
            !fixture
                .sink
                .as_slice()
                .iter()
                .any(|effect| matches!(effect, Effect::Emit { .. })),
            "no pre-persist event"
        );
        let result = pump(&mut fixture, false).await;
        assert!(result.error.is_none(), "{:?}", result.error);
        let row = get_row(&fixture, "message", "id", &sid(index)).await;
        assert_eq!(
            serde_json::from_str::<Value>(text(&row, "mentions")).unwrap(),
            mentions
        );
        let events = named_events(&result.trace, "im:post:received");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["data"]["mentions"], mentions,
            "stored mentions must reach received"
        );
        if props != json!({}) {
            assert_eq!(events[0]["data"]["props"], props);
        }
        received.push(events[0].clone());
        fixture
            .module
            .handle(&tick, 1_001, &mut fixture.sink)
            .unwrap();
        let duplicate = pump(&mut fixture, false).await;
        assert!(duplicate.error.is_none());
        assert!(named_events(&duplicate.trace, "im:post:received").is_empty());
    }
    // 显式生成客户端合同 fixture；内容只取公开 Tick 实际释放的事件。
    if let Ok(path) = std::env::var("HELIX_MENTION_FIXTURE_PATH") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&received).unwrap() + "\n",
        )
        .unwrap();
    }
}
