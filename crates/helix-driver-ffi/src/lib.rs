//! # helix-driver-ffi
//!
//! C ABI / uniffi 平台驱动（iOS Swift / Android Kotlin / 纯 C 宿主）。
//!
//! ## FFI 内存安全约束（必须遵守）
//!
//! - C 侧传入的 `*const u8` buffer **必须在回调返回前拷贝到 Rust 堆**
//!   再包成 `Tick::Inbound`。C 侧可能在回调返回后立即复用 buffer。
//! - `JsValue`（如果有 wasm 混用）是 `!Send`，绝不让其漏进 helix-core/helix-im
//! - 所有 `#[no_mangle]` 函数必须包含 `catch_unwind`（`panic=abort` 下自动 abort）
//!
//! ## cbindgen 用法
//!
//! ```bash
//! cbindgen --config cbindgen.toml --crate helix-driver-ffi --output helix.h
//! ```

pub mod api;
pub mod callbacks;
pub mod storage;
pub mod transport;
