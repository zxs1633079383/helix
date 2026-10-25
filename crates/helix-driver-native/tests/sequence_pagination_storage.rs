//! Public Command → real Native/Host SQLite → Emit. HTTP authority is synthetic, not real Go evidence.
use bytes::Bytes;
use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, ScanSpec, SqlValue, StorageOp, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic};
use helix_driver_native::NativeStorage;
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

const CHANNEL: &str = "chfixx00000000000000000001";
const VIEWER: &str = "usrfix00000000000000000001";

/// Isolated file database survives module restart; only this run's files are removed.
struct Fixture {
    path: String,
    storage: NativeStorage,
    module: ImModule,
    requests: Vec<Value>,
    head: i64,
    revision: i64,
    visible: BTreeSet<i64>,
    revoked: BTreeSet<i64>,
    fail_write: bool,
    late_update: Option<i64>,
    late_revoke: Option<i64>,
    late_delete: Option<i64>,
    late_snapshot: Option<i64>,
    omit_slot: bool,
    reverse_time: bool,
    notice_first: bool,
}
impl Fixture {
    async fn new(head: i64, visible: impl IntoIterator<Item = i64>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "helix-sequence-{}-{}.db",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .to_string_lossy()
            .into_owned();
        let storage = NativeStorage::open(&format!("sqlite:{path}"))
            .await
            .unwrap();
        for sql in helix_im::IM_SCHEMA {
            storage.execute_raw(sql).await.unwrap();
        }
        for sql in helix_im::IM_SCHEMA_MIGRATIONS {
            if let Err(e) = storage.execute_raw(sql).await {
                assert!(e.to_string().contains("duplicate column"), "{e}");
            }
        }
        for sql in helix_im::IM_TIMELINE_SEQ_SCHEMA {
            storage.execute_raw(sql).await.unwrap();
        }
        Self {
            path,
            storage,
            module: module(),
            requests: Vec::new(),
            head,
            revision: 1,
            visible: visible.into_iter().collect(),
            revoked: BTreeSet::new(),
            fail_write: false,
            late_update: None,
            late_revoke: None,
            late_delete: None,
            late_snapshot: None,
            omit_slot: false,
            reverse_time: false,
            notice_first: false,
        }
    }

