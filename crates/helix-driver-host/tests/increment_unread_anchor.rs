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
        api_base_url: "http://cold-recovery.invalid/api/cses".into(),
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
    let mut member = complete_member(revision, &format!("effect-{revision}"));
    member["unreadCount"] = json!(count);
    member["unreadPostId"] = json!(anchor);
    member["lastReadSeq"] = json!(read_seq);
    frame["data"]["memberProjection"] = member;
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
    let marker = helix_im::acl::to_effect::terminal_tombstone_and_cursor_op(
        ChannelId::from_str(CHANNEL).unwrap(),
        helix_im::state::Seq(152),
    );
    assert!(matches!(
        execute_storage_ops_atomic(&storage, &[marker]).await,
        PortOutcome::Ok(_)
    ));
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

/// 回归锚: 1012 摘要版本衔接。SQLite 中的正版本成员事实不被无版本 POST 改写。
#[tokio::test]
async fn post_preserves_versioned_member_summary_but_bootstraps_legacy_row() {
    for revision in [0, 13] {
        let (storage, mut module) = setup().await;
        storage
            .batch_upsert(UpsertSpec {
                table: "channel_member",
                version_column: None,
                update_guard: None,
                conflict_key: Some("channel_id,user_id"),
                exclude_from_update: vec![],
                rows: vec![vec![
                    ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                    ("user_id".into(), SqlValue::Text(VIEWER.into())),
                    ("projection_revision".into(), SqlValue::Integer(revision)),
                    ("effect_id".into(), SqlValue::Text("server-effect".into())),
                    ("unread_count".into(), SqlValue::Integer(7)),
                    ("unread_post_id".into(), SqlValue::Text("stored".into())),
                    (
                        "last_post".into(),
                        SqlValue::Text(json!({"id":"stored"}).to_string()),
                    ),
                    ("last_post_at".into(), SqlValue::Integer(100)),
                    ("last_root_post_at".into(), SqlValue::Integer(100)),
                    ("last_unread_event_seq".into(), SqlValue::Integer(0)),
                ]],
            })
            .await
            .unwrap();
        let post = json!({ "action": "post", "data": { "channelId": CHANNEL, "event_seq": 152, "post": {
            "id": "newfix00000000000000000001", "temporaryId": "tmpfix00000000000000000001", "channelId": CHANNEL,
            "userId": "other-viewer", "type": "TEXT", "message": "new", "simpleMessage": "new",
            "viewers": ["all"], "props": {}, "createAt": 200, "updateAt": 200
        } } });
        let mut sink = EffectSink::new();
        module
            .handle(
                &Tick::Inbound(InboundBytes(serde_json::to_vec(&post).unwrap().into())),
                200,
                &mut sink,
            )
            .unwrap();
        let ops = sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::PersistAtomic { ops, .. } => Some(ops),
                _ => None,
            })
            .expect("post atomic write");
        assert!(matches!(
            execute_storage_ops_atomic(&storage, ops).await,
            PortOutcome::Ok(_)
        ));
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
        assert!(
            matches!(column(&member, "projection_revision"), SqlValue::Integer(value) if *value == revision)
        );
        let expected_count = if revision == 0 { 8 } else { 7 };
        assert!(
            matches!(column(&member, "unread_count"), SqlValue::Integer(value) if *value == expected_count)
        );
        let SqlValue::Text(last_post) = column(&member, "last_post") else {
            panic!("last_post JSON")
        };
        let last_post: Value = serde_json::from_str(last_post).unwrap();
        assert_eq!(
            last_post["id"],
            if revision == 0 {
                "newfix00000000000000000001"
            } else {
                "stored"
            }
        );
    }
}

fn complete_member(revision: i64, effect: &str) -> Value {
    json!({"channelId":CHANNEL,"userId":VIEWER,"projectionRevision":revision,
        "effectId":effect,"unreadCount":0,"reactionUnreadCount":2,"unreadPostId":"",
        "lastPost":"","lastReadSeq":77,"mentionList":[],"mentionCount":0,
        "mentionCountRoot":0,"mentionUser":"","urgentCount":0,"urgentPostList":[],
        "urgentMentionUser":"","msgCount":30,"msgCountRoot":30,"msgCountPrivate":0,
        "lastPostAt":12345,"lastRootPostAt":12345})
}

