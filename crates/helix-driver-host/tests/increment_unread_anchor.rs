//! Go increment wire → 真实 SQLite → channelSyncPage；不连接网络或启动 Tauri。
use helix_core::effect::{GetSpec, Row, ScopedGetSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use helix_im::{state::ChannelId, ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde_json::{json, Value};

const CHANNEL: &str = "bi643wys7fgy9pbai9wjfusnfa";
const VIEWER: &str = "444";
const COMPANY: &str = "64118eebd2b665246b7880eb";
const ANCHOR: &str = "boy4dpqadiy3jktkysxtnm7rqe";

/// 保留真实 Go 字段，只关闭独立的消息补拉，隔离本次落库/分页合同。
fn increment_frame() -> Value {
    let raw = include_str!("../../helix-im/tests/fixtures/real_go_frames/increment_channel.json");
    let mut frame: Value = serde_json::from_str(
        &raw.lines()
            .filter(|line| !line.starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    frame["data"]["needSync"] = json!(false);
    frame
}

/// 每例独享内存 SQLite；schema 由真实业务模块提供。
async fn setup() -> (HostStorage, ImModule) {
    let storage = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .unwrap();
    for sql in IM_SCHEMA.iter().chain(IM_SCHEMA_MIGRATIONS) {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error}"
            );
        }
    }
    (storage, active_module(151))
}

/// 模拟消息同步已追平的登录用户，恢复只测试频道快照。
fn active_module(cursor: u64) -> ImModule {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: VIEWER.into(),
        company_id: COMPANY.into(),
        ..Default::default()
    });
    // 消息游标已追平服务端；本例只验证频道快照，不触发冷启动消息补偿。
    module.register_channel(ChannelId::from_str(CHANNEL).unwrap(), cursor);
    module
}

/// 有界兑现全部存储回执并收集真实 Emit；任何网络 Effect 都使本地用例失败。
async fn drain(storage: &HostStorage, module: &mut ImModule, mut sink: EffectSink) -> Vec<Value> {
    let mut events = Vec::new();
    for _ in 0..16 {
        if sink.is_empty() {
            return events;
        }
        let batch = std::mem::take(&mut sink);
        for effect in batch.as_slice() {
            let (corr, outcome) = match effect {
                Effect::Persist { corr, ops } => (*corr, execute_storage_ops(storage, ops).await),
                Effect::PersistAtomic { corr, ops } => {
                    (*corr, execute_storage_ops_atomic(storage, ops).await)
                }
                Effect::PersistFire { ops } => {
                    assert!(matches!(
                        execute_storage_ops(storage, ops).await,
                        PortOutcome::Ok(_)
                    ));
                    continue;
                }
                Effect::Emit { event } => {
                    events.push(serde_json::from_slice(event.0.as_ref()).unwrap());
                    continue;
                }
                Effect::ScheduleTimer { .. } | Effect::CancelTimer { .. } => continue,
                other => panic!("unexpected non-storage effect: {other:?}"),
            };
            assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
            module
                .handle(&Tick::PortReply { corr, outcome }, 1000, &mut sink)
                .unwrap();
        }
    }
    panic!("storage replies did not terminate");
}

/// increment 单帧不能提前发布 ready；全局封口后由真实 PersistOk 释放分页身份。
async fn persist_increment(storage: &HostStorage, module: &mut ImModule, frame: Value) -> String {
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Inbound(InboundBytes(serde_json::to_vec(&frame).unwrap().into())),
            1000,
            &mut sink,
        )
        .unwrap();
    assert!(!sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    module
        .handle(
            &Tick::Inbound(InboundBytes(
                serde_json::to_vec(&json!({"action":"increment_channel_end","data":{}}))
                    .unwrap()
                    .into(),
            )),
            1001,
            &mut sink,
        )
        .unwrap();
    let events = drain(storage, module, sink).await;
    let ready: Vec<_> = events
        .iter()
        .filter(|event| event["event"] == "im:channel-sync-ready")
        .collect();
    assert_eq!(ready.len(), 1, "{events:?}");
    ready[0]["data"]["channelSyncSessionId"]
        .as_str()
        .unwrap()
        .into()
}