    /// Drive only the public module entry; all database reads/writes use production executors.
    async fn query(&mut self, name: &str, mut args: Value) -> Value {
        args["channel_id"] = json!(CHANNEL);
        args["req_id"] = json!("sequence-test");
        args["sequencePagination"] = json!(true);
        let command =
            helix_im::client_api::build_command(name, args.to_string().as_bytes()).unwrap();
        let mut next = Some(Tick::Command(command));
        let mut sink = EffectSink::new();
        let mut timer = None;
        for turn in 0..500 {
            sink.clear();
            self.module
                .handle(&next.take().unwrap(), turn, &mut sink)
                .unwrap();
            for effect in sink.as_slice() {
                match effect {
                    Effect::Http { corr, req } => {
                        assert!(req.url.ends_with("/posts/sequenceRange"));
                        let body: Value =
                            serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
                        let answer = self.authority(&body);
                        self.requests.push(body);
                        next = Some(Tick::PortReply {
                            corr: *corr,
                            outcome: PortOutcome::Ok(ReplyBytes(envelope(answer))),
                        });
                    }
                    Effect::Persist { corr, ops } => {
                        next = Some(Tick::PortReply {
                            corr: *corr,
                            outcome: execute_storage_ops(&self.storage, ops).await,
                        })
                    }
                    Effect::PersistAtomic { corr, ops } => {
                        assert!(
                            !ops.iter()
                                .any(|op| format!("{op:?}").contains("channel_event")),
                            "history must not write an event cursor/ledger"
                        );
                        if let Some(seq) = self.late_update.take() {
                            self.storage
                                .batch_update(BatchUpdateSpec {
                                    table: "message",
                                    key_col: "temporary_id",
                                    key_vals: vec![SqlValue::Text(format!("t{seq}"))],
                                    patch: vec![
                                        ("event_seq".into(), SqlValue::Integer(999)),
                                        (
                                            "message".into(),
                                            SqlValue::Text("newer live body".into()),
                                        ),
                                    ],
                                })
                                .await
                                .unwrap();
                        }
                        if let Some(seq) = self.late_delete.take() {
                            self.storage
                                .batch_delete(BatchDeleteSpec {
                                    table: "message",
                                    scope_col: "channel_id",
                                    scope_val: SqlValue::Text(CHANNEL.into()),
                                    key_col: "temporary_id",
                                    key_vals: vec![SqlValue::Text(format!("t{seq}"))],
                                })
                                .await
                                .unwrap();
                        }
                        if let Some(seq) = self.late_revoke.take() {
                            self.storage
                                .batch_update(BatchUpdateSpec {
                                    table: "message",
                                    key_col: "temporary_id",
                                    key_vals: vec![SqlValue::Text(format!("t{seq}"))],
                                    patch: vec![
                                        ("revoke".into(), SqlValue::Integer(1)),
                                        ("event_seq".into(), SqlValue::Integer(999)),
                                    ],
                                })
                                .await
                                .unwrap();
                        }
                        if let Some(seq) = self.late_snapshot.take() {
                            self.storage
                                .batch_upsert(UpsertSpec::new(
                                    "message_sequence_slot",
                                    vec![vec![
                                        (
                                            "slot_key".into(),
                                            SqlValue::Text(format!("{CHANNEL}:{seq}")),
                                        ),
                                        ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                                        ("seq".into(), SqlValue::Integer(seq)),
                                        ("is_visible".into(), SqlValue::Integer(1)),
                                        ("temporary_id".into(), SqlValue::Text(format!("t{seq}"))),
                                        ("membership_epoch".into(), SqlValue::Integer(1)),
                                        ("revision".into(), SqlValue::Integer(2)),
                                        ("body_event_seq".into(), SqlValue::Integer(seq)),
                                    ]],
                                    Some("slot_key"),
                                ))
                                .await
                                .unwrap();
                            self.storage
                                .batch_update(BatchUpdateSpec {
                                    table: "message",
                                    key_col: "temporary_id",
                                    key_vals: vec![SqlValue::Text(format!("t{seq}"))],
                                    patch: vec![(
                                        "message".into(),
                                        SqlValue::Text("newer private snapshot".into()),
                                    )],
                                })
                                .await
                                .unwrap();
                        }
                        let outcome = if self.fail_write {
                            PortOutcome::Err(helix_core::tick::PortError::Storage(1))
                        } else {
                            execute_storage_ops_atomic(&self.storage, ops).await
                        };
                        next = Some(Tick::PortReply {
                            corr: *corr,
                            outcome,
                        });
                    }
                    Effect::ScheduleTimer { id, .. } => timer = Some(*id),
                    Effect::CancelTimer { id } => {
                        assert_eq!(timer, Some(*id));
                        timer = None;
                    }
                    Effect::Emit { event } => {
                        let event: Value = serde_json::from_slice(&event.0).unwrap();
                        assert_eq!(event["event"], "im:read:result");
                        assert!(timer.is_none(), "terminal must cancel deadline");
                        return event["data"].clone();
                    }
                    unexpected => panic!("unexpected effect {unexpected:?}"),
                }
            }
            assert!(next.is_some(), "query stalled at turn {turn}");
        }
        panic!("unbounded sequence query");
    }

    /// The authority covers every sequence; invisible entries contain no identity/body.
    fn authority(&self, request: &Value) -> Value {
        let head = request["snapshotHeadSeq"]
            .as_str()
            .map(|v| v.parse::<i64>().unwrap())
            .unwrap_or(self.head);
        let mut data = json!({"supported":true,"ready":true,"headSeq":self.head.to_string(),"snapshotHeadSeq":head.to_string(),"firstSeq":if head==0{"0"}else{"1"},"membershipEpoch":"1","revision":self.revision.to_string(),"slots":[]});
        if let Some(anchor) = request["anchorPostId"].as_str() {
            data["anchorSeq"] = json!(anchor
                .strip_prefix('p')
                .unwrap()
                .parse::<i64>()
                .unwrap()
                .to_string());
        }
        if let (Some(lo), Some(hi)) = (request["fromSeq"].as_str(), request["toSeq"].as_str()) {
            data["fromSeq"] = json!(lo);
            data["toSeq"] = json!(hi);
            data["slots"]=Value::Array((lo.parse::<i64>().unwrap()..=hi.parse::<i64>().unwrap()).map(|seq| {
                if self.visible.contains(&seq) {
                    let mut post = json!({"id":format!("p{seq}"),"temporaryId":format!("t{seq}"),"channelId":CHANNEL,"userId":"other","type":if self.notice_first && seq==1 {"NOTICE"}else{"TEXT"},"message":format!("body {seq}"),"createAt":if self.reverse_time { head-seq+1 } else { seq },"updateAt":seq,"viewers":["all"],"props":{},"revoke":self.revoked.contains(&seq)});
                    if self.revoked.contains(&seq) {
                        // Go sequenceVisiblePost clears these fields; ent's omitempty removes them.
                        for field in ["message", "simpleMessage", "props", "repliedMessage", "replyMessages", "topic", "quickReply", "expediteMap"] {
                            post.as_object_mut().unwrap().remove(field);
                        }
                    }
                    json!({"seq":seq.to_string(),"isVisible":true,"postEventSeq":seq.to_string(),"post":post})
                }
                else { json!({"seq":seq.to_string(),"isVisible":false}) }
            }).collect());
            if self.omit_slot {
                data["slots"].as_array_mut().unwrap().pop();
            }
        }
        json!({"status":"SUCCESS","data":data})
    }
    async fn clean(self) {
        let path = self.path.clone();
        drop(self);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
    }

