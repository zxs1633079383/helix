use helix_core::effect::{BatchDeleteSpec, ScanSpec, SqlValue, StorageOp, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_native::NativeStorage;

async fn storage() -> (NativeStorage, String) {
    let path =
        std::env::temp_dir().join(format!("helix-g03-pending-media-{}.db", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    let storage = NativeStorage::open(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open pending_media test database");
    for &sql in helix_im::IM_SCHEMA {
        storage.execute_raw(sql).await.expect("apply IM schema");
    }
    (storage, path)
}

#[tokio::test]
async fn pending_media_composite_identity_updates_and_scoped_delete_removes_one_target() {
    let (storage, path) = storage().await;
    let row = |target: &str, stage: &str| {
        vec![
            (
                "temporary_id".to_string(),
                SqlValue::Text("tmp-1".to_string()),
            ),
            ("target_key".to_string(), SqlValue::Text(target.to_string())),
            ("stage".to_string(), SqlValue::Text(stage.to_string())),
            (
                "channel_id".to_string(),
                SqlValue::Text("chfixx00000000000000000001".to_string()),
            ),
            (
                "local_path".to_string(),
                SqlValue::Text(format!("/tmp/{target}")),
            ),
            ("file_name".to_string(), SqlValue::Text(target.to_string())),
            (
                "content_type".to_string(),
                SqlValue::Text("image/png".to_string()),
            ),
            ("size".to_string(), SqlValue::Integer(3)),
            ("sha256".to_string(), SqlValue::Text("sha".to_string())),
        ]
    };
    storage
        .batch_upsert(UpsertSpec::new(
            "pending_media",
            vec![row("rich:0", "prepare"), row("rich:1", "complete")],
            None,
        ))
        .await
        .expect("insert pending media rows");
    storage
        .batch_upsert(UpsertSpec::new(
            "pending_media",
            vec![row("rich:0", "put")],
            None,
        ))
        .await
        .expect("replace one composite-key row");

    storage
        .batch_delete(BatchDeleteSpec {
            table: "pending_media",
            scope_col: "temporary_id",
            scope_val: SqlValue::Text("tmp-1".to_string()),
            key_col: "target_key",
            key_vals: vec![SqlValue::Text("rich:0".to_string())],
        })
        .await
        .expect("delete one attachment");
    let rows = storage
        .scan(ScanSpec {
            table: "pending_media",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan pending media");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].iter().any(|(column, value)| {
        column == "target_key" && matches!(value, SqlValue::Text(target) if target == "rich:1")
    }));

    drop(storage);
    let _ = std::fs::remove_file(path);
}

#[test]
fn storage_op_shape_keeps_pending_media_internal() {
    let operation = StorageOp::BatchDelete(BatchDeleteSpec {
        table: "pending_media",
        scope_col: "temporary_id",
        scope_val: SqlValue::Text("tmp-1".to_string()),
        key_col: "target_key",
        key_vals: vec![SqlValue::Text("file".to_string())],
    });
    assert!(matches!(operation, StorageOp::BatchDelete(spec)
        if spec.table == "pending_media" && spec.scope_col == "temporary_id"));
}
