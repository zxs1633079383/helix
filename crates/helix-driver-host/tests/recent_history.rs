//! 使用真实 SQLite 和通用驱动验证最近记录协议，不复制业务存储规则。
use helix_core::{effect::Effect, tick::AppCommand, EffectSink, Module, Tick};
use helix_driver_host::{execute_storage_ops, HostStorage};
use helix_im::{
    module::{ImConfig, ImModule},
    IM_SCHEMA, IM_SCHEMA_MIGRATIONS,
};
use serde_json::{json, Value};

fn module(user: &str) -> ImModule {
    ImModule::new(ImConfig {
        auth_user_id: user.into(),
        company_id: "company".into(),
        api_base_url: "https://im.test".into(),
        ..Default::default()
    })
}
async fn command(
    storage: &HostStorage,
    module: &mut ImModule,
    action: &str,
    targets: Value,
) -> Value {
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_recent_history",
                serde_json::to_vec(&json!({
                    "kind":"forward", "action":action, "targets":targets, "req_id":"test",
                    "scope":json!(["https://im.test","company","actor"]).to_string()
                }))
                .unwrap(),
            )),
            0,
            &mut sink,
        )
        .unwrap();
    let mut result = None;
    while !sink.is_empty() {
        let effects = std::mem::take(&mut sink);
        for effect in effects.as_slice() {
            match effect {
                Effect::Persist { corr, ops } => {
                    let outcome = execute_storage_ops(storage, ops).await;
                    module
                        .handle(
                            &Tick::PortReply {
                                corr: *corr,
                                outcome,
                            },
                            0,
                            &mut sink,
                        )
                        .unwrap();
                }
                Effect::Emit { event } => {
                    let event: Value = serde_json::from_slice(&event.0).unwrap();
                    result = Some(event["data"]["body"].clone());
                }
                _ => panic!("unexpected external effect"),
            }
        }
    }
    result.expect("must reply")
}
#[tokio::test]
async fn sqlite_history_upsert_restart_clear_and_scope_isolation() {
    let storage = HostStorage::open_sqlite_url("file:recent-history-test?mode=memory&cache=shared")
        .await
        .unwrap();
    for sql in IM_SCHEMA {
        storage.execute_raw(sql).await.unwrap();
    }
    for sql in IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error}"
            );
        }
    }
    let mut im = module("actor");
    let target = json!([{"type":"channel","id":"first"}]);
    assert_eq!(
        command(&storage, &mut im, "import", target.clone()).await["items"],
        target
    );
    let next = json!([{"type":"channel","id":"next"}]);
    assert_eq!(
        command(&storage, &mut im, "record", next).await["items"][0]["id"],
        "next"
    );
    let mut restarted = module("actor");
    assert_eq!(
        command(&storage, &mut restarted, "read", json!([])).await["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        command(&storage, &mut module("other"), "read", json!([])).await["items"],
        json!([])
    );
    command(&storage, &mut restarted, "clear", json!([])).await;
    assert_eq!(
        command(&storage, &mut restarted, "import", target).await["items"],
        json!([])
    );
}