    /// Seed only trusted account ownership, then drive a real inbound edit through commit/readback.
    async fn edit_live(&mut self) -> Vec<Value> {
        self.module
            .register_channel(helix_im::state::ChannelId::from_str(CHANNEL).unwrap(), 0);
        let ops = vec![StorageOp::BatchUpsert(UpsertSpec::new(
            "channel",
            vec![vec![
                ("id".into(), SqlValue::Text(CHANNEL.into())),
                ("team_id".into(), SqlValue::Text("company".into())),
                ("user_id".into(), SqlValue::Text(VIEWER.into())),
            ]],
            Some("id"),
        ))];
        let mut sink = EffectSink::new();
        let corr = self
            .module
            .queue_message_v3_commit(ops.clone(), vec![], &mut sink);
        let outcome = execute_storage_ops_atomic(&self.storage, &ops).await;
        sink.clear();
        self.module
            .handle(&Tick::PortReply { corr, outcome }, 100, &mut sink)
            .unwrap();
        let mut next=Some(Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(json!({"action":"post_update","data":{"event_seq":1,"id":"p45","temporaryId":"t45","channelId":CHANNEL,"userId":"other","type":"TEXT","message":"edited live","createAt":45,"updateAt":61,"viewers":["all"],"props":{}}}).to_string()))));
        let mut events = Vec::new();
        for turn in 101..120 {
            let Some(tick) = next.take() else {
                return events;
            };
            sink.clear();
            self.module.handle(&tick, turn, &mut sink).unwrap();
            for effect in sink.as_slice() {
                match effect {
                    Effect::PersistAtomic { corr, ops } => {
                        let outcome = execute_storage_ops_atomic(&self.storage, ops).await;
                        assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
                        next = Some(Tick::PortReply {
                            corr: *corr,
                            outcome,
                        });
                    }
                    Effect::Persist { corr, ops } => {
                        next = Some(Tick::PortReply {
                            corr: *corr,
                            outcome: execute_storage_ops(&self.storage, ops).await,
                        })
                    }
                    Effect::Emit { event } => {
                        events.push(serde_json::from_slice(&event.0).unwrap())
                    }
                    Effect::ScheduleTimer { .. } | Effect::CancelTimer { .. } => {}
                    unexpected => panic!("unexpected live effect {unexpected:?}"),
                }
            }
        }
        panic!("live refresh stalled");
    }
}

/// Stable identity keeps the test focused on query semantics, not auth setup.
fn module() -> ImModule {
    ImModule::new(ImConfig {
        auth_user_id: VIEWER.into(),
        company_id: "company".into(),
        api_base_url: "http://synthetic/api/cses".into(),
        ..Default::default()
    })
}

/// Real driver HTTP envelope, no handcrafted emitted projection.
fn envelope(body: Value) -> Bytes {
    Bytes::from(json!({"status":200,"headers":[],"body":helix_driver_host::base64_encode(body.to_string().as_bytes())}).to_string())
}

