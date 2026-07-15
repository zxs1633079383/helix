//! # ports
//!
//! core 定义的所有 port trait（BC-A 的 PortRegistry 聚合）。
//!
//! ## 对象安全设计（审查意见2 P2 落实）
//!
//! 所有 port trait **不使用关联类型 `type Error`**，统一返回 `Result<_, PortError>`。
//!
//! 原因：带关联类型的 trait 作为 `dyn Trait` 时，调用方必须命名 `Self::Error`，
//! 但不同 driver 的 Error 类型不同，无法用同一个 `Box<dyn FrameSender>` 持有
//! native 和 wasm 两种实现。统一返回 `PortError` 解决此问题，且去掉了
//! wasm 单线程环境下多余的 `type Error: Send + Sync` 约束。

pub mod clock;
pub mod event_sink;
pub mod frame_sender;
pub mod http;
pub mod id;
pub mod storage;
pub mod upload;

pub use crate::effect::{
    BatchDeleteSpec, BatchUpdateSpec, DeleteWhereSpec, GetSpec, GuardedBumpSpec,
    MonotonicUpsertSpec, ScanSpec, UpsertSpec,
};
pub use clock::Clock;
#[cfg(any(test, feature = "testing"))]
pub use clock::MockClock;
pub use event_sink::EventSink;
pub use frame_sender::FrameSender;
pub use http::HttpRequester;
pub use id::{IdSource, Random};
pub use storage::Storage;
pub use upload::FileUploader;