/// 直接比较两张实际表，避免只断言写操作里存在同名字段。
async fn assert_anchor(storage: &HostStorage, expected: &str) {
    let channel = storage
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(CHANNEL.into()),
        })
        .await
        .unwrap()
        .unwrap();
    let member = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "user_id",
            key_val: SqlValue::Text(VIEWER.into()),
        })
        .await
        .unwrap()
        .unwrap();
    for (name, row) in [("channel", channel), ("channel_member", member)] {
        assert!(
            matches!(column(&row, "unread_post_id"), SqlValue::Text(value) if value == expected),
            "{name}: expected {expected:?}, got {:?}",
            column(&row, "unread_post_id")
        );
    }
}

/// Assert all four durable viewer fields together so a torn snapshot cannot pass.
async fn assert_member_projection(
    storage: &HostStorage,
    expected_count: i64,
    expected_anchor: &str,
    expected_read_seq: i64,
    expected_revision: i64,
) {
    let member = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "user_id",
            key_val: SqlValue::Text(VIEWER.into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        column(&member, "unread_count"),
        SqlValue::Integer(value) if *value == expected_count
    ));
    assert!(matches!(
        column(&member, "unread_post_id"),
        SqlValue::Text(value) if value == expected_anchor
    ));
    assert!(matches!(
        column(&member, "last_read_seq"),
        SqlValue::Integer(value) if *value == expected_read_seq
    ));
    assert!(matches!(
        column(&member, "projection_revision"),
        SqlValue::Integer(value) if *value == expected_revision
    ));
}

/// 缺列直接失败，不用空串掩盖回归。
fn column<'a>(row: &'a Row, name: &str) -> &'a SqlValue {
    &row.iter()
        .find(|(key, _)| key == name)
        .expect("required column")
        .1
}

/// 分页只经公开 command 和真实双表 Scan 生成，测试不构造投影 payload。
async fn assert_page(
    storage: &HostStorage,
    module: &mut ImModule,
    session: &str,
    anchor: &str,
    count: i64,
) {
    let mut sink = EffectSink::new();
    module.handle(&Tick::Command(AppCommand {
        name: "im_query_channel_sync_page".into(),
        payload: serde_json::to_vec(&json!({"channel_sync_session_id":session,"page_size":20,"req_id":"unread-anchor-page"})).unwrap().into(),
    }), 1002, &mut sink).unwrap();
    let events = drain(storage, module, sink).await;
    let results: Vec<_> = events
        .iter()
        .filter(|event| event["event"] == "im:read:result")
        .collect();
    assert_eq!(results.len(), 1, "{events:?}");
    let data = &results[0]["data"];
    assert_eq!(data["req_id"], "unread-anchor-page");
    assert!(data.get("error").is_none(), "{data}");
    let items = data["body"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], CHANNEL);
    assert_eq!(items[0]["unreadPostId"], anchor);
    assert_eq!(items[0]["unreadCount"], count);
}

/// 原始 Go 大写 R 的定位 ID 在 count=0 时按合同清空两表并穿过恢复分页。
#[tokio::test]
async fn real_go_zero_count_clears_stale_anchor_in_sqlite_and_sync_page() {
    let (storage, mut module) = setup().await;
    let frame = increment_frame();
    assert_eq!(frame["data"]["unReadPostId"], ANCHOR);
    assert!(frame["data"].get("unreadPostId").is_none());
    let count = frame["data"]["unreadCount"].as_i64().unwrap();
    let session = persist_increment(&storage, &mut module, frame).await;
    assert_anchor(&storage, "").await;
    assert_page(&storage, &mut module, &session, "", count).await;
}