/// Extract immutable post IDs from the production viewer projection.
fn ids(reply: &Value) -> Vec<String> {
    assert!(reply.get("error").is_none(), "{reply}");
    reply["body"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Restart keeps the slot proof; only metadata is requested for a complete warm range.
#[tokio::test]
async fn sparse_history_fills_visible_page_and_reuses_persisted_slots_after_restart() {
    let mut f = Fixture::new(500, (1..=500).filter(|n| n % 10 == 0)).await;
    let first = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(
        ids(&first),
        (31..=50)
            .map(|n| format!("p{}", n * 10))
            .collect::<Vec<_>>()
    );
    assert!(
        f.requests
            .iter()
            .filter(|r| r.get("fromSeq").is_some())
            .count()
            >= 2
    );
    assert_eq!(first["body"]["olderCursor"]["nextSeq"], "309");
    f.module = module();
    f.requests.clear();
    let repeated = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(ids(&repeated), ids(&first));
    assert_eq!(f.requests.len(), 1);
    let slots = f
        .storage
        .scan(ScanSpec {
            table: "message_sequence_slot",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .unwrap();
    assert!(slots
        .iter()
        .filter(|row| row
            .iter()
            .any(|(k, v)| k == "is_visible" && matches!(v, SqlValue::Integer(0))))
        .all(|row| row
            .iter()
            .any(|(k, v)| k == "temporary_id" && matches!(v, SqlValue::Null))));
    f.clean().await;
}

/// Missing body is repaired individually even when its sequence placeholder still exists.
#[tokio::test]
async fn absent_body_is_a_gap_and_only_that_sequence_is_fetched() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.storage
        .batch_delete(BatchDeleteSpec {
            table: "message",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "temporary_id",
            key_vals: vec![SqlValue::Text("t18".into())],
        })
        .await
        .unwrap();
    f.requests.clear();
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(ids(&result).len(), 20);
    let ranges = f
        .requests
        .iter()
        .filter(|r| r.get("fromSeq").is_some())
        .collect::<Vec<_>>();
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0]["fromSeq"], "18");
    assert_eq!(ranges[0]["toSeq"], "18");
    f.clean().await;
}

/// Invisible runs advance by scanned seq and an empty partial page retains a continuation.
#[tokio::test]
async fn invisible_budget_continues_without_anchor_and_never_claims_exhaustion() {
    let mut f = Fixture::new(5000, 1..=20).await;
    let first = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert!(ids(&first).is_empty());
    assert_eq!(first["body"]["hasOlder"], true);
    let cursor = first["body"]["olderCursor"].clone();
    assert!(cursor["nextSeq"].as_str().unwrap().parse::<i64>().unwrap() < 5000);
    let second = f
        .query(
            "im_load_older_context",
            json!({"pageSize":20,"cursor":cursor}),
        )
        .await;
    assert!(ids(&second).is_empty());
    assert!(second["body"]["olderCursor"]["nextSeq"] != first["body"]["olderCursor"]["nextSeq"]);
    f.clean().await;
}

/// Locator forward starts at its target; default locator still supplies context on both sides.
#[tokio::test]
async fn forward_and_center_locators_share_durable_cache_and_order() {
    let mut f = Fixture::new(60, 1..=60).await;
    let forward = f
        .query(
            "im_locate_context",
            json!({"targetPostId":"p30","window":"forward","pageSize":20}),
        )
        .await;
    assert_eq!(
        ids(&forward),
        (30..50).map(|n| format!("p{n}")).collect::<Vec<_>>()
    );
    assert_eq!(forward["body"]["targetIndex"], 0);
    let centered = f
        .query(
            "im_locate_context",
            json!({"targetPostId":"p30","pageSize":20}),
        )
        .await;
    assert_eq!(
        ids(&centered),
        (20..40).map(|n| format!("p{n}")).collect::<Vec<_>>()
    );
    assert_eq!(centered["body"]["targetIndex"], 10);
    f.clean().await;
}

/// Failed atomic persistence never publishes rows or leaves partial completeness evidence.
#[tokio::test]
async fn failed_persist_never_publishes_history_or_advances_sync_cursor() {
    let mut f = Fixture::new(20, 1..=20).await;
    f.fail_write = true;
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert!(result["error"].as_str().unwrap().contains("PORT_FAILED"));
    for table in [
        "message_sequence_slot",
        "message",
        "channel_event_cursor",
        "channel_event_ledger",
    ] {
        assert!(f
            .storage
            .scan(ScanSpec {
                table,
                limit: None,
                filter: None,
                order_by: &[]
            })
            .await
            .unwrap()
            .is_empty());
    }
    f.clean().await;
}

/// New arrivals change head, not the historical range revision, so old slots remain reusable.
#[tokio::test]
async fn newer_head_does_not_discard_complete_older_range() {
    let mut f = Fixture::new(30, 1..=40).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.head = 40;
    f.requests.clear();
    let result = f
        .query(
            "im_load_older_context",
            json!({"anchor_post_id":"p31","pageSize":20}),
        )
        .await;
    assert_eq!(
        ids(&result),
        (11..=30).map(|n| format!("p{n}")).collect::<Vec<_>>()
    );
    assert_eq!(f.requests.len(), 1);
    f.clean().await;
}

/// Changed visibility invalidates old evidence and removes the formerly visible cached body.
#[tokio::test]
async fn visibility_revision_refreshes_slots_and_purges_body_atomically() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.visible.remove(&18);
    f.revision = 2;
    f.requests.clear();
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(ids(&result).len(), 20);
    assert!(!ids(&result).contains(&"p18".to_owned()));
    assert!(f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t18".into())
        })
        .await
        .unwrap()
        .is_none());
    f.clean().await;
}

