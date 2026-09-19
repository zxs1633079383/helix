//! Go increment wire → 真实 SQLite → channelSyncPage；不连接网络或启动 Tauri。
use helix_core::effect::{GetSpec, Row, ScopedGetSpec, SqlValue};
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

/// 原始 Go 大写 R 的定位 ID 必须落两表并穿过恢复分页。
#[tokio::test]
async fn real_go_unread_anchor_survives_sqlite_and_sync_page() {
    let (storage, mut module) = setup().await;
    let frame = increment_frame();
    assert_eq!(frame["data"]["unReadPostId"], ANCHOR);
    assert!(frame["data"].get("unreadPostId").is_none());
    let count = frame["data"]["unreadCount"].as_i64().unwrap();
    let session = persist_increment(&storage, &mut module, frame).await;
    assert_anchor(&storage, ANCHOR).await;
    assert_page(&storage, &mut module, &session, ANCHOR, count).await;
}

/// 不同字段拼写、冲突更新、稀疏/坏值及明确清空均保持同一读写合同。
#[tokio::test]
async fn unread_anchor_alias_missing_invalid_and_empty_have_distinct_semantics() {
    for (key, value, expected) in [
        ("unReadPostId", Some(json!("replacement")), "replacement"),
        ("unreadPostId", Some(json!("legacy")), "legacy"),
        ("unReadPostId", None, ANCHOR),
        ("unReadPostId", Some(Value::Null), ANCHOR),
        ("unReadPostId", Some(json!(17)), ANCHOR),
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
