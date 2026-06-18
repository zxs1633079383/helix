//! NativeStorage — PC/Tauri storage shell over the shared host storage.
//!
//! SQLite schema, migration, parameter binding, row conversion, and StorageOp
//! semantics live in `helix-driver-host`. This crate keeps the native public
//! type name so Tauri-side code and existing tests do not learn the shared
//! implementation details.

use helix_core::effect::{
    BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec, Row as HelixRow, ScanSpec,
    UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use helix_driver_host::HostStorage;

#[derive(Clone)]
pub struct NativeStorage {
    inner: HostStorage,
}

impl NativeStorage {
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
impl Storage for NativeStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        self.inner.batch_upsert(spec).await
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        self.inner.batch_update(spec).await
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        self.inner.monotonic_upsert(spec).await
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        self.inner.guarded_bump(spec).await
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        self.inner.get(spec).await
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        self.inner.scan(spec).await
    }
}