/// A newer live message committed between HTTP and Persist cannot be overwritten by old body.
#[tokio::test]
async fn stale_http_body_cannot_overwrite_newer_live_version() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.storage
        .batch_delete(BatchDeleteSpec {
            table: "message_sequence_slot",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "seq",
            key_vals: vec![SqlValue::Integer(18)],
        })
        .await
        .unwrap();
    f.late_update = Some(18);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(
        result["body"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == "p18")
            .unwrap()["message"],
        "newer live body"
    );
    f.clean().await;
}

/// A concurrent visibility restoration/body update aborts stale purge and the whole slot write.
#[tokio::test]
async fn stale_hidden_response_cannot_delete_a_newer_live_message() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.visible.remove(&18);
    f.revision = 2;
    f.late_update = Some(18);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert!(result["error"].is_string());
    let row = f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t18".into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(row.iter().any(|(key, value)| key == "message"
        && matches!(value,SqlValue::Text(v) if v=="newer live body")));
    f.clean().await;
}

/// Server omission is not an invisible slot and cannot create completeness evidence.
#[tokio::test]
async fn missing_remote_slot_is_rejected_instead_of_silently_skipped() {
    let mut f = Fixture::new(20, 1..=20).await;
    f.omit_slot = true;
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("INCOMPLETE_RANGE"));
    assert!(f
        .storage
        .scan(ScanSpec {
            table: "message_sequence_slot",
            limit: None,
            filter: None,
            order_by: &[]
        })
        .await
        .unwrap()
        .is_empty());
    f.clean().await;
}

/// int64 sequences stay exact above JavaScript's safe-integer limit in SQL and public cursors.
#[tokio::test]
async fn large_sequences_roundtrip_as_decimal_strings() {
    let head = 9_007_199_254_741_007_i64;
    let mut f = Fixture::new(head, head - 19..=head).await;
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(ids(&result).len(), 20);
    assert_eq!(
        result["body"]["olderCursor"]["nextSeq"],
        (head - 20).to_string()
    );
    f.clean().await;
}

/// Post order survives descending wall clocks, render shaping and a durable cache restart.
#[tokio::test]
async fn post_sequence_projection_preserves_order_when_create_times_descend() {
    let head = 9_007_199_254_741_007_i64;
    let mut f = Fixture::new(head, head - 4..=head).await;
    f.reverse_time = true;
    for warm in [false, true] {
        if warm {
            f.module = module();
            f.requests.clear();
        }
        let result = f
            .query("im_query_messages_by_channel", json!({"pageSize":5}))
            .await;
        assert_eq!(
            ids(&result),
            (head - 4..=head)
                .map(|seq| format!("p{seq}"))
                .collect::<Vec<_>>()
        );
        let rows = result["body"]["messages"].as_array().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row["postSeq"].clone())
                .collect::<Vec<_>>(),
            (head - 4..=head)
                .map(|seq| json!(seq.to_string()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            rows.iter()
                .map(|row| row["createAt"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![5, 4, 3, 2, 1]
        );
        if warm {
            assert_eq!(
                f.requests.len(),
                1,
                "complete persisted range only probes metadata"
            );
        }
    }
    f.clean().await;
}

/// Center locators lend the unavailable newer quota to the older side at the tail.
#[tokio::test]
async fn center_at_tail_still_fills_the_visible_page() {
    let mut f = Fixture::new(30, 1..=30).await;
    let result = f
        .query(
            "im_locate_context",
            json!({"targetPostId":"p30","pageSize":20}),
        )
        .await;
    assert_eq!(
        ids(&result),
        (11..=30).map(|n| format!("p{n}")).collect::<Vec<_>>()
    );
    assert_eq!(result["body"]["targetIndex"], 19);
    assert_eq!(result["body"]["hasNewer"], false);
    f.clean().await;
}

/// The int64 tail has no representable next seq; neither forward nor newer may repeat it.
#[tokio::test]
async fn int64_tail_terminates_without_repeating_anchor() {
    let mut f = Fixture::new(i64::MAX, [i64::MAX]).await;
    let forward = f
        .query(
            "im_locate_context",
            json!({"targetPostId":format!("p{}",i64::MAX),"window":"forward","pageSize":20}),
        )
        .await;
    assert_eq!(ids(&forward), vec![format!("p{}", i64::MAX)]);
    assert_eq!(forward["body"]["hasNewer"], false);
    let newer = f
        .query(
            "im_load_newer_context",
            json!({"anchorPostId":format!("p{}",i64::MAX),"pageSize":20}),
        )
        .await;
    assert!(ids(&newer).is_empty());
    assert_eq!(newer["body"]["hasNewer"], false);
    f.clean().await;
}

/// A body removed between local read and guarded purge must not leave a synthetic row behind.
#[tokio::test]
async fn hidden_purge_after_concurrent_delete_leaves_no_placeholder_message() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.visible.remove(&18);
    f.revision = 2;
    f.late_delete = Some(18);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert!(!ids(&result).contains(&"p18".to_owned()));
    assert!(f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t18".into())
        })
        .await
        .unwrap()
        .is_none());
    f.clean().await;
}

