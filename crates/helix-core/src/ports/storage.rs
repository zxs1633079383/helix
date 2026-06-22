//! Storage port — 持久化抽象
//!
//! ## 精简说明（审查意见2 S3 落实）
//!
//! 原草稿删掉单行 `update` / `set`，公开面收敛为少数批量/原子 port：
//! `batch_upsert`、`batch_update`、`monotonic_upsert`、`guarded_bump`、`get`、`scan`、
//! `batch_delete`。这些 spec 都是通用存储原语；表名/列名由业务模块提供，core 不理解业务 schema。
//!
//! ## 当前产品边界
//!
//! `helix-driver-host` 提供共享的 rusqlite-backed Storage 实现（schema/migration、SQL 兑现、
//! rows reply 编解码），PC Tauri/native 与 Flutter FFI 交付路径都复用这一个 host-driver
//! 存储边界。`helix-driver-native` 只是兼容 Tauri 侧类型名的薄壳；`helix-driver-ffi` 的 pump
//! 直接装配 `HostStorage`。
//!
//! `helix-driver-wasm` 是 archived/experimental 历史实验：只保留 trait shape 以免代码腐烂，
//! 不属于当前 roadmap、CI 必跑矩阵或交付验收（见 ADR-006）。

use crate::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec, Row, ScanSpec,
    UpsertSpec,
};
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
    /// `update(single_row)` 即 `batch_update(spec.key_vals.len=1)`。
    /// 实现可对 len=1 的情况内部优化为单行 UPDATE。
    ///
    /// CORE-1：收 `BatchUpdateSpec` 与其余 4 个 spec 方法对齐（API 一致性，零行为变更）。
    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError>;

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

    /// 守卫式自增：`UPDATE … SET bump_col = bump_col + delta[, set…] WHERE key=? AND ? > guard_col`。
    ///
    /// 单行原子计数器递增（**无 read-modify-write**，守 HX-C005 O(1)）+ 前进守卫
    /// （仅当 `guard_val > guard_col` 现值才命中）。SQL 语义见 `GuardedBumpSpec`。
    /// 列名都是 `&'static str` 编译期常量（由上层提供，driver 不解释含义，守 HX-C001）。
    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError>;

    /// get：单行查询
    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError>;

    /// scan：多行扫描读（评审待办 C3）。`limit = None` 表示全表，driver 应设硬上限防御。
    ///
    /// 典型用途：模块 on_start 一次性载入全部单调水位（proactive resync 前置）。
    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError>;

    /// batch delete：`DELETE FROM table WHERE scope_col = ? AND key_col IN (key_vals)`。
    ///
    /// 复合 PK 作用域删除（如 channel_member 成员离场：scope=channel_id + key=user_id IN 列表）。
    /// O(k) 单语句；scope 等值约束**绝不跨作用域误删**。列名都是 `&'static str` 编译期常量
    /// （由上层提供，driver 不解释含义，守 HX-C001）。`key_vals` 空 → 上层应不产 op（driver 也兜底跳过）。
    async fn batch_delete(&self, spec: BatchDeleteSpec) -> Result<(), PortError>;
}

// The spec types are defined in crate::effect and imported above.
// They are publicly re-exported through crate::ports::mod.rs.