/// 不同字段拼写、冲突更新、稀疏/坏值及明确清空均保持同一读写合同。
#[tokio::test]
async fn unread_anchor_alias_missing_invalid_and_empty_have_distinct_semantics() {
    for (key, value, expected) in [
        ("unReadPostId", Some(json!("replacement")), "replacement"),
        ("unreadPostId", Some(json!("legacy")), "legacy"),
        ("unReadPostId", None, ""),
        ("unReadPostId", Some(Value::Null), ""),
        ("unReadPostId", Some(json!(17)), ""),
        ("unReadPostId", Some(json!("")), ""),
        ("unreadPostId", Some(json!("")), ""),
    ] {
        let (storage, mut module) = setup().await;
        persist_increment(&storage, &mut module, increment_frame()).await;
        // 在相同 SQLite 上重建模块，模拟重连/重启后的既有行冲突更新。
        module = active_module(152);
        let mut next = increment_frame();
        next["data"].as_object_mut().unwrap().remove("unReadPostId");
        if let Some(value) = value {
            next["data"][key] = value;
        }
        next["data"]["unreadCount"] = json!(if expected.is_empty() { 0 } else { 3 });
        next["data"]["lastEventSeq"] = json!(152);
        let session = persist_increment(&storage, &mut module, next).await;
        assert_anchor(&storage, expected).await;
        assert_page(
            &storage,
            &mut module,
            &session,
            expected,
            if expected.is_empty() { 0 } else { 3 },
        )
        .await;
    }
}

/// Build a complete absolute projection using the existing Go wire spelling.
fn versioned_frame(revision: i64, count: i64, anchor: &str, read_seq: i64) -> Value {
    let mut frame = increment_frame();
    frame["data"]["unreadCount"] = json!(count);
    frame["data"]["unReadPostId"] = json!(anchor);
    frame["data"]["lastReadSeq"] = json!(read_seq);
    frame["data"]["projectionRevision"] = json!(revision);
    frame["data"]["lastEventSeq"] = json!(151);
    frame
}

/// Member projection rows are guarded by projectionRevision, so an old, duplicate, or
/// reordered snapshot cannot overwrite count, anchor, read boundary, or revision.
#[tokio::test]
async fn versioned_member_projection_is_strictly_monotonic_and_clears_zero_anchor() {
    let (storage, mut module) = setup().await;
    persist_increment(&storage, &mut module, versioned_frame(2, 3, "post-r2", 41)).await;
    module = active_module(151);
    persist_increment(&storage, &mut module, versioned_frame(4, 7, "post-r4", 53)).await;
    module = active_module(151);
    persist_increment(
        &storage,
        &mut module,
        versioned_frame(3, 99, "stale-r3", 49),
    )
    .await;
    assert_member_projection(&storage, 7, "post-r4", 53, 4).await;
    module = active_module(151);
    persist_increment(
        &storage,
        &mut module,
        versioned_frame(4, 99, "duplicate-must-not-win", 53),
    )
    .await;
    assert_member_projection(&storage, 7, "post-r4", 53, 4).await;

    module = active_module(151);
    persist_increment(
        &storage,
        &mut module,
        versioned_frame(5, 0, "stale-anchor-must-clear", 60),
    )
    .await;
    assert_member_projection(&storage, 0, "", 60, 5).await;
}

/// A legacy increment has no projection revision and must not clobber an already-versioned
/// member row even when its event sequence is newer.
#[tokio::test]
async fn legacy_increment_cannot_overwrite_versioned_member_projection() {
    let (storage, mut module) = setup().await;
    persist_increment(
        &storage,
        &mut module,
        versioned_frame(8, 4, "authoritative-r8", 80),
    )
    .await;
    module = active_module(160);
    let mut legacy = increment_frame();
    legacy["data"]["unreadCount"] = json!(91);
    legacy["data"]["unReadPostId"] = json!("legacy-must-not-win");
    legacy["data"]["lastReadSeq"] = json!(12);
    legacy["data"]["lastEventSeq"] = json!(160);
    let session = persist_increment(&storage, &mut module, legacy).await;
    assert_member_projection(&storage, 4, "authoritative-r8", 80, 8).await;
    assert_page(&storage, &mut module, &session, "authoritative-r8", 4).await;
}

