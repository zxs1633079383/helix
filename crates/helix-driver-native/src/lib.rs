//! # helix-driver-native
//!
//! tokio shell + shared host storage/network implementations for helix-core ports.
//!
//! ## tokio::spawn 在此
//!
//! helix-core 不含任何运行时依赖。`tokio::spawn` 在此 crate 的 `engine_loop.rs`，
//! 不在 helix-core。
//!
//! ## 错误翻译（ACL-2）
//!
//! 具体 I/O crate 错误在 driver/host 边界翻译为 helix_core::PortError，
//! 不让具体类型穿透 port 边界。

pub mod clock;
pub mod engine_loop;
pub mod event_sink;
pub mod http;
pub mod storage;
pub mod timer;
pub mod transport;

// 公共 API 再导出（方便 helix-tauri / 测试直接 use helix_driver_native::*）
pub use clock::NativeClock;
pub use engine_loop::{rows_from_reply_bytes, rows_to_reply_bytes, run_engine_loop, EngineConfig};
pub use event_sink::{EventReceiver, NativeEventSink, RecvOutcome};
pub use http::NativeHttp;
pub use storage::NativeStorage;
pub use timer::TimerRegistry;
pub use transport::NativeTransport;
