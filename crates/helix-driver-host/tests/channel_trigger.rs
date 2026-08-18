//! Helix IM channel schema 的真实 SQLite 触发器回归测试。

use helix_core::effect::{ScanSpec, SqlValue};
use helix_core::ports::Storage;
use helix_driver_host::HostStorage;
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

/// 使用与 host/FFI/native 启动相同的 schema + additive migration 链路创建隔离数据库。
async fn storage_with_im_schema() -> HostStorage {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_im_channel_trigger_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open storage");
    for &sql in helix_im::IM_SCHEMA {
        storage
            .execute_raw(sql)
            .await
            .unwrap_or_else(|error| panic!("execute IM_SCHEMA failed for {sql:?}: {error:?}"));
    }
    for &sql in helix_im::IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "execute IM_SCHEMA_MIGRATIONS failed for {sql:?}: {error:?}"
            );
        }
    }
    storage
}

/// 读取 channel 行，验证真实 Storage 读连接能看到写连接触发器的结果。
async fn scan_channel_rows(storage: &HostStorage) -> Vec<helix_core::effect::Row> {
    storage
        .scan(ScanSpec {
            table: "channel",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan channel table")
}

/// 从一行中取指定列，避免测试把列顺序误当成 schema 合同。
fn column<'a>(row: &'a [(String, SqlValue)], name: &str) -> Option<&'a SqlValue> {
    row.iter()
        .find(|(column, _)| column == name)
        .map(|(_, value)| value)
}

/// 回归：省略 updated_at 时自动写入毫秒时间，显式值仍由调用方控制。
#[tokio::test]
async fn channel_update_at_trigger_is_installed_and_preserves_explicit_values() {
    let storage = storage_with_im_schema().await;
    storage
        .execute_raw(
            "INSERT INTO channel (id, display_name, updated_at)\n\
VALUES ('trigger-channel', 'before', 1)",
        )
        .await
        .expect("insert channel fixture");

    storage
        .execute_raw(
            "UPDATE channel SET display_name = 'after'\n\
WHERE id = 'trigger-channel'",
        )
        .await
        .expect("update channel without timestamp");
    let rows = scan_channel_rows(&storage).await;
    let first_updated_at = match column(&rows[0], "updated_at") {
        Some(SqlValue::Integer(value)) => {
            assert!(*value > 1);
            *value
        }
        other => panic!("updated_at expected Integer after trigger, got {other:?}"),
    };

    storage
        .execute_raw(
            "UPDATE channel SET display_name = 'after-again'\n\
WHERE id = 'trigger-channel'",
        )
        .await
        .expect("update channel twice without timestamp");
    let rows = scan_channel_rows(&storage).await;
    match column(&rows[0], "updated_at") {
        Some(SqlValue::Integer(value)) => assert!(*value > first_updated_at),
        other => panic!("updated_at expected Integer after second trigger, got {other:?}"),
    }

    storage
        .execute_raw(
            "UPDATE channel SET updated_at = 42\n\
WHERE id = 'trigger-channel'",
        )
        .await
        .expect("update channel with explicit timestamp");
    let rows = scan_channel_rows(&storage).await;
    match column(&rows[0], "updated_at") {
        Some(SqlValue::Integer(value)) => assert_eq!(*value, 42),
        other => panic!("updated_at expected explicit Integer(42), got {other:?}"),
    }
}
