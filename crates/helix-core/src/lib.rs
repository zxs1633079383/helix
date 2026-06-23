//! # helix-core
//!
//! Runtime-agnostic 确定性业务内核基座的核心层。
//!
//! ## 设计约束（不可违反）
//!
//! - 此 crate 不含任何运行时依赖（无 tokio / sqlx / web-sys / tauri）
//! - `engine::ExecutionShell::step()` 严格同步，零 await，零 I/O，零 spawn
//! - spawn 能力由各 `helix-driver-*` 的 `engine_loop.rs` 持有
//! - 所有 port trait 返回 `Result<_, PortError>`，确保对象安全（dyn-safe）
//! - `Module` trait 4 方法（无 tick()），周期行为由 `ScheduleTimer` self-arm 表达
//!
//! ## 三栖编译
//!
//! ```bash
//! # native
//! cargo check -p helix-core
//! # wasm32
//! cargo check -p helix-core --target wasm32-unknown-unknown
//! # ffi (aarch64-apple-ios as example)
//! cargo check -p helix-core --target aarch64-apple-ios
//! ```

pub mod auth;
pub mod effect;
pub mod engine;
pub mod error;
pub mod module_host;
pub mod platform;
pub mod ports;
pub mod tick;

// Re-export most-used types at crate root for ergonomic imports
pub use auth::{AuthKind, AUTH_KIND_HEADER};
pub use effect::{Correlation, DomainEventBytes, Effect, EffectSink, TimerId};
pub use engine::ExecutionShell;
pub use error::{CoreError, PortError};
pub use module_host::{Module, ModuleHost};
pub use platform::{MaybeSend, MaybeSync};
pub use tick::Tick;
