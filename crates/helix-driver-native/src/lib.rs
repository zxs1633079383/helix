//! # helix-driver-native
//!
//! tokio + shared rusqlite host storage + tokio-tungstenite + reqwest 实现 helix-core ports。
//!
//! ## tokio::spawn 在此
//!
//! helix-core 不含任何运行时依赖。`tokio::spawn` 在此 crate 的 `engine_loop.rs`，
//! 不在 helix-core。
//!
//! ## 错误翻译（ACL-2）
//!
//! 每个 port 实现用 thiserror 把具体 crate 错误（rusqlite / reqwest / 等）
//! 翻译为 helix_core::PortError，不让具体类型穿透 port 边界。

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
