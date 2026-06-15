//! FfiStorage — Flutter FFI storage shell over the shared host storage.
//!
//! Flutter/Dart does not receive StorageOp, SQL, cursor, or message schema
//! responsibilities. Rust owns SQLite I/O through `helix-driver-host`.

use helix_core::effect::{
    GetSpec, MonotonicUpsertSpec, Row as HelixRow, ScanSpec, SqlValue, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use helix_driver_host::HostStorage;

#[derive(Clone)]
pub struct FfiStorage {
    inner: HostStorage,
}

impl FfiStorage {
    pub async fn open(db_url: &str) -> Result<Self, PortError> {
        Ok(Self {
            inner: HostStorage::open_sqlite_url(db_url).await?,
        })
    }

    pub async fn execute_raw(&self, sql: &'static str) -> Result<(), PortError> {
        self.inner.execute_raw(sql).await
    }
}

#[async_trait::async_trait]
impl Storage for FfiStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        self.inner.batch_upsert(spec).await
    }

    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[SqlValue],
        patch: HelixRow,
    ) -> Result<(), PortError> {
        self.inner
            .batch_update(table, key_col, key_vals, patch)
            .await
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        self.inner.monotonic_upsert(spec).await
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        self.inner.get(spec).await
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        self.inner.scan(spec).await
    }
}
