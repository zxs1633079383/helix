use helix_core::effect::{ScanSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_host::{BackendScope, HostStorage, LocalStoreIdentity, LocalStoreLocator};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

fn temp_base() -> PathBuf {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "helix_multitenant_local_store_{}_{}",
        std::process::id(),
        seq
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn identity(account_id: &str) -> LocalStoreIdentity {
    LocalStoreIdentity {
        app_namespace: "com.jinqidongli.cses.dev".to_string(),
        backend: BackendScope {
            environment: "dev".to_string(),
            api_base_url: "http://127.0.0.1:8066/api/cses".to_string(),
            ws_url: "ws://127.0.0.1:8066/api/cses/websocket".to_string(),
        },
        company_id: "64118".to_string(),
        account_id: account_id.to_string(),
        tenant_hint: Some(account_id.to_string()),
        schema_family: "helix-im-v1".to_string(),
    }
}

async fn open_im_store(db_url: &str) -> HostStorage {
    let storage = HostStorage::open_sqlite_url(db_url)
        .await
        .expect("open file-backed SQLite store");
    for &sql in helix_im::IM_SCHEMA {
        storage
            .execute_raw(sql)
            .await
            .unwrap_or_else(|e| panic!("execute IM_SCHEMA failed for {sql:?}: {e:?}"));
    }
    storage
}

fn row_for_message(account_id: &str) -> Vec<(String, SqlValue)> {
    vec![
        (
            "temporary_id".to_string(),
            SqlValue::Text(format!("tmp-local-store-{account_id}")),
        ),
        (
            "id".to_string(),
            SqlValue::Text(format!("post-local-store-{account_id}")),
        ),
        (
            "channel_id".to_string(),
            SqlValue::Text(format!("channel-local-store-{account_id}")),
        ),
        (
            "user_id".to_string(),
            SqlValue::Text(account_id.to_string()),
        ),
        ("type".to_string(), SqlValue::Text("P".to_string())),
        (
            "message".to_string(),
            SqlValue::Text(format!("hello from {account_id}")),
        ),
        (
            "send_status".to_string(),
            SqlValue::Text("sent".to_string()),
        ),
        (
            "create_at".to_string(),
            SqlValue::Integer(1_783_320_000_000),
        ),
        (
            "update_at".to_string(),
            SqlValue::Integer(1_783_320_000_000),
        ),
    ]
}

fn row_for_member(account_id: &str) -> Vec<(String, SqlValue)> {
    vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(format!("channel-local-store-{account_id}")),
        ),
        (
            "user_id".to_string(),
            SqlValue::Text(account_id.to_string()),
        ),
        ("team_id".to_string(), SqlValue::Text("64118".to_string())),
        ("role".to_string(), SqlValue::Text("MEMBER".to_string())),
        (
            "joined_at".to_string(),
            SqlValue::Integer(1_783_320_000_000),
        ),
        (
            "updated_at".to_string(),
            SqlValue::Integer(1_783_320_000_000),
        ),
    ]
}

fn row_for_cursor(account_id: &str, seq: i64) -> Vec<(String, SqlValue)> {
    vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(format!("channel-local-store-{account_id}")),
        ),
        ("last_event_seq".to_string(), SqlValue::Integer(seq)),
        (
            "updated_at".to_string(),
            SqlValue::Integer(1_783_320_000_000),
        ),
    ]
}

async fn seed_account(storage: &HostStorage, account_id: &str, seq: i64) {
    storage
        .batch_upsert(UpsertSpec::new(
            "message",
            vec![row_for_message(account_id)],
            Some("temporary_id"),
        ))
        .await
        .expect("insert message row");
    storage
        .batch_upsert(UpsertSpec::new(
            "channel_member",
            vec![row_for_member(account_id)],
            None,
        ))
        .await
        .expect("insert channel_member row");
    storage
        .batch_upsert(UpsertSpec::new(
            "channel_event_cursor",
            vec![row_for_cursor(account_id, seq)],
            Some("channel_id"),
        ))
        .await
        .expect("insert channel_event_cursor row");
}

