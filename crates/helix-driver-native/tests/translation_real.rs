//! Native translation integration: public Tick → ExecutionShell → isolated NativeHttp → EventSink.
//!
//! The test uses the currently running local Docker translator.  It intentionally does not call a
//! private handler or a direct HTTP helper, and it does not involve Go, WS, or account state because
//! translation is a non-durable remote read.  A run-owned SQLite database is still opened and checked
//! before/after the engine run to prove that the message table is untouched.
//!
//! Required environment (the API key is read into process memory only):
//! `CSES_TRANSLATION_URL` (or `TRANSLATION_URL`) and `CSES_TRANSLATION_API_KEY`
//! (or `TRANSLATION_API_KEY`).

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Row, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::AppCommand;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::{HostNetworkConfig, NoopMetricSink};
use helix_driver_native::{
    run_engine_loop, EngineConfig, NativeEventSink, NativeHttp, NativeStorage, SharedFileUploader,
    TranslationHttpConfig,
};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::{timeout, Duration};

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const USER_ID: &str = "translation-native-user";
const COMPANY_ID: &str = "translation-native-company";
const TRANSLATION_EVENT: &str = "im:translation:result";
static DB_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[tokio::test]
async fn real_native_translation_both_directions_keep_message_table_unchanged() {
    let endpoint = std::env::var("CSES_TRANSLATION_URL")
        .or_else(|_| std::env::var("TRANSLATION_URL"))
        .expect("BLOCKED_PRECONDITION: CSES_TRANSLATION_URL is not configured");
    let api_key = std::env::var("CSES_TRANSLATION_API_KEY")
        .or_else(|_| std::env::var("TRANSLATION_API_KEY"))
        .expect("BLOCKED_PRECONDITION: translation API key is not configured");

    let (storage, db_path) = open_test_storage().await;
    seed_channel(&storage).await;
    let before = message_fingerprint(&scan_messages(&storage).await);

    let http = NativeHttp::with_config(HostNetworkConfig::new(
        "http://127.0.0.1:1",
        "ws://127.0.0.1:1",
    ))
    .expect("native main HTTP client should build")
    .with_translation(TranslationHttpConfig::new(
        endpoint,
        Duration::from_secs(30),
        Some(("Authorization".to_string(), format!("Bearer {api_key}"))),
    ))
    .expect("translation HTTP client should build");

    let (event_sink, mut events) = NativeEventSink::new();
    let mut im_config = ImConfig::default();
    im_config.api_base_url = "http://127.0.0.1:1".to_string();
    im_config.ws_url = "ws://127.0.0.1:1".to_string();
    im_config.auth_user_id = USER_ID.to_string();
    im_config.company_id = COMPANY_ID.to_string();

    let mut module = ImModule::new(im_config);
    module.register_channel(
        helix_im::state::ChannelId::from_str(CHANNEL_ID).expect("canonical channel id"),
        0,
    );
    let mut shell = ExecutionShell::new();
    shell.register(module);

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let config = EngineConfig::new(
        storage.clone(),
        http,
        SharedFileUploader::default(),
        event_sink,
        8,
    )
    .with_metric_sink(std::sync::Arc::new(NoopMetricSink));
    let engine = tokio::spawn(run_engine_loop(
        shell,
        tick_rx,
        tick_tx.clone(),
        config,
        shutdown_rx,
    ));

    send_translation(&tick_tx, "native-zh-en", "zh", "en", "你好").await;
    let first = wait_translation_event(&mut events, "native-zh-en").await;
    assert_eq!(
        first["data"]["status"], "success",
        "first translation: {first}"
    );
    assert_eq!(first["data"]["from"], "zh");
    assert_eq!(first["data"]["to"], "en");
    assert!(first["data"]["translated_text"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));

    send_translation(&tick_tx, "native-en-zh", "en", "zh", "Hello").await;
    let second = wait_translation_event(&mut events, "native-en-zh").await;
    assert_eq!(second["data"]["status"], "success");
    assert_eq!(second["data"]["from"], "en");
    assert_eq!(second["data"]["to"], "zh");
    assert!(second["data"]["translated_text"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));

    shutdown_tx.send(()).expect("engine shutdown signal");
    timeout(Duration::from_secs(10), engine)
        .await
        .expect("engine graceful shutdown should finish")
        .expect("engine task should not panic");

    let after = message_fingerprint(&scan_messages(&storage).await);
    assert_eq!(before, after, "translation must not change message rows");
    drop(storage);
    remove_test_storage(&db_path);
}

async fn send_translation(
    tick_tx: &mpsc::Sender<Tick>,
    req_id: &str,
    from: &str,
    to: &str,
    text: &str,
) {
    let payload = json!({
        "req_id": req_id,
        "channel_id": CHANNEL_ID,
        "message_key": "srvfix00000000000000000001",
        "source_revision": format!("sha256:{req_id}"),
        "from": from,
        "to": to,
        "text": text,
    });
    tick_tx
        .send(Tick::Command(AppCommand::new(
            "im_translate_message",
            Bytes::from(serde_json::to_vec(&payload).expect("translation payload serializes")),
        )))
        .await
        .expect("translation Tick should enqueue");
}

async fn wait_translation_event(
    events: &mut broadcast::Receiver<DomainEventBytes>,
    req_id: &str,
) -> Value {
    timeout(Duration::from_secs(20), async {
        loop {
            let event = events
                .recv()
                .await
                .expect("NativeEventSink should stay open");
            let envelope: Value =
                serde_json::from_slice(event.0.as_ref()).expect("event envelope is JSON");
            if envelope["event"] == TRANSLATION_EVENT && envelope["data"]["req_id"] == req_id {
                return envelope;
            }
        }
    })
    .await
    .expect("translation result should arrive from the real service")
}

async fn open_test_storage() -> (NativeStorage, PathBuf) {
    let sequence = DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "helix-translation-native-{}-{sequence}-{nonce}.db",
        std::process::id()
    ));
    let storage = NativeStorage::open(&format!("sqlite:{}?mode=rwc", path.display()))
        .await
        .expect("open run-owned NativeStorage");
    for statement in IM_SCHEMA {
        storage
            .execute_raw(statement)
            .await
            .expect("apply IM schema");
    }
    for statement in IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(statement).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "unexpected IM migration error: {error}"
            );
        }
    }
    (storage, path)
}

async fn seed_channel(storage: &NativeStorage) {
    storage
        .batch_upsert(UpsertSpec::new(
            "channel",
            vec![vec![
                ("id".to_string(), SqlValue::Text(CHANNEL_ID.to_string())),
                ("user_id".to_string(), SqlValue::Text(USER_ID.to_string())),
                (
                    "team_id".to_string(),
                    SqlValue::Text(COMPANY_ID.to_string()),
                ),
            ]],
            Some("id"),
        ))
        .await
        .expect("seed run-owned channel scope");
}

async fn scan_messages(storage: &NativeStorage) -> Vec<Row> {
    storage
        .scan(helix_core::effect::ScanSpec {
            table: "message",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan run-owned message table")
}

fn message_fingerprint(rows: &[Row]) -> String {
    format!("{rows:?}")
}

fn remove_test_storage(path: &Path) {
    let _ = std::fs::remove_file(path);
}
