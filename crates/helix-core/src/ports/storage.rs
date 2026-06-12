//! Storage port — 持久化抽象
//!
//! ## 精简说明（审查意见2 S3 落实）
//!
//! 原草稿 6 方法精简为 4 方法：
//! - 删除 `update`（语义 ⊆ `batch_update`，实现可内部委托）
//! - 删除 `set`（经核查无独立 IM 调用点，等价于 `batch_upsert(rows=1, no_conflict_key)`）
//! - 保留 `monotonic_upsert`（通用单调 MAX guard，独立语义，不可并入 batch_upsert）
//! - 保留 `get`（读路径需要）
//!
//! ## 三端实现（ACL-2）
//! - native: sqlx::SqlitePool (WAL，写池 max=1，读池 max=4)
//! - wasm:   sqlite-wasm-rs + OPFS（需 RUSTFLAGS=atomics+bulk-memory，在 Worker 初始化）
//! - ffi:    host SQLite handle via C ABI

use crate::effect::{GetSpec, MonotonicUpsertSpec, Row, ScanSpec, UpsertSpec};
use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

/// 持久化操作的 core 抽象。
///
/// ## 热路径 O(1) 保证（轴①）
///
/// - `batch_upsert`：ON CONFLICT PK lookup，每行 O(log N) ≈ O(1)
/// - `monotonic_upsert`：单调 MAX guard，单行 WHERE PK UPDATE，O(log N) ≈ O(1)
///
/// ## 注意：模块不直接调用此 trait
///
/// 模块通过 `Effect::Persist{ ops: Vec<StorageOp> }` 表达意图，
/// driver 在 effect 兑现器里调用 Storage，并把结果包成 `Tick::PortReply` 喂回。
/// 这保证 `Module::handle` 严格同步无 I/O。
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Storage: MaybeSend + MaybeSync + 'static {
    /// batch upsert：INSERT ... ON CONFLICT DO UPDATE
    ///
    /// `spec.conflict_key` 的具体值（如 "temporary_id"）由 ACL-1 提供，
    /// driver 只把它拼进 SQL，不理解其业务含义。
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError>;

    /// batch update：UPDATE table SET patch WHERE key_col IN (key_vals)
    ///
    /// `update(single_row)` 即 `batch_update(key_vals.len=1)`。
    /// 实现可对 len=1 的情况内部优化为单行 UPDATE。
    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[crate::effect::SqlValue],
        patch: Row,
    ) -> Result<(), PortError>;

    /// 通用单调 MAX-guard 写：仅在 new value > 当前值时才写入（CAS / 单调寄存器语义）。
    ///
    /// SQL 语义（表名由 driver 约定）：
    /// ```sql
    /// INSERT INTO <table> (scope_key, value) VALUES (?, ?)
    /// ON CONFLICT(scope_key) DO UPDATE SET value = excluded.value
    ///   WHERE excluded.value > <table>.value
    /// ```
    ///
    /// `spec.scope_key` 是运行时值（由上层提供），core 不含业务含义。
    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError>;

    /// get：单行查询
    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError>;

    /// scan：多行扫描读（评审待办 C3）。`limit = None` 表示全表，driver 应设硬上限防御。
    ///
    /// 典型用途：模块 on_start 一次性载入全部单调水位（proactive resync 前置）。
    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError>;
}

// The spec types are defined in crate::effect and imported above.
// They are publicly re-exported through crate::ports::mod.rs which imports them
// via `pub use storage::{Storage, UpsertSpec, MonotonicUpsertSpec, BatchUpdateSpec, GetSpec}`.