/// Revision, not event seq alone, fences old HTTP when private edits generate no new event.
#[tokio::test]
async fn stale_range_cannot_overwrite_newer_revision_with_equal_event_version() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.storage
        .batch_delete(BatchDeleteSpec {
            table: "message_sequence_slot",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "seq",
            key_vals: vec![SqlValue::Integer(18)],
        })
        .await
        .unwrap();
    f.late_snapshot = Some(18);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert!(result["error"].is_string());
    let row = f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t18".into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(row.iter().any(|(key, value)| key == "message"
        && matches!(value,SqlValue::Text(v) if v=="newer private snapshot")));
    f.clean().await;
}

/// Latest and direct locate both retain live edit subscriptions; locate keeps an anchored update.
#[tokio::test]
async fn sequence_entry_preserves_live_refresh_and_history_anchor() {
    for locate in [false, true] {
        let mut f = Fixture::new(60, 1..=60).await;
        let result = if locate {
            f.query("im_locate_context",json!({"targetPostId":"p30","window":"forward","windowToken":"surface","pageSize":20})).await
        } else {
            f.query(
                "im_query_messages_by_channel",
                json!({"windowToken":"surface","pageSize":20}),
            )
            .await
        };
        assert_eq!(ids(&result).len(), 20);
        let events = f.edit_live().await;
        assert!(
            events
                .iter()
                .any(|event| event["event"] == "im:post:updated"),
            "{events:?}"
        );
        let timeline = events
            .iter()
            .find(|event| event["event"] == "im:timeline:update")
            .expect("successful sequence entry must retain timeline refresh");
        assert_eq!(timeline["data"]["windowToken"], "surface");
        assert_eq!(
            timeline["data"]["direction"],
            if locate { "newer" } else { "append" }
        );
        if locate {
            assert_eq!(timeline["data"]["anchorPostId"], "p30");
        }
        f.clean().await;
    }
}

/// Selection can resolve a post sequence without moving or creating a visible timeline.
#[tokio::test]
async fn detached_sequence_locator_preserves_existing_timeline_subscription() {
    for attached in [false, true] {
        let mut f = Fixture::new(60, 1..=60).await;
        f.query(
            "im_query_messages_by_channel",
            json!({"windowToken":"surface","pageSize":20}),
        )
        .await;
        if !attached {
            f.module = module();
        }
        let result = f
            .query(
                "im_locate_context",
                json!({"targetPostId":"p30","window":"forward","pageSize":1,"attach":false}),
            )
            .await;
        assert_eq!(ids(&result), vec!["p30"]);
        assert_eq!(result["body"]["messages"][0]["postSeq"], "30");
        let events = f.edit_live().await;
        let timeline = events
            .iter()
            .find(|event| event["event"] == "im:timeline:update");
        if attached {
            let timeline = timeline.expect("existing live subscription must survive selection");
            assert_eq!(timeline["data"]["windowToken"], "surface");
            assert_eq!(timeline["data"]["direction"], "append");
        } else {
            assert!(
                timeline.is_none(),
                "read-only selection must not create a timeline"
            );
        }
        f.clean().await;
    }
}

/// Legacy live refresh remains a patch, leaving sequence-aware merge order to its consumer.
#[tokio::test]
async fn descending_clock_sequence_window_receives_live_patch_without_replacement() {
    for locate in [false, true] {
        let mut f = Fixture::new(60, 1..=60).await;
        f.reverse_time = true;
        let result = if locate {
            f.query("im_locate_context",json!({"targetPostId":"p30","window":"forward","windowToken":"surface","pageSize":20})).await
        } else {
            f.query(
                "im_query_messages_by_channel",
                json!({"windowToken":"surface","pageSize":20}),
            )
            .await
        };
        let initial_ids = ids(&result);
        let events = f.edit_live().await;
        assert!(!events
            .iter()
            .any(|event| event["event"] == "im:timeline:ready"));
        let timeline = events
            .iter()
            .find(|event| event["event"] == "im:timeline:update")
            .expect("live patch");
        assert_eq!(
            timeline["data"]["direction"],
            if locate { "newer" } else { "append" }
        );
        let rows = timeline["data"]["messages"]
            .as_array()
            .expect("patch messages");
        eprintln!(
            "sequence-live-patch {}",
            json!({"locate":locate,"initialIds":initial_ids,"direction":timeline["data"]["direction"],"anchorPostId":timeline["data"]["anchorPostId"],"hasOlder":timeline["data"]["hasOlder"],"hasNewer":timeline["data"]["hasNewer"],"rows":rows.iter().map(|row| json!({"id":row["id"],"postSeq":row["postSeq"],"createAt":row["createAt"]})).collect::<Vec<_>>()})
        );
        f.clean().await;
    }
}

