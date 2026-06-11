//! # helix-driver-native
//!
//! tokio + sqlx + tokio-tungstenite + reqwest 实现 helix-core ports。
//!
//! ## tokio::spawn 在此
//!
//! helix-core 不含任何运行时依赖。`tokio::spawn` 在此 crate 的 `engine_loop.rs`，
//! 不在 helix-core。
//!
//! ## 错误翻译（ACL-2）
//!
//! 每个 port 实现用 thiserror 把具体 crate 错误（sqlx::Error / reqwest::Error / 等）
//! 翻译为 helix_core::PortError，不让具体类型穿透 port 边界。

pub mod transport;
pub mod storage;
pub mod clock;
pub mod event_sink;
pub mod http;
pub mod engine_loop;
pub mod timer;
