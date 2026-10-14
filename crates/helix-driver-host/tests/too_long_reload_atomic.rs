//! Public IM recovery → the same SQLite executor used by App FFI.
#[path = "../../helix-im/tests/common.rs"]
mod common;

use common::{
    channel_id, enveloped, inbound_event, port_reply_with_body, sync_reply_too_long, Harness,
};
use helix_core::effect::{GetSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::PortOutcome;
use helix_core::{Effect, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use serde_json::{json, Value};

/// Each run owns its database and seeds an existing conversation.
async fn storage() -> HostStorage {
    let store = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .unwrap();
    for sql in helix_im::IM_SCHEMA
        .iter()
        .chain(helix_im::IM_SCHEMA_MIGRATIONS)
    {
        if let Err(e) = store.execute_raw(sql).await {
            assert!(e.to_string().contains("duplicate column name"), "{e}");
        }
    }
    store
        .batch_upsert(UpsertSpec::new(
            "channel",
            vec![vec![
                ("id".into(), SqlValue::Text(common::cid(1))),
                ("unread_count".into(), SqlValue::Integer(0)),
                ("last_root_post_at".into(), SqlValue::Integer(0)),
            ]],
            Some("id"),
        ))
        .await
        .unwrap();
    store
}

/// Start routine gap recovery through public inbound and HTTP reply seams.
fn recovery() -> Harness {
    let mut h = Harness::new_with_identity(&[1], "viewer-g13c", "company-g13c", "", "", "");
    h.step(inbound_event(1, 5, 1), 1000);
    let sync = http_corr(&h);
    h.step(
        port_reply_with_body(sync, enveloped(sync_reply_too_long(1, 100))),
        2000,
    );
    let reload = http_corr(&h);
    let posts = [99, 100].map(|seq| {
        json!({
            "id":format!("reload-post-{seq}"), "channelId":common::cid(1),
            "eventSeq":seq, "userId":"author", "type":"TEXT",
            "message":"authoritative", "createAt":1700000000_i64 + seq,
            "viewers":["viewer-g13c"]
        })
    });
    let body = json!({"status":"SUCCESS", "data":{"posts": posts}});
    h.step(
        port_reply_with_body(reload, enveloped(serde_json::to_vec(&body).unwrap().into())),
        2100,
    );
    h
}

/// Extract the current public HTTP correlation, never private state.
fn http_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("recovery must issue HTTP")
}

/// Observe persisted integer columns through the Storage port.
async fn integer(
    store: &HostStorage,
    table: &'static str,
    key: &'static str,
    col: &str,
) -> Option<i64> {
    store
        .get(GetSpec {
            table,
            key_col: key,
            key_val: SqlValue::Text(common::cid(1)),
        })
        .await
        .unwrap()
        .and_then(|row| {
            row.into_iter().find_map(|(k, v)| match v {
                SqlValue::Integer(n) if k == col => Some(n),
                _ => None,
            })
        })
}

/// A nonempty routine TooLong window commits with native write-only transactions;
/// absolute channel projection is released only after committed-row readback.
#[tokio::test]
async fn routine_reload_commits_cursor_and_reads_channel_once() {
    let store = storage().await;
    let mut h = recovery();
    let (corr, ops) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .unwrap();
    let outcome = execute_storage_ops_atomic(&store, &ops).await;
    assert!(
        matches!(outcome, PortOutcome::Ok(_)),
        "App SQLite rejected recovery: {outcome:?}"
    );
    assert_eq!(
        integer(
            &store,
            "channel_event_cursor",
            "channel_id",
            "last_event_seq"
        )
        .await,
        Some(100)
    );
    assert_eq!(
        integer(&store, "channel", "id", "unread_count").await,
        Some(2)
    );
    h.step(Tick::PortReply { corr, outcome }, 2200);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(100));
    let reads: Vec<_> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        reads.len(),
        1,
        "one readback for the latest post in the window"
    );
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    let sync_body = h.last_http_body().unwrap();
    assert_eq!(
        sync_body["cursors"],
        json!([{"channelId": common::cid(1), "fromSeq": 100}])
    );
    let (read_corr, read_ops) = &reads[0];
    assert!(matches!(
        read_ops.as_slice(),
        [helix_core::effect::StorageOp::Get(_)]
    ));
    let read_outcome = execute_storage_ops(&store, read_ops).await;
    h.step(
        Tick::PortReply {
            corr: *read_corr,
            outcome: read_outcome,
        },
        2300,
    );
    let events: Vec<Value> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => Some(serde_json::from_slice(event.0.as_ref()).unwrap()),
            _ => None,
        })
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:channel:update");
    assert_eq!(events[0]["data"]["dialogPatch"]["unreadCount"], 2);
    assert_eq!(events[0]["data"]["msgId"], "reload-post-100");
    assert_eq!(
        integer(&store, "channel", "id", "unread_count").await,
        Some(2)
    );
    h.step(common::port_reply_ok(corr.raw()), 2400);
    assert!(
        h.sink.is_empty(),
        "duplicate commit must not repeat projection or catch-up"
    );
}

