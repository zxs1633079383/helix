//! # helix-driver-wasm
//!
//! wasm32-unknown-unknown 平台驱动。
//!
//! ## MAJ-2: 编译目标强制检查
//!
//! 此 crate 必须且只能编译到 wasm32-unknown-unknown。
//! 下方的 `compile_error!` 在非 wasm32 target 时立即报错，
//! 防止误用（如在 native CI 中链接此 crate 后运行时神秘失败）。
//!
//! ## spawn 在此
//!
//! `wasm_bindgen_futures::spawn_local` 在此 crate 的 `engine_loop.rs`，
//! helix-core 不含任何 wasm 运行时依赖。
//!
//! ## OPFS 编译要求
//!
//! sqlite-wasm-rs 的 OPFS 模式需要：
//! ```bash
//! # .cargo/config.toml 的 wasm32-unknown-unknown target 段
//! [target.wasm32-unknown-unknown]
//! rustflags = ["-C", "target-feature=+atomics,+bulk-memory,+mutable-globals"]
//! ```
//! 同时需要 `-Z build-std` 或 nightly（stable 对 atomics 的支持有限）。
//! 初始化必须在 Web Worker 内完成，不能在主线程（见 MAJ-3）。
//!
//! ## ws_stream_wasm 维护状态
//!
//! ws_stream_wasm 最后活跃提交 2021 年。备选：`web-sys WebSocket + futures-channel 手桥`。
//! 评估后选择备选方案时，替换 WasmTransport 实现即可，port 契约不变。

// MAJ-2: 非 wasm32 target 时立即编译失败，防止误链接
#[cfg(not(target_arch = "wasm32"))]
compile_error!(
    "helix-driver-wasm must only be compiled for wasm32-unknown-unknown. \
     Use helix-driver-native for native targets, helix-driver-ffi for FFI/mobile."
);

pub mod clock;
pub mod engine_loop;
pub mod event_sink;
pub mod http;
pub mod storage;
pub mod transport;
