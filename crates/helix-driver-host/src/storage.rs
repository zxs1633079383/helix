//! Shared rusqlite storage implementation for PC Tauri and Flutter FFI.
//!
//! ## 读写分离（B1 回归修复）
//!
//! 旧实现（commit a0595607 重构后）把 native 三池塌成单个 `Arc<Mutex<Connection>>`，
//! 导致 get/scan 与任何 op 串行在同一把锁。本实现恢复读写分离：
//!
//! - 1 个写连接：batch_upsert / batch_update / monotonic_upsert 串行兑现。
//! - N=4 个只读连接：get / scan 在 WAL 下与写者及其他读者并发。
//! - 不支持共享连接的内存数据库回退到写连接读取，语义优先。
//!
//! ## 不变量（不可破）
//!
//! - batch_upsert 单事务（E2 写放大 1x）
//! - monotonic MAX guard（E5）与 GuardedBump 单条 UPDATE（HX-C005）
//! - `_helix_monotonic` 表在 open()/migrate() 建（不在热路径）
//! - PRAGMA journal_mode=WAL + synchronous=NORMAL + busy_timeout=5000
//! - SQL trace 的 statement、parameters、table 与错误映射保持一致

mod connection;
mod read;
mod trace;
mod values;
mod write;

use std::sync::{Arc, Mutex};

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec,
    Row as HelixRow, ScanSpec, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use rusqlite::Connection;

use connection::{
    map_join_err, map_lock_err, map_sqlite_err, open_reader, open_writer, sqlite_target_from_url,
    target_supports_shared_readers, ReadPool, READ_POOL_SIZE,
};
use trace::{current_storage_trace_context, trace_sql};

pub use trace::{with_storage_trace_context, StorageTraceContext};

/// 内存数据库若未启用 shared cache，第二个连接会得到独立空库；文件 WAL 库与显式
/// `cache=shared` 的内存库才启用只读池。
#[derive(Clone)]
pub struct HostStorage {
    writer: Arc<Mutex<Connection>>,
    readers: Option<Arc<ReadPool>>,
}

impl HostStorage {
    pub async fn open_sqlite_url(db_url: &str) -> Result<Self, PortError> {
        let target = sqlite_target_from_url(db_url);
        let shareable = target_supports_shared_readers(&target);

        let target_for_blocking = target.clone();
        let (writer, readers) = tokio::task::spawn_blocking(move || {
            let writer = open_writer(&target_for_blocking)?;

            let readers = if shareable {
                let mut conns = Vec::with_capacity(READ_POOL_SIZE);
                for _ in 0..READ_POOL_SIZE {
                    conns.push(open_reader(&target_for_blocking)?);
                }
                Some(Arc::new(ReadPool::new(conns)))
            } else {
                None
            };

            Ok::<_, PortError>((writer, readers))
        })
        .await
        .map_err(map_join_err)??;

        let storage = Self {
            writer: Arc::new(Mutex::new(writer)),
            readers,
        };
        storage.migrate().await?;
        Ok(storage)
    }

    pub async fn execute_raw(&self, sql: &'static str) -> Result<(), PortError> {
        let writer = Arc::clone(&self.writer);
        let trace = current_storage_trace_context();
        tokio::task::spawn_blocking(move || {
            let conn = writer.lock().map_err(map_lock_err)?;
            let _span = trace_sql(&trace, "EXECUTE", None, sql, &[]);
            conn.execute_batch(sql).map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn migrate(&self) -> Result<(), PortError> {
        self.execute_raw(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS _helix_monotonic
             (scope_key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL DEFAULT 0);",
        )
        .await
    }

    /// 在只读连接上跑闭包；无可共享只读池时回退到写连接。
    pub(super) async fn with_reader<T, F>(&self, f: F) -> Result<T, PortError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, PortError> + Send + 'static,
    {
        match &self.readers {
            Some(pool) => {
                let permit = Arc::clone(&pool.permits)
                    .acquire_owned()
                    .await
                    .map_err(|_| PortError::Storage("read pool semaphore closed".to_string()))?;
                let pool = Arc::clone(pool);
                tokio::task::spawn_blocking(move || {
                    let conn = {
                        let mut idle = pool.idle.lock().map_err(map_lock_err)?;
                        idle.pop()
                    };
                    let Some(conn) = conn else {
                        return Err(PortError::Storage(
                            "read pool permit/conn mismatch".to_string(),
                        ));
                    };
                    let out = f(&conn);
                    if let Ok(mut idle) = pool.idle.lock() {
                        idle.push(conn);
                    }
                    drop(permit);
                    out
                })
                .await
                .map_err(map_join_err)?
            }
            None => {
                let writer = Arc::clone(&self.writer);
                tokio::task::spawn_blocking(move || {
                    let conn = writer.lock().map_err(map_lock_err)?;
                    f(&conn)
                })
                .await
                .map_err(map_join_err)?
            }
        }
    }
}

#[async_trait::async_trait]
impl Storage for HostStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        write::batch_upsert(self, spec).await
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        write::batch_update(self, spec).await
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        write::monotonic_upsert(self, spec).await
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        write::guarded_bump(self, spec).await
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        read::get(self, spec).await
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        read::scan(self, spec).await
    }

    async fn batch_delete(&self, spec: BatchDeleteSpec) -> Result<(), PortError> {
        write::batch_delete(self, spec).await
    }
}
