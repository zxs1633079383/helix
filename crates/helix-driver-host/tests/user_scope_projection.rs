//! 真 SQLite 的用户级共库存储与重开后的离线公司投影隔离，不发送网络请求。
use helix_core::effect::{GetSpec, SqlValue, StorageOp, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::AppCommand;
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use helix_im::{event::MessageV3Event, ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde_json::{json, Value};

const A: &str = "chfixx00000000000000000001";
const B: &str = "chfixx00000000000000000002";

fn module(company: &str) -> ImModule {
    ImModule::new(ImConfig {
        auth_user_id: "scope-user".into(),
        company_id: company.into(),
        ..Default::default()
    })
}

/// 只兑现真实存储端口，任何网络 Effect 都使离线测试失败。
async fn drain(storage: &HostStorage, module: &mut ImModule, sink: &mut EffectSink) -> Vec<Value> {
    let mut events = Vec::new();
    for _ in 0..16 {
        if sink.is_empty() {
            return events;
        }
        let batch = std::mem::take(sink);
        for effect in batch.as_slice() {
            let (corr, outcome) = match effect {
                Effect::Persist { corr, ops } => (*corr, execute_storage_ops(storage, ops).await),
                Effect::PersistAtomic { corr, ops } => {
                    (*corr, execute_storage_ops_atomic(storage, ops).await)
                }
                Effect::Emit { event } => {
                    events.push(serde_json::from_slice(event.0.as_ref()).unwrap());
                    continue;
                }
                Effect::Http { .. } | Effect::HttpFire { .. } | Effect::Send { .. } => {
                    panic!("offline test must not issue network I/O")
                }
                _ => continue,
            };
            assert!(
                matches!(outcome, helix_core::tick::PortOutcome::Ok(_)),
                "{outcome:?}"
            );
            module
                .handle(&Tick::PortReply { corr, outcome }, 1, sink)
                .unwrap();
        }
    }
    panic!("storage lifecycle did not terminate");
}

/// 同一文件保存 A/B，B 在线不广播；重启切换 A/B 时各自只返回自己的左侧群聊。
#[tokio::test]
async fn foreign_company_persists_but_never_renders_until_selected() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "helix-user-scope-{}-{stamp}.db",
        std::process::id()
    ));
    assert!(!path.exists());
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let storage = HostStorage::open_sqlite_url(&url).await.unwrap();
    for sql in IM_SCHEMA.iter().chain(IM_SCHEMA_MIGRATIONS) {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error}"
            );
        }
    }
    let mut active = module("company-a");
    for (channel, company) in [(A, "company-a"), (B, "company-b")] {
        let mut sink = EffectSink::new();
        active.queue_message_v3_commit(
            vec![StorageOp::BatchUpsert(UpsertSpec::new(
                "channel",
                vec![vec![
                    ("id".into(), SqlValue::Text(channel.into())),
                    ("team_id".into(), SqlValue::Text(company.into())),
                    ("user_id".into(), SqlValue::Text("scope-user".into())),
                    ("display_name".into(), SqlValue::Text(company.into())),
                    ("is_active".into(), SqlValue::Integer(1)),
                ]],
                Some("id"),
            ))],
            vec![MessageV3Event::new(
                "im:channel:update",
                json!({"channelId":channel,"displayName":company}),
            )
            .unwrap()],
            &mut sink,
        );
        let events = drain(&storage, &mut active, &mut sink).await;
        assert_eq!(events.len(), usize::from(channel == A));
    }
    drop(active);
    // 当前公司最后一条来自其他作者；旧消息的 update_at 和外公司的更大 create_at 均不能冒充水位。
    storage
        .batch_upsert(UpsertSpec::new(
            "message",
            [
                ("a-old", A, "scope-user", 100, 9999),
                ("a-last", A, "other-author", 300, 300),
                ("b-last", B, "scope-user", 900, 900),
            ]
            .into_iter()
            .map(|(id, channel, author, created, updated)| {
                vec![
                    ("temporary_id".into(), SqlValue::Text(id.into())),
                    ("channel_id".into(), SqlValue::Text(channel.into())),
                    ("user_id".into(), SqlValue::Text(author.into())),
                    ("create_at".into(), SqlValue::Integer(created)),
                    ("update_at".into(), SqlValue::Integer(updated)),
                ]
            })
            .collect(),
            Some("temporary_id"),
        ))
        .await
        .unwrap();
    drop(storage);
    let reopened = HostStorage::open_sqlite_url(&url).await.unwrap();
    for (company, channel) in [("company-a", A), ("company-b", B)] {
        assert!(
            reopened
                .get(GetSpec {
                    table: "channel",
                    key_col: "id",
                    key_val: SqlValue::Text(channel.into())
                })
                .await
                .unwrap()
                .is_some(),
            "both companies must remain durable"
        );
        let mut offline = module(company);
        let mut sink = EffectSink::new();
        offline
            .handle(
                &Tick::Command(AppCommand::new(
                    "im_query_dialog_list",
                    br#"{"req_id":"offline"}"#.to_vec(),
                )),
                0,
                &mut sink,
            )
            .unwrap();
        let events = drain(&reopened, &mut offline, &mut sink).await;
        let result = events
            .iter()
            .find(|event| event["event"] == "im:read:result")
            .unwrap();
        let items = result["data"]["body"]["items"]
            .as_array()
            .expect("offline query must return current-company body");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], channel);

        // 模拟上线 hello，只执行本地查询并检查 HTTP 意图，绝不发送真实网络请求。
        sink.clear();
        offline
            .handle(
                &Tick::Inbound(helix_core::tick::InboundBytes(
                    serde_json::to_vec(
                        &json!({"action":"hello","data":{"connectionId":"scope-test"}}),
                    )
                    .unwrap()
                    .into(),
                )),
                2,
                &mut sink,
            )
            .unwrap();
        let (corr, ops) = sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
                _ => None,
            })
            .expect("hello must read a scoped local watermark before HTTP");
        assert!(matches!(ops.as_slice(), [StorageOp::ScopedMax(_)]));
        let outcome = execute_storage_ops(&reopened, &ops).await;
        sink.clear();
        offline
            .handle(&Tick::PortReply { corr, outcome }, 3, &mut sink)
            .unwrap();
        let body: Value = sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { req, .. } if req.url.ends_with("/channels/load/increment") => req
                    .body
                    .as_ref()
                    .map(|body| serde_json::from_slice(body).unwrap()),
                _ => None,
            })
            .expect("local read must feed increment HTTP intent");
        assert_eq!(body["timestamp"], if channel == A { 300 } else { 900 });
    }
    drop(reopened);
    std::fs::remove_file(&path).unwrap();
    for suffix in ["db-wal", "db-shm"] {
        let sidecar = path.with_extension(suffix);
        if sidecar.exists() {
            std::fs::remove_file(sidecar).unwrap();
        }
    }
}
