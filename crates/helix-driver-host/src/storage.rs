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
use std::time::Instant;

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec,
    Row as HelixRow, ScanSpec, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use rusqlite::Connection;

use crate::metrics::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

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
    metrics: Arc<dyn AsyncMetricSink>,
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
            metrics: Arc::new(NoopMetricSink),
        };
        storage.migrate().await?;
        Ok(storage)
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
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
        if !self.metrics.is_enabled() {
            return write::batch_upsert(self, spec).await;
        }
        let rows = spec.rows.len();
        let started = Instant::now();
        let result = write::batch_upsert(self, spec).await;
        self.record_storage_result("batch_upsert", rows, started, &result);
        result
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        if !self.metrics.is_enabled() {
            return write::batch_update(self, spec).await;
        }
        let rows = spec.key_vals.len();
        let started = Instant::now();
        let result = write::batch_update(self, spec).await;
        self.record_storage_result("batch_update", rows, started, &result);
        result
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        if !self.metrics.is_enabled() {
            return write::monotonic_upsert(self, spec).await;
        }
        let started = Instant::now();
        let result = write::monotonic_upsert(self, spec).await;
        self.record_storage_result("monotonic_upsert", 1, started, &result);
        result
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        if !self.metrics.is_enabled() {
            return write::guarded_bump(self, spec).await;
        }
        let started = Instant::now();
        let result = write::guarded_bump(self, spec).await;
        self.record_storage_result("guarded_bump", 1, started, &result);
        result
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        if !self.metrics.is_enabled() {
            return read::get(self, spec).await;
        }
        let started = Instant::now();
        let result = read::get(self, spec).await;
        let rows = result
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map_or(0, |_| 1);
        self.record_storage_result("get", rows, started, &result);
        result
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        if !self.metrics.is_enabled() {
            return read::scan(self, spec).await;
        }
        let started = Instant::now();
        let result = read::scan(self, spec).await;
        let rows = result.as_ref().map_or(0, Vec::len);
        self.record_storage_result("scan", rows, started, &result);
        result
    }

    async fn batch_delete(&self, spec: BatchDeleteSpec) -> Result<(), PortError> {
        if !self.metrics.is_enabled() {
            return write::batch_delete(self, spec).await;
        }
        let rows = spec.key_vals.len();
        let started = Instant::now();
        let result = write::batch_delete(self, spec).await;
        self.record_storage_result("batch_delete", rows, started, &result);
        result
    }
}

impl HostStorage {
    fn record_storage_result<T>(
        &self,
        operation: &'static str,
        rows: usize,
        started: Instant,
        result: &Result<T, PortError>,
    ) {
        let status = if result.is_ok() { "ok" } else { "error" };
        let labels = MetricLabels::one(LabelKey::Stage, "storage")
            .with(LabelKey::StorageOp, operation)
            .with(LabelKey::Status, status);
        let _ = self.metrics.try_record(MetricEvent::histogram(
            MetricId::StorageTxDurationSeconds,
            started.elapsed().as_secs_f64(),
            labels,
        ));
        if result.is_ok() {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::StorageRowsTotal,
                rows as f64,
                labels,
            ));
        } else {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::ErrorsTotal,
                1.0,
                labels.with(LabelKey::ErrorKind, "storage_failed"),
            ));
        }
    }
}
