//! # platform.rs
//!
//! 三栖 Send/Sync 管理——纯 cfg trait 别名，**零运行时依赖**。
//!
//! ## 设计说明
//!
//! - native (non-wasm32)：`MaybeSend = Send`，`MaybeSync = Sync`（多线程安全）
//! - wasm32：`MaybeSend = ()`，`MaybeSync = ()`（单线程，Send 约束无意义）
//!
//! ## spawn 能力不在此处
//!
//! `tokio::spawn` 和 `wasm_bindgen_futures::spawn_local` 不属于 helix-core。
//! 它们由各 driver 的 `engine_loop.rs` 持有，helix-core 保持零运行时依赖。

// ─── native（非 wasm32）─────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> MaybeSync for T {}

// ─── wasm32（单线程，Send/Sync 无意义）─────────────────────────────────────

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}

#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}

#[cfg(target_arch = "wasm32")]
impl<T> MaybeSync for T {}