async fn scan_table(storage: &HostStorage, table: &'static str) -> Vec<helix_core::effect::Row> {
    storage
        .scan(ScanSpec {
            table,
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .unwrap_or_else(|e| panic!("scan {table}: {e:?}"))
}

fn col_text(row: &helix_core::effect::Row, name: &str) -> String {
    match row.iter().find(|(k, _)| k == name).map(|(_, value)| value) {
        Some(SqlValue::Text(value)) => value.clone(),
        other => panic!("{name} expected Text, got {other:?}"),
    }
}

fn col_i64(row: &helix_core::effect::Row, name: &str) -> i64 {
    match row.iter().find(|(k, _)| k == name).map(|(_, value)| value) {
        Some(SqlValue::Integer(value)) => *value,
        other => panic!("{name} expected Integer, got {other:?}"),
    }
}

fn manifest_account_id(path: &std::path::Path) -> String {
    let raw = std::fs::read_to_string(path).expect("read manifest");
    let json: serde_json::Value = serde_json::from_str(&raw).expect("manifest json");
    json["accountId"]
        .as_str()
        .expect("manifest accountId")
        .to_string()
}

#[tokio::test]
async fn real_file_sqlite_isolates_users_444_and_678() {
    let base = temp_base();
    let locator = LocalStoreLocator::new(&base);
    let paths_444 = locator.resolve(&identity("444")).expect("resolve 444");
    let paths_678 = locator.resolve(&identity("678")).expect("resolve 678");

    assert_ne!(paths_444.key, paths_678.key);
    assert_ne!(paths_444.db_path, paths_678.db_path);
    assert_eq!(paths_444.db_path.file_name().unwrap(), "tenant.db");
    assert_eq!(paths_678.db_path.file_name().unwrap(), "tenant.db");
    assert_eq!(manifest_account_id(&paths_444.manifest_path), "444");
    assert_eq!(manifest_account_id(&paths_678.manifest_path), "678");

    let store_444 = open_im_store(&paths_444.db_url).await;
    let store_678 = open_im_store(&paths_678.db_url).await;

    seed_account(&store_444, "444", 44).await;
    assert!(
        paths_444.db_path.exists(),
        "444 tenant.db must be a real file"
    );

    let empty_678_messages = scan_table(&store_678, "message").await;
    let empty_678_members = scan_table(&store_678, "channel_member").await;
    let empty_678_cursors = scan_table(&store_678, "channel_event_cursor").await;
    assert!(
        empty_678_messages.is_empty(),
        "678 must not see 444 messages"
    );
    assert!(empty_678_members.is_empty(), "678 must not see 444 members");
    assert!(empty_678_cursors.is_empty(), "678 must not see 444 cursors");

    seed_account(&store_678, "678", 67).await;
    assert!(
        paths_678.db_path.exists(),
        "678 tenant.db must be a real file"
    );

    let reopened_444 = open_im_store(&paths_444.db_url).await;
    let messages_444 = scan_table(&reopened_444, "message").await;
    let members_444 = scan_table(&reopened_444, "channel_member").await;
    let cursors_444 = scan_table(&reopened_444, "channel_event_cursor").await;
    assert_eq!(messages_444.len(), 1);
    assert_eq!(members_444.len(), 1);
    assert_eq!(cursors_444.len(), 1);
    assert_eq!(col_text(&messages_444[0], "user_id"), "444");
    assert_eq!(col_text(&members_444[0], "user_id"), "444");
    assert_eq!(col_i64(&cursors_444[0], "last_event_seq"), 44);

    let reopened_678 = open_im_store(&paths_678.db_url).await;
    let messages_678 = scan_table(&reopened_678, "message").await;
    let members_678 = scan_table(&reopened_678, "channel_member").await;
    let cursors_678 = scan_table(&reopened_678, "channel_event_cursor").await;
    assert_eq!(messages_678.len(), 1);
    assert_eq!(members_678.len(), 1);
    assert_eq!(cursors_678.len(), 1);
    assert_eq!(col_text(&messages_678[0], "user_id"), "678");
    assert_eq!(col_text(&members_678[0], "user_id"), "678");
    assert_eq!(col_i64(&cursors_678[0], "last_event_seq"), 67);
}