#[tokio::test]
async fn cold_snapshot_persists_complete_projection() {
    let (storage, mut module) = setup().await;
    let mut frame = versioned_frame(61, 0, "", 77);
    frame["data"]["memberProjection"] = complete_member(61, "full-61");
    persist_increment(&storage, &mut module, frame).await;
    let row = storage
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
    assert!(matches!(column(&row,"effect_id"),SqlValue::Text(value) if value == "full-61"));
    assert!(matches!(column(&row, "msg_count"), SqlValue::Integer(30)));
    assert!(matches!(
        column(&row, "reaction_unread_count"),
        SqlValue::Integer(2)
    ));
}

#[tokio::test]
async fn cold_legacy_snapshot_cannot_claim_canonical_revision() {
    let (storage, mut module) = setup().await;
    let mut legacy = versioned_frame(61, 0, "", 77);
    legacy["data"]
        .as_object_mut()
        .unwrap()
        .remove("memberProjection");
    persist_increment(&storage, &mut module, legacy).await;
    let row = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL.into()),
            key_col: "user_id",
            key_val: SqlValue::Text(VIEWER.into()),
        })
        .await
        .unwrap();
    assert!(row
        .as_ref()
        .is_none_or(|row| matches!(column(row, "projection_revision"), SqlValue::Integer(0))));
}

#[tokio::test]
async fn cold_same_revision_empty_effect_is_repaired_without_overwriting_conflicts() {
    for (stored_revision, stored_effect, expected_effect, expected_count) in [
        (61, "", "full-61", 30),
        (60, "", "full-61", 30),
        (62, "", "", 0),
        (61, "different", "different", 0),
        (i64::MAX, "", "full-max", 30),
    ] {
        let (storage, mut module) = setup().await;
        storage
            .batch_upsert(UpsertSpec::new(
                "channel_member",
                vec![vec![
                    ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                    ("user_id".into(), SqlValue::Text(VIEWER.into())),
                    (
                        "projection_revision".into(),
                        SqlValue::Integer(stored_revision),
                    ),
                    ("effect_id".into(), SqlValue::Text(stored_effect.into())),
                ]],
                Some("channel_id,user_id"),
            ))
            .await
            .unwrap();
        let revision = if stored_revision == i64::MAX {
            i64::MAX
        } else {
            61
        };
        let effect = if revision == i64::MAX {
            "full-max"
        } else {
            "full-61"
        };
        let wire = json!({"action":"member_projection","data":complete_member(revision,effect)});
        let mut sink = EffectSink::new();
        module
            .handle(
                &Tick::Inbound(InboundBytes(serde_json::to_vec(&wire).unwrap().into())),
                1000,
                &mut sink,
            )
            .unwrap();
        drain(&storage, &mut module, sink).await;
        let row = storage
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
        assert!(
            matches!(column(&row,"effect_id"),SqlValue::Text(value) if value == expected_effect),
            "{row:?}"
        );
        assert!(
            matches!(column(&row,"msg_count"),SqlValue::Integer(value) if *value == expected_count),
            "{row:?}"
        );
    }
}

#[tokio::test]
async fn cold_snapshot_rejects_missing_fields_and_foreign_identity() {
    let full = complete_member(61, "effect-61");
    let mut invalid: Vec<Value> = full
        .as_object()
        .unwrap()
        .keys()
        .map(|key| {
            let mut member = full.clone();
            member.as_object_mut().unwrap().remove(key);
            member
        })
        .collect();
    for (key, value) in [
        ("userId", json!("another-user")),
        ("channelId", json!("another-channel")),
        ("projectionRevision", json!(62)),
        ("effectId", json!("")),
        ("mentionList", json!("[]")),
        ("msgCount", json!(-1)),
        ("lastReadSeq", json!(78)),
    ] {
        let mut member = full.clone();
        member[key] = value;
        invalid.push(member);
    }
    for member in invalid {
        let (storage, mut module) = setup().await;
        let mut frame = versioned_frame(61, 0, "", 77);
        frame["data"]["memberProjection"] = member.clone();
        persist_increment(&storage, &mut module, frame).await;
        let row = storage
            .scoped_get(ScopedGetSpec {
                table: "channel_member",
                scope_col: "channel_id",
                scope_val: SqlValue::Text(CHANNEL.into()),
                key_col: "user_id",
                key_val: SqlValue::Text(VIEWER.into()),
            })
            .await
            .unwrap();
        assert!(
            row.as_ref().is_none_or(|row| matches!(
                column(row, "projection_revision"),
                SqlValue::Integer(0)
            )),
            "{member}"
        );
    }
}