const WITHDRAWN_CONTENT: &str = "WITHDRAWN_SEQ_FIXTURE_CONTENT";

/// Populate the existing cache with every body family omitted by Go's revoked shell.
async fn seed_withdrawn_cache(storage: &NativeStorage, seq: i64) {
    let patch = [
        ("message", WITHDRAWN_CONTENT.to_owned()),
        ("simple_message", WITHDRAWN_CONTENT.to_owned()),
        ("props", json!({"text":WITHDRAWN_CONTENT}).to_string()),
        (
            "replied_message",
            json!({"message":WITHDRAWN_CONTENT,"type":"TEXT"}).to_string(),
        ),
        (
            "reply_messages",
            json!({"child":{"message":WITHDRAWN_CONTENT}}).to_string(),
        ),
        ("topic", json!({"message":WITHDRAWN_CONTENT}).to_string()),
        (
            "quick_reply",
            json!([{"emoji":WITHDRAWN_CONTENT,"userIds":["other"]}]).to_string(),
        ),
        (
            "expedite_map",
            json!({"other":{"message":WITHDRAWN_CONTENT}}).to_string(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), SqlValue::Text(value)))
    .collect();
    storage
        .batch_update(BatchUpdateSpec {
            table: "message",
            key_col: "temporary_id",
            key_vals: vec![SqlValue::Text(format!("t{seq}"))],
            patch,
        })
        .await
        .unwrap();
}

/// Assert the public render result keeps identity/order while carrying no withdrawn content.
fn assert_revoked_shell(reply: &Value, seq: i64) {
    let row = reply["body"]["messages"]
        .as_array()
        .expect("message rows")
        .iter()
        .find(|row| row["id"] == format!("p{seq}"))
        .expect("revoked shell must remain visible");
    assert_eq!(row["postSeq"], seq.to_string());
    assert_eq!(row["revoke"], true);
    assert_eq!(row["revoked"], true);
    assert_eq!(row["message"], "");
    assert_eq!(row["text"], "");
    assert!(
        !row.to_string().contains(WITHDRAWN_CONTENT),
        "withdrawn content leaked in a render field"
    );
}

/// A 121-slot revoked history is four complete pages at 20/50, not one NOTICE plus gaps.
#[tokio::test]
async fn revoked_shells_fill_cold_and_warm_history_pages() {
    let mut f = Fixture::new(121, 1..=121).await;
    f.notice_first = true;
    f.revoked.extend(2..=121);
    for warm in [false, true] {
        if warm {
            f.module = module();
        }
        let mut cursor = Value::Null;
        let mut collected = BTreeSet::new();
        for (page, expected) in [20, 50, 50, 1].into_iter().enumerate() {
            f.requests.clear();
            let begin = std::time::Instant::now();
            let result = if page == 0 {
                f.query("im_query_messages_by_channel", json!({"pageSize":20}))
                    .await
            } else {
                f.query(
                    "im_load_older_context",
                    json!({"pageSize":50,"cursor":cursor}),
                )
                .await
            };
            let page_ids = ids(&result);
            assert_eq!(page_ids.len(), expected);
            for id in &page_ids {
                assert!(collected.insert(id.clone()), "duplicate history row");
            }
            for row in result["body"]["messages"].as_array().unwrap() {
                let seq = row["postSeq"].as_str().unwrap().parse::<i64>().unwrap();
                if seq != 1 {
                    assert_revoked_shell(&result, seq);
                }
            }
            let ranges = f
                .requests
                .iter()
                .filter(|request| request.get("fromSeq").is_some())
                .count();
            assert_eq!(ranges, usize::from(!warm));
            assert_eq!(f.requests.len(), if warm { 1 } else { 2 });
            eprintln!(
                "sequence-revoked-page {}",
                json!({"warm":warm,"page":page,"rows":page_ids.len(),"httpRequests":f.requests.len(),"rangeRequests":ranges,"elapsedMs":begin.elapsed().as_secs_f64()*1000.0})
            );
            assert_eq!(result["body"]["hasOlder"], page < 3);
            cursor = result["body"]["olderCursor"].clone();
        }
        assert_eq!(collected, (1..=121).map(|seq| format!("p{seq}")).collect());
    }
    f.clean().await;
}

