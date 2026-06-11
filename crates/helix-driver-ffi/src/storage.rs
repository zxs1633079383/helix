//! FfiStorage — host SQLite handle via C ABI
//!
//! ## MAJ-4: tokio worker thread 阻塞防范
//!
//! 此 driver 的每个 `async fn` 实现中，同步 SQLite C ABI 调用必须包入
//! `tokio::task::spawn_blocking(|| { /* sqlite call */ }).await`，
//! 否则会在 tokio worker thread 上做阻塞 C 调用，饿死 tokio reactor。
//!
//! 若使用单线程 executor（`tokio::runtime::Builder::new_current_thread()`），
//! 则 blocking 问题不存在；但必须在文档中明确声明 executor 约束。
//! 此骨架选择更安全的 `spawn_blocking` 方案（多线程 executor 兼容）。
//!
//! ## 使用说明
//!
//! FfiStorage 设计为由宿主（Swift/Kotlin/C 调用方）传入 SQLite 句柄，
//! 通过 C ABI 回调执行 SQL 操作，不依赖 Rust SQLite crate。

use helix_core::PortError;
use helix_core::effect::{UpsertSpec, MonotonicUpsertSpec, GetSpec, ScanSpec, Row, SqlValue};

pub struct FfiStorage {
    // TODO: sqlite_handle: *mut std::ffi::c_void,  // host 传入的 SQLite 句柄
    // MAJ-4: handle 必须实现 Send（跨线程传入 spawn_blocking）
    // 建议用 Arc<Mutex<*mut c_void>> 包裹 raw pointer 保证线程安全
}

#[async_trait::async_trait]
impl helix_core::ports::Storage for FfiStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        // MAJ-4: 实际实现应包入 tokio::task::spawn_blocking：
        // tokio::task::spawn_blocking(move || {
        //     // 调用 host C ABI: ffi_sqlite_batch_upsert(handle, &spec)
        // }).await.map_err(|e| PortError::Storage(e.to_string()))?
        todo!("FfiStorage::batch_upsert: wrap C call in spawn_blocking")
    }

    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[SqlValue],
        patch: Row,
    ) -> Result<(), PortError> {
        // MAJ-4: 同上，C ABI 调用需要 spawn_blocking
        todo!("FfiStorage::batch_update: wrap C call in spawn_blocking")
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        // MAJ-4: 同上
        todo!("FfiStorage::monotonic_upsert: wrap C call in spawn_blocking")
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError> {
        // MAJ-4: 同上
        todo!("FfiStorage::get: wrap C call in spawn_blocking")
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError> {
        // MAJ-4: 同上（同步 C 调用必须包 spawn_blocking）
        let _ = spec;
        todo!("FfiStorage::scan: wrap C call in spawn_blocking")
    }
}
