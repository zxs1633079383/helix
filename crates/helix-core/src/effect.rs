//! # effect.rs
//!
//! `Effect` 是模块吐出的"想做的 I/O"声明，core 不执行，由 driver 兑现。
//!
//! ## 设计要点
//!
//! - `Persist` 强制带 `Correlation`（类型层逼模块走 peek/commit 两段式，轴⑤）
//! - `PersistFire`：fire-and-forget，**仅用于幂等且丢失可接受的写操作**
//!   （如 monotonic_upsert 在 Persist{corr} 成功后推进单调水位）
//!   禁止用于非幂等业务数据写入（如初次落库消息）
//! - `EffectSink`：跨 step 复用，clear 而非重建，零分配（轴①）
//!   原草稿 EffectBuffer/EffectBatch/EffectSink 三名字收敛为一个（审查意见2 S4）

mod ids;
mod io;
mod operation;
mod sink;
mod storage;

pub use crate::ports::upload::{
    FileUploadProgress, FileUploadProgressPolicy, FileUploadProgressReporter, FileUploadUrls,
};
pub use ids::{Correlation, DomainEventBytes, TimerId, TransportId};
pub use io::{FileUploadRequest, FileUploadResponse, HttpRequest, HttpResponse};
pub use operation::Effect;
pub use sink::EffectSink;
pub use storage::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, GuardComparison, GuardedBumpSpec,
    MonotonicUpsertSpec, Row, ScanOrder, ScanSpec, ScopedGetSpec, ScopedGuardedBumpSpec,
    ScopedMaxSpec, ScopedScanSpec, SortDirection, SparseTableSpec, SqlValue, StorageOp,
    UpsertGuard, UpsertSpec,
};
