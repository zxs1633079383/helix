//! Markdown 在真实 SQLite 的发送写入与重开读回证据；不冒充 Go/WS 真实发送。
use helix_core::effect::{Effect, GetSpec, SqlValue};
use helix_core::module_host::Module;
use helix_core::ports::Storage;
use helix_core::tick::AppCommand;
use helix_core::{EffectSink, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use helix_im::state::ChannelId;
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde_json::json;

/// 生产 Command 生成 StorageOp，Host 执行后才允许生成 HTTP；文件重开仍逐字一致。
#[tokio::test]
async fn markdown_command_survives_sqlite_reopen() {
    verify_markdown_sqlite("text", "TEXT", "# 标题\n\n**重点** 和 [链接](https://example.com)\n\n```html\n  <button>按钮 &amp;</button>\n```\n", json!({}), "标题 重点 和 链接 <button>按钮 &amp;</button>").await;
}

/// 图文布局、提及和稳定附件索引经过生产发送链落库，关闭数据库后仍可恢复。
#[tokio::test]
async fn rich_markdown_command_survives_sqlite_reopen() {
    verify_markdown_sqlite("rich", "RICH", "@张三 **前文** 后文", json!({"imagePosition":"bottom", "files":[{"bucket":"image", "id":"image-1", "uri":"/media/image-1.png", "contentType":"image/png"}], "richText":{"version":1,"markdown":"@张三 **前文** ![图片](cses-media:0) 后文"}}), "[媒体]@张三 前文 后文").await;
}

/// 共用真实 SQLite 屏障与重开验证，各测试使用独占数据库文件。
async fn verify_markdown_sqlite(
    case: &str,
    msg_type: &str,
    source: &str,
    props: serde_json::Value,
    summary: &str,
) {
    let path =
        std::env::temp_dir().join(format!("helix-markdown-{case}-{}.db", std::process::id()));
    assert!(!path.exists(), "测试库必须独占");
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let storage = HostStorage::open_sqlite_url(&url).await.unwrap();
    for &sql in IM_SCHEMA {
        storage.execute_raw(sql).await.unwrap();
    }
    for &sql in IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error}"
            );
        }
    }
    let channel = "chfixx00000000000000000001";
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "markdown-user".into(),
        company_id: "markdown-team".into(),
        api_base_url: "http://127.0.0.1:1".into(),
        ..Default::default()
    });
    module.register_channel(ChannelId::from_str(channel).unwrap(), 0);
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_send_message",
                serde_json::to_vec(&json!({
                    "channel_id": channel, "allocated_temporary_id": "markdown-temp", "text": source, "type": msg_type, "props": props, "mentions": ["user-zhang"]
                }))
                .unwrap(),
            )),
            1000,
            &mut sink,
        )
        .unwrap();
    assert!(!sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));
    let initial = std::mem::take(&mut sink);
    let mut persisted = false;
    for effect in initial.as_slice() {
        let (corr, outcome) = match effect {
            Effect::Persist { corr, ops } => (*corr, execute_storage_ops(&storage, &ops).await),
            Effect::PersistAtomic { corr, ops } => {
                (*corr, execute_storage_ops_atomic(&storage, &ops).await)
            }
            _ => continue,
        };
        assert!(matches!(outcome, helix_core::tick::PortOutcome::Ok(_)));
        persisted = true;
        module
            .handle(&Tick::PortReply { corr, outcome }, 1001, &mut sink)
            .unwrap();
    }
    assert!(persisted);
    let body = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some(
                serde_json::from_slice::<serde_json::Value>(req.body.as_ref().unwrap()).unwrap(),
            ),
            _ => None,
        })
        .expect("持久化后才产生 HTTP");
    assert_eq!(body["message"], source);
    assert_eq!(body["simpleMessage"], summary);
    if msg_type == "RICH" {
        assert_eq!(body["props"]["richText"], props["richText"]);
    }
    drop(module);
    drop(storage);
    let reopened = HostStorage::open_sqlite_url(&url).await.unwrap();
    let row = reopened
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("markdown-temp".into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(row.iter().find(|(key, _)| key == "message").map(|(_, value)| value), Some(SqlValue::Text(text)) if text == source)
    );
    if msg_type == "RICH" {
        let saved = row
            .iter()
            .find(|(key, _)| key == "props")
            .map(|(_, value)| value)
            .unwrap();
        if let SqlValue::Text(saved) = saved {
            let saved: serde_json::Value = serde_json::from_str(saved).unwrap();
            assert_eq!(saved["richText"], props["richText"]);
            assert_eq!(saved["files"][0]["id"], "image-1");
        } else {
            panic!("props must be persisted JSON text");
        }
    }
    drop(reopened);
    std::fs::remove_file(&path).unwrap();
    for suffix in ["db-wal", "db-shm"] {
        let sidecar = path.with_extension(suffix);
        if sidecar.exists() {
            std::fs::remove_file(sidecar).unwrap();
        }
    }
    assert!(!path.exists());
}