/// A legacy zero-count snapshot clears a stale anchor at both durable table boundaries.
#[tokio::test]
async fn legacy_zero_count_clears_stale_anchor() {
    let (storage, mut module) = setup().await;
    let mut seeded = increment_frame();
    seeded["data"]["unreadCount"] = json!(3);
    seeded["data"]["unReadPostId"] = json!("stale-before-zero");
    seeded["data"]["lastEventSeq"] = json!(150);
    persist_increment(&storage, &mut module, seeded).await;
    module = active_module(151);
    let mut legacy = increment_frame();
    legacy["data"]["unreadCount"] = json!(0);
    legacy["data"]
        .as_object_mut()
        .unwrap()
        .remove("unReadPostId");
    legacy["data"]["lastEventSeq"] = json!(151);
    let session = persist_increment(&storage, &mut module, legacy).await;
    assert_anchor(&storage, "").await;
    assert_page(&storage, &mut module, &session, "", 0).await;
}

/// Exercise both unread rounds through real SQLite and the public channelSyncPage Emit seam.
#[tokio::test]
async fn two_rounds_replace_the_read_entry_anchor_and_ignore_late_first_round() {
    let (storage, mut module) = setup().await;
    let first =
        persist_increment(&storage, &mut module, versioned_frame(18, 4, "post-32", 29)).await;
    assert_page(&storage, &mut module, &first, "post-32", 4).await;
    module = active_module(151);
    persist_increment(&storage, &mut module, versioned_frame(19, 0, "", 38)).await;
    assert_member_projection(&storage, 0, "", 38, 19).await;
    module = active_module(151);
    persist_increment(&storage, &mut module, versioned_frame(23, 4, "post-41", 38)).await;
    module = active_module(151);
    let second =
        persist_increment(&storage, &mut module, versioned_frame(18, 4, "post-32", 29)).await;
    assert_member_projection(&storage, 4, "post-41", 38, 23).await;
    assert_page(&storage, &mut module, &second, "post-41", 4).await;
}

/// 通过真实 SQLite 和公开 Command 逐页读取，删除记录占满前两页也不能提前完成。
#[tokio::test]
async fn deleted_channel_pages_continue_to_live_tail_without_zero_revision_items() {
    let (storage, mut module) = setup().await;
    let session = persist_increment(&storage, &mut module, increment_frame()).await;
    let mut channels = Vec::new();
    let mut members = Vec::new();
    for index in 0..41 {
        let id = format!("c{index:025}");
        let deleted = if index < 40 { 123 } else { 0 };
        channels.push(vec![
            ("id".into(), SqlValue::Text(id.clone())),
            ("team_id".into(), SqlValue::Text(COMPANY.into())),
            ("user_id".into(), SqlValue::Text(VIEWER.into())),
            ("type".into(), SqlValue::Text("P".into())),
            ("is_top".into(), SqlValue::Integer(1)),
            (
                "last_post_at".into(),
                SqlValue::Integer(1_900_000_000_000 - index),
            ),
            ("delete_at".into(), SqlValue::Integer(deleted)),
        ]);
        members.push(vec![
            ("channel_id".into(), SqlValue::Text(id)),
            ("user_id".into(), SqlValue::Text(VIEWER.into())),
            ("team_id".into(), SqlValue::Text(COMPANY.into())),
            ("role".into(), SqlValue::Text("OWNER".into())),
            ("unread_count".into(), SqlValue::Integer(0)),
            ("unread_post_id".into(), SqlValue::Text(String::new())),
            ("last_read_seq".into(), SqlValue::Integer(0)),
            (
                "projection_revision".into(),
                SqlValue::Integer(if deleted > 0 { 0 } else { 1 }),
            ),
        ]);
    }
    storage
        .batch_upsert(UpsertSpec::new("channel", channels, Some("id")))
        .await
        .unwrap();
    storage
        .batch_upsert(UpsertSpec::new(
            "channel_member",
            members,
            Some("channel_id,user_id"),
        ))
        .await
        .unwrap();
    let mut cursor = Value::Null;
    let mut ids = Vec::new();
    for page_index in 0..3 {
        let mut sink = EffectSink::new();
        module
            .handle(
                &Tick::Command(AppCommand {
                    name: "im_query_channel_sync_page".into(),
                    payload: serde_json::to_vec(&json!({"channel_sync_session_id":session,
                "page_size":20,"req_id":"deleted-page","next_cursor":cursor}))
                    .unwrap()
                    .into(),
                }),
                1002 + page_index,
                &mut sink,
            )
            .unwrap();
        let events = drain(&storage, &mut module, sink).await;
        let replies: Vec<_> = events
            .iter()
            .filter(|e| e["event"] == "im:read:result")
            .collect();
        assert_eq!(replies.len(), 1);
        let data = &replies[0]["data"];
        assert!(data.get("error").is_none(), "{data}");
        let body = &data["body"];
        let items = body["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            if page_index < 2 { 0 } else { 2 },
            "page {page_index}: {body}"
        );
        assert_eq!(body["hasMore"], json!(page_index < 2));
        for item in items {
            assert!(item["deleteAt"].as_i64().unwrap_or(0) <= 0);
            ids.push(item["id"].as_str().unwrap().to_owned());
        }
        cursor = body["nextCursor"].clone();
    }
    assert_eq!(ids, vec![format!("c{:025}", 40), CHANNEL.to_owned()]);
    assert!(cursor.is_null());
}

