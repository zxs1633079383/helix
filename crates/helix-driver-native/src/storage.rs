//! NativeStorage — sqlx::SqlitePool (WAL)
//!
//! ## WAL 配置
//!
//! - 写池 max=1（单写者，避免 SQLite write lock 竞争）
//! - 读池 max=4（并发读）
//! - 热路径 O(1)：batch_upsert ON CONFLICT + monotonic_upsert MAX guard

use helix_core::PortError;
use helix_core::ports::Storage;
use helix_core::effect::{UpsertSpec, MonotonicUpsertSpec, GetSpec, ScanSpec, Row, SqlValue};

pub struct NativeStorage {
    // TODO: write_pool: sqlx::SqlitePool,  // max=1
    // TODO: read_pool: sqlx::SqlitePool,   // max=4
}

impl NativeStorage {
    pub fn new() -> Self {
        Self {}
    }

    /// sqlx 错误 → PortError（ACL-2 翻译）
    fn map_err(e: impl std::fmt::Display) -> PortError {
        PortError::Storage(e.to_string())
    }
}

#[async_trait::async_trait]
impl Storage for NativeStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        // TODO: 构造 INSERT INTO {spec.table} (...) VALUES (...)
        //       ON CONFLICT ({spec.conflict_key}) DO UPDATE SET ...
        //       使用 sqlx query!宏（编译期检查）
        todo!("NativeStorage::batch_upsert")
    }

    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[SqlValue],
        patch: Row,
    ) -> Result<(), PortError> {
        // TODO: UPDATE {table} SET {patch} WHERE {key_col} IN (...)
        todo!("NativeStorage::batch_update")
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        // TODO:
        // INSERT INTO <table> (scope_key, value) VALUES (?, ?)
        // ON CONFLICT(scope_key) DO UPDATE SET value = excluded.value
        //   WHERE excluded.value > <table>.value
        // O(1) per call（MAX guard 保证单调推进）
        todo!("NativeStorage::monotonic_upsert")
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError> {
        // TODO: SELECT * FROM {spec.table} WHERE {spec.key_col} = ?
        todo!("NativeStorage::get")
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError> {
        // TODO: SELECT * FROM {spec.table} LIMIT {spec.limit | 硬上限}（走读池）
        let _ = spec;
        todo!("NativeStorage::scan")
    }
}