/// A real SQLite trigger failure rolls back every message and leaves both cursors old.
#[tokio::test]
async fn routine_reload_write_failure_keeps_window_and_cursor() {
    let store = storage().await;
    store.execute_raw("CREATE TRIGGER reject_reload BEFORE INSERT ON message BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").await.unwrap();
    let mut h = recovery();
    let (corr, ops) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .unwrap();
    let outcome = execute_storage_ops_atomic(&store, &ops).await;
    assert!(matches!(outcome, PortOutcome::Err(_)));
    h.step(Tick::PortReply { corr, outcome }, 2200);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert_eq!(
        integer(
            &store,
            "channel_event_cursor",
            "channel_id",
            "last_event_seq"
        )
        .await,
        None
    );
    assert_eq!(
        integer(&store, "channel", "id", "unread_count").await,
        Some(0)
    );
    assert!(!h.sink.as_slice().iter().any(|e| matches!(
        e,
        Effect::Persist { .. } | Effect::Http { .. } | Effect::Emit { .. }
    )));
    assert!(store
        .get(GetSpec {
            table: "message",
            key_col: "id",
            key_val: SqlValue::Text("reload-post-99".into())
        })
        .await
        .unwrap()
        .is_none());
}

/// A → B → A must revoke each continuation after the actual SQLite commit.
#[tokio::test]
async fn identity_reset_drops_committed_reload_and_topic_continuations() {
    for phase in 0..3 {
        let store = storage().await;
        for (table, rows) in [
            (
                "channel",
                vec![
                    vec![
                        ("id".into(), SqlValue::Text(common::cid(1))),
                        ("root_post_id".into(), SqlValue::Text("topic-root".into())),
                        ("display_name".into(), SqlValue::Text("Topic".into())),
                    ],
                    vec![
                        ("id".into(), SqlValue::Text(common::cid(2))),
                        ("root_post_id".into(), SqlValue::Text(String::new())),
                        ("display_name".into(), SqlValue::Text("Parent".into())),
                    ],
                ],
            ),
            (
                "message",
                vec![vec![
                    ("temporary_id".into(), SqlValue::Text("topic-root".into())),
                    ("id".into(), SqlValue::Text("topic-root".into())),
                    ("channel_id".into(), SqlValue::Text(common::cid(2))),
                    ("type".into(), SqlValue::Text("TEXT".into())),
                    ("message".into(), SqlValue::Text("Root".into())),
                ]],
            ),
        ] {
            let key = if table == "message" {
                "temporary_id"
            } else {
                "id"
            };
            store
                .batch_upsert(UpsertSpec::new(table, rows, Some(key)))
                .await
                .unwrap();
        }
        let mut h = recovery();
        let (commit, ops) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone())),
                _ => None,
            })
            .unwrap();
        h.step(
            Tick::PortReply {
                corr: commit,
                outcome: execute_storage_ops_atomic(&store, &ops).await,
            },
            2200,
        );
        let mut continuation = pending_storage(&h);
        // Advance independently to channel read, topic root read, or topic write.
        for step in 0..phase {
            h.step(
                Tick::PortReply {
                    corr: continuation.0,
                    outcome: execute_storage_ops(&store, &continuation.1).await,
                },
                2300 + step,
            );
            continuation = pending_storage(&h);
        }
        let late = execute_storage_ops(&store, &continuation.1).await;
        assert!(
            matches!(late, PortOutcome::Ok(_)),
            "phase {phase}: {late:?}"
        );
        for (viewer, company) in [
            ("viewer-other", "company-other"),
            ("viewer-g13c", "company-g13c"),
        ] {
            h.step(
                common::command_json(
                    "__helix_runtime_identity",
                    json!({
                        "auth_user_id": viewer, "company_id": company,
                    }),
                ),
                2400,
            );
        }
        h.step(
            Tick::PortReply {
                corr: continuation.0,
                outcome: late,
            },
            2500,
        );
        assert!(
            h.sink.is_empty(),
            "phase {phase} leaked old identity effects: {:?}",
            h.sink.as_slice()
        );
    }
}

/// Only observe the public continuation emitted by the reducer.
fn pending_storage(h: &Harness) -> (helix_core::Correlation, Vec<helix_core::effect::StorageOp>) {
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("reload must schedule its next storage continuation")
}
