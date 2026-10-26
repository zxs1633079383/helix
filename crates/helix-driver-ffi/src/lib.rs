//! # helix-driver-ffi
//!
//! C ABI 平台驱动（C++ 直连模型）。
//!
//! ## 模型变更（Dart-host → C++ host）
//!
//! - **已删**：`HelixCallbacks`（Dart-host 喂帧/喂响应遗物）、
//!   `FfiTransportMode::Callback`、`helix_on_ws_frame`、`helix_on_http_response`。
//! - **C++ 直连**：Rust 持有 SharedWsClient + SharedHttpClient；
//!   host 只实现 `CppHostVtable`（事件接收 + 壁钟）。
//!
//! ## FFI 内存安全约束（HX-C006）
//!
//! - C 侧传入的 `*const u8` buffer **必须在函数返回前拷贝到 Rust 堆**（ACL-3）
//! - `Box<T>` ↔ raw ptr 必须用 `Box::into_raw` / `Box::from_raw` 配对，各一次
//! - 生产 cdylib/staticlib 采用 `panic=abort`（workspace release profile 已设置），
//!   panic 不跨 FFI unwind
//!
//! ## cbindgen
//!
//! build.rs 自动调用 cbindgen 生成 `helix.hpp`（C++，pragma_once）。
//! `CppHostVtable` repr(C) 自动扫描进头文件；`HelixHandle` 为不透明前向声明。

pub mod api;
pub mod batch;
pub mod clock;
pub mod event_sink;
pub mod http;
pub mod pump;
pub mod vtable;