/// A revoked target remains a valid stable anchor in both locator layouts.
#[tokio::test]
async fn revoked_target_supports_forward_and_center_locate() {
    let mut f = Fixture::new(40, 1..=40).await;
    f.revoked.insert(20);
    for (window, expected_index) in [("forward", 0), ("center", 5)] {
        let result = f
            .query(
                "im_locate_context",
                json!({"targetPostId":"p20","window":window,"pageSize":10}),
            )
            .await;
        assert_eq!(ids(&result).len(), 10);
        assert_eq!(result["body"]["targetIndex"], expected_index);
        assert_revoked_shell(&result, 20);
    }
    f.clean().await;
}

/// Omitted revoked body fields clear the durable old cache instead of being overlaid back.
#[tokio::test]
async fn revoked_snapshot_clears_cached_body_families_before_restart() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    seed_withdrawn_cache(&f.storage, 18).await;
    f.revoked.insert(18);
    f.revision = 2;
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(ids(&result).len(), 20);
    assert_revoked_shell(&result, 18);
    let row = f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t18".into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(
        !row.iter().any(
            |(_, value)| matches!(value,SqlValue::Text(text) if text.contains(WITHDRAWN_CONTENT))
        ),
        "withdrawn content remained durable after authority refresh"
    );
    f.module = module();
    f.requests.clear();
    let repeated = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_revoked_shell(&repeated, 18);
    assert_eq!(f.requests.len(), 1);
    f.clean().await;
}

/// A newer live revoke wins over stale normal HTTP and its old cached body is not rendered.
#[tokio::test]
async fn newer_live_revoke_keeps_shell_without_leaking_cached_body() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    seed_withdrawn_cache(&f.storage, 18).await;
    f.storage
        .batch_delete(BatchDeleteSpec {
            table: "message_sequence_slot",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "seq",
            key_vals: vec![SqlValue::Integer(18)],
        })
        .await
        .unwrap();
    f.late_revoke = Some(18);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(
        ids(&result),
        (11..=30).map(|seq| format!("p{seq}")).collect::<Vec<_>>()
    );
    assert_revoked_shell(&result, 18);
    let row = f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t18".into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(row
        .iter()
        .any(|(key, value)| key == "event_seq" && matches!(value, SqlValue::Integer(999))));
    f.module = module();
    f.requests.clear();
    let repeated = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_revoked_shell(&repeated, 18);
    assert_eq!(f.requests.len(), 1);
    f.clean().await;
}

/// A stale revoked snapshot cannot erase content from a later authoritative live update.
#[tokio::test]
async fn stale_revoked_snapshot_cannot_erase_newer_live_body() {
    let mut f = Fixture::new(30, 1..=30).await;
    f.query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    f.revoked.insert(18);
    f.revision = 2;
    f.late_update = Some(18);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(ids(&result).len(), 20);
    let row = result["body"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "p18")
        .unwrap();
    assert_eq!(row["revoke"], false);
    assert_eq!(row["message"], "newer live body");
    assert_eq!(row["eventSeq"], 999);
    f.clean().await;
}

/// Permission-hidden slots stay body-free even beside visible revoked shells.
#[tokio::test]
async fn hidden_slots_stay_private_beside_visible_revoked_shells() {
    let mut f = Fixture::new(60, (1..=60).filter(|seq| seq % 2 == 0)).await;
    f.revoked.extend(1..=60);
    let result = f
        .query("im_query_messages_by_channel", json!({"pageSize":20}))
        .await;
    assert_eq!(
        ids(&result),
        (11..=30).map(|n| format!("p{}", n * 2)).collect::<Vec<_>>()
    );
    for row in result["body"]["messages"].as_array().unwrap() {
        assert_revoked_shell(&result, row["postSeq"].as_str().unwrap().parse().unwrap());
    }
    let slot = f
        .storage
        .get(GetSpec {
            table: "message_sequence_slot",
            key_col: "slot_key",
            key_val: SqlValue::Text(format!("{CHANNEL}:59")),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(slot
        .iter()
        .any(|(key, value)| key == "is_visible" && matches!(value, SqlValue::Integer(0))));
    assert!(slot
        .iter()
        .any(|(key, value)| key == "temporary_id" && matches!(value, SqlValue::Null)));
    assert!(f
        .storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("t59".into())
        })
        .await
        .unwrap()
        .is_none());
    f.clean().await;
}
