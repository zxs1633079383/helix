//! 频道群主转让的真 SQLite 持久化回归。

use helix_core::effect::{GetSpec, SqlValue};
use helix_core::ports::Storage;
use helix_core::tick::PortOutcome;
use helix_driver_host::{execute_storage_ops_atomic, HostStorage};
use helix_im::acl::to_effect::upsert_channel_full_op;
use helix_im::channel_write::program_full;

/// 使用生产 IM schema 构造隔离的内存 SQLite。
async fn storage_with_im_schema() -> HostStorage {
    let db = format!(
        "file:helix_channel_owner_persist_{}?mode=memory&cache=shared",
        std::process::id()
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open owner persistence storage");
    for &sql in helix_im::IM_SCHEMA {
        storage.execute_raw(sql).await.expect("apply IM schema");
    }
    storage
}

/// 从真 SQLite 行读取文本列。
fn col_text(row: &helix_core::effect::Row, name: &str) -> String {
    match row
        .iter()
        .find(|(column, _)| column == name)
        .map(|(_, value)| value)
    {
        Some(SqlValue::Text(value)) => value.clone(),
        other => panic!("column {name} expected Text, got {other:?}"),
    }
}

/// 明确 owner 覆盖旧群主，后续稀疏帧继续保留新群主。
#[tokio::test]
async fn channel_owner_transfer_updates_then_sparse_frame_preserves_owner() {
    let storage = storage_with_im_schema().await;
    let channel_id = "chfixx00000000000000000078".to_string();
    let owner_a = serde_json::json!({"id": "owner-a", "teamId": "team-1"});
    let owner_b = serde_json::json!({"id": "owner-b", "teamId": "team-1"});

    for data in [
        serde_json::json!({"id": channel_id, "owner": owner_a}),
        serde_json::json!({"id": channel_id, "owner": owner_b}),
        serde_json::json!({"id": channel_id, "displayName": "稀疏更新"}),
    ] {
        let (row, omit) = program_full(&data, "viewer-1", 1_700_000_000_000)
            .expect("channel fixture must produce a full row");
        let outcome =
            execute_storage_ops_atomic(&storage, &[upsert_channel_full_op(row, omit)]).await;
        assert!(matches!(outcome, PortOutcome::Ok(_)));
    }

    let row = storage
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(channel_id),
        })
        .await
        .expect("read channel after transfer")
        .expect("channel row must exist");
    assert_eq!(col_text(&row, "owner"), owner_b.to_string());
}