/// 关闭只有 terminal marker、没有时间戳；旧库恢复后迟到快照也必须保持只读。
#[tokio::test]
async fn closed_marker_survives_restart_and_late_increment_in_channel_page() {
    let (storage, mut module) = setup().await;
    persist_increment(&storage, &mut module, versioned_frame(1, 0, "", 151)).await;
    let marker = helix_im::acl::to_effect::terminal_tombstone_and_cursor_op(ChannelId::from_str(CHANNEL).unwrap(), helix_im::state::Seq(152));
    assert!(matches!(execute_storage_ops_atomic(&storage, &[marker]).await, PortOutcome::Ok(_)));
    // 重放升级迁移，模拟现场 delete_at=0、terminal_event_seq>0 的历史库。
    for sql in IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error}"
            );
        }
    }
    let mut restarted = active_module(152);
    let session = persist_increment(&storage, &mut restarted, versioned_frame(2, 0, "", 151)).await;
    let mut sink = EffectSink::new();
    restarted
        .handle(
            &Tick::Command(AppCommand::new(
                "im_query_channel_sync_page",
                serde_json::to_vec(
                    &json!({"channel_sync_session_id":session,"req_id":"closed-page"}),
                )
                .unwrap(),
            )),
            1002,
            &mut sink,
        )
        .unwrap();
    let events = drain(&storage, &mut restarted, sink).await;
    let result = events
        .iter()
        .find(|event| event["event"] == "im:read:result")
        .unwrap();
    assert_eq!(
        result["data"]["body"]["items"][0]["close"], true,
        "{events:?}"
    );
    assert_eq!(result["data"]["body"]["items"][0]["deleteAt"], 0);
}

/// 在线 canonical terminal 的关闭投影必须随 cursor 原子提交，而不是只发一次 UI 通知。
#[tokio::test]
async fn live_terminal_persists_closed_channel_projection() {
    let (storage, mut module) = setup().await;
    persist_increment(&storage, &mut module, versioned_frame(1, 0, "", 151)).await;
    let mut sink = EffectSink::new();
    let frame = json!({"action":"channel_stream_event","data":{
        "channel_id":CHANNEL,"stream_seq":152,"event_type":7,"event_id":"closed-event",
        "payload":{"state":"closed"}
    }});
    module
        .handle(
            &Tick::Inbound(InboundBytes(serde_json::to_vec(&frame).unwrap().into())),
            1002,
            &mut sink,
        )
        .unwrap();
    assert!(!sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    drain(&storage, &mut module, sink).await;
    let row = storage
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(CHANNEL.into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(column(&row, "is_closed"), SqlValue::Integer(1)));
    assert!(matches!(column(&row, "delete_at"), SqlValue::Integer(0)));
}
