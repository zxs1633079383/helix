//! # helix-driver-ffi
//!
//! C ABI / uniffi 平台驱动（Flutter Dart FFI / iOS Swift / Android Kotlin / 纯 C 宿主）。
//!
//! ## FFI 内存安全约束（必须遵守）
//!
//! - C 侧传入的 `*const u8` buffer **必须在回调返回前拷贝到 Rust 堆**
//!   再包成 `Tick::Inbound`。C 侧可能在回调返回后立即复用 buffer。
//! - Flutter/Dart 只调用命令与订阅事件，不暴露 Tick/Effect/StorageOp
//! - 生产 cdylib/staticlib 采用 `panic=abort`，panic 不跨 FFI unwind
//!
//! ## cbindgen 用法
//!
//! ```bash
//! cbindgen --config cbindgen.toml --crate helix-driver-ffi --output helix.h
//! ```

pub mod api;
pub mod callbacks;
pub mod http;
pub mod storage;
pub mod transport;