async fn reaction_http_effects(module: &mut ImModule, member: Value, ack: bool) -> EffectSink {
    let mut sink = EffectSink::new();
    let mut args = json!({"channel_id":CHANNEL,"req_id":"cold-reaction"});
    if ack {
        args["post_id"] = json!("reacted-post");
        args["observed_revision"] = json!("61");
    }
    module
        .handle(
            &Tick::Command(AppCommand::new(
                if ack {
                    "im_reaction_unread_ack"
                } else {
                    "im_reaction_unread"
                },
                serde_json::to_vec(&args).unwrap(),
            )),
            1001,
            &mut sink,
        )
        .unwrap();
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("authorized reaction HTTP");
    sink.clear();
    let count = member["reactionUnreadCount"].clone();
    let body = json!({"status":"SUCCESS","data":{"count":count,"nextTarget":if count == 0 {Value::Null} else {json!({"postId":"reacted-post","revision":"61"})},"memberProjection":member}});
    let reply = json!({"status":200,"headers":{},"body":helix_driver_host::base64_encode(&serde_json::to_vec(&body).unwrap())});
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    serde_json::to_vec(&reply).unwrap().into(),
                )),
            },
            1002,
            &mut sink,
        )
        .unwrap();
    sink
}

#[tokio::test]
async fn cold_query_and_ack_heal_legacy_cache_and_reject_newer_readback() {
    for ack in [false, true] {
        for newer_race in [false, true] {
            let (storage, mut module) = setup().await;
            persist_increment(&storage, &mut module, increment_frame()).await;
            let mut query = EffectSink::new();
            module
                .handle(
                    &Tick::Command(AppCommand::new("im_query_dialog_list", b"{}".to_vec())),
                    1000,
                    &mut query,
                )
                .unwrap();
            drain(&storage, &mut module, query).await;
            storage
                .batch_upsert(UpsertSpec::new(
                    "channel_member",
                    vec![vec![
                        ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                        ("user_id".into(), SqlValue::Text(VIEWER.into())),
                        ("projection_revision".into(), SqlValue::Integer(61)),
                        ("effect_id".into(), SqlValue::Text(String::new())),
                    ]],
                    Some("channel_id,user_id"),
                ))
                .await
                .unwrap();
            let mut member = complete_member(61, "query-61");
            if ack {
                member["reactionUnreadCount"] = json!(0);
            }
            let pending = reaction_http_effects(&mut module, member, ack).await;
            let (corr, ops) = pending
                .as_slice()
                .iter()
                .find_map(|effect| match effect {
                    Effect::PersistAtomic { corr, ops } => Some((*corr, ops)),
                    _ => None,
                })
                .expect("durable projection write");
            let outcome = execute_storage_ops_atomic(&storage, ops).await;
            let mut readback = EffectSink::new();
            module
                .handle(&Tick::PortReply { corr, outcome }, 1003, &mut readback)
                .unwrap();
            assert!(readback
                .as_slice()
                .iter()
                .all(|effect| !matches!(effect, Effect::Emit { .. })));
            if newer_race {
                let mut newer = EffectSink::new();
                module.handle(&Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"member_projection","data":complete_member(62,"newer-62")})).unwrap().into())),1004,&mut newer).unwrap();
                drain(&storage, &mut module, newer).await;
            }
            let events = drain(&storage, &mut module, readback).await;
            let result = events
                .iter()
                .find(|event| event["event"] == "im:read:result")
                .expect("one terminal");
            if newer_race {
                assert_eq!(
                    result["data"]["error"],
                    "REACTION_UNREAD_PROJECTION_CHANGED"
                );
            } else {
                assert!(result["data"].get("error").is_none(), "{result}");
                assert_eq!(
                    result["data"]["body"]["data"]["count"],
                    if ack { 0 } else { 2 }
                );
            }
        }
    }
}

