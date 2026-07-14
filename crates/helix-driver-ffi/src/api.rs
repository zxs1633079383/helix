//! FFI 公开 API（#[no_mangle] C ABI 导出）
//!
//! ## C++ 直连模型
//!
//! - Rust 持有网络（SharedWsClient / SharedHttpClient）；C++ host 不喂帧/喂响应。
//! - 事件回传：pump 吐 Effect::Emit → FfiEventSink 攒批 → 每 step 单次
//!   `CppHostVtable::on_domain_events_batch`（HX-C007）。
//! - pump：`helix_create_with_network` 起专用线程（current_thread runtime）跑 host
//!   `run_engine_loop`；命令经 tick channel 进泵，事件经 vtable 回 C++。
//!
//! ## 内存安全约束（HX-C006）
//!
//! - 所有 *const u8 参数：**在函数返回前必须拷贝到 Rust 堆**（ACL-3）
//! - Box<T> raw ptr 必须用 Box::into_raw / Box::from_raw 配对，各一次
//! - panic=abort 保证不跨 FFI unwind（release profile 已设置）
//! - 每个 unsafe 块必须有 // SAFETY: 注释（clippy deny undocumented_unsafe_blocks）

mod command;
mod handle;
mod headers;
mod query;
mod shared;

pub use command::helix_command;
pub use handle::{helix_create_with_network, helix_destroy, HelixHandle};
pub use headers::{helix_remove_header, helix_set_header};
pub use query::{helix_query, helix_query_free};
pub use shared::{FFI_BUSY, FFI_INVALID, FFI_NULL, FFI_OK, FFI_UNIMPLEMENTED, FFI_UTF8};

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "api_e2e_tests.rs"]
mod e2e_tests;