async fn cold_page(storage: &HostStorage, module: &mut ImModule, session: &str) -> Value {
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_query_channel_sync_page",
                serde_json::to_vec(
                    &json!({"channel_sync_session_id":session,"req_id":"cold-page"}),
                )
                .unwrap(),
            )),
            1002,
            &mut sink,
        )
        .unwrap();
    let events = drain(storage, module, sink).await;
    events
        .iter()
        .find(|e| e["event"] == "im:read:result")
        .expect("page")["data"]["body"]["items"][0]
        .clone()
}

#[tokio::test]
async fn cold_page_never_claims_incomplete_revision_and_repair_exposes_canonical_count() {
    let (storage, mut module) = setup().await;
    let mut frame = versioned_frame(61, 0, "", 77);
    frame["data"]
        .as_object_mut()
        .unwrap()
        .remove("memberProjection");
    let session = persist_increment(&storage, &mut module, frame).await;
    let page = cold_page(&storage, &mut module, &session).await;
    assert!(!page.is_null());
    assert!(page.get("projectionRevision").is_none(), "{page}");
    storage
        .batch_upsert(UpsertSpec::new(
            "channel_member",
            vec![vec![
                ("channel_id".into(), SqlValue::Text(CHANNEL.into())),
                ("user_id".into(), SqlValue::Text(VIEWER.into())),
                ("projection_revision".into(), SqlValue::Integer(61)),
                ("effect_id".into(), SqlValue::Text(String::new())),
            ]],
            Some("channel_id,user_id"),
        ))
        .await
        .unwrap();
    let page = cold_page(&storage, &mut module, &session).await;
    assert!(page.get("projectionRevision").is_none(), "{page}");
    let pending = reaction_http_effects(&mut module, complete_member(61, "query-61"), false).await;
    let events = drain(&storage, &mut module, pending).await;
    assert!(
        events.iter().any(|e| e["event"] == "im:channel:update"
            && e["data"]["projectionRevision"] == 61
            && e["data"]["reactionUnreadCount"] == 2),
        "{events:?}"
    );
    let page = cold_page(&storage, &mut module, &session).await;
    assert_eq!(page["projectionRevision"], 61);
    assert_eq!(page["reactionUnreadCount"], 2);
}

#[tokio::test]
async fn cold_legacy_display_counts_do_not_claim_or_overwrite_canonical_state() {
    let (storage, mut module) = setup().await;
    let mut legacy = versioned_frame(61, 7, "legacy-anchor", 70);
    legacy["data"]
        .as_object_mut()
        .unwrap()
        .remove("memberProjection");
    legacy["data"]["reactionUnreadCount"] = json!(2);
    legacy["data"]["msgCount"] = json!(900);
    let session = persist_increment(&storage, &mut module, legacy.clone()).await;
    let page = cold_page(&storage, &mut module, &session).await;
    assert_eq!(page["unreadCount"], 7);
    assert_eq!(page["reactionUnreadCount"], 2);
    assert!(page.get("projectionRevision").is_none());
    module = active_module(151);
    persist_increment(&storage, &mut module, versioned_frame(62, 0, "", 77)).await;
    module = active_module(160);
    legacy["data"]["lastEventSeq"] = json!(160);
    legacy["data"]["reactionUnreadCount"] = json!(99);
    let session = persist_increment(&storage, &mut module, legacy).await;
    let page = cold_page(&storage, &mut module, &session).await;
    assert_eq!(page["unreadCount"], 0);
    assert_eq!(page["reactionUnreadCount"], 2);
    assert_eq!(page["projectionRevision"], 62);
    let row = storage
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
    assert!(matches!(column(&row, "msg_count"), SqlValue::Integer(30)));
}
