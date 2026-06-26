//! # helix-im
//!
//! IM 业务模块（BC-B：IM Business Domain）。
//!
//! ## 设计约束
//!
//! - 只依赖 helix-core，不含任何运行时 crate
//! - 所有 I/O 意图通过 `Effect` 表达，由 driver 异步兑现
//! - 三栖编译：native / wasm32-unknown-unknown / ffi 全绿
//!
//! ## 不渗透验收
//!
//! ```bash
//! grep -r "channel\|message\|cursor\|temporaryId\|im:" \
//!      crates/helix-core/src/  # 期望 0 行
//! ```

pub mod acl;
pub(crate) mod base64;
pub mod channel;
pub mod channel_write;
pub mod commands;
pub mod error;
pub mod http_envelope;
mod lifecycle;
pub mod module;
pub mod older_context;
pub mod outbound;
pub mod parser;
pub mod pending_send;
pub mod pong_compensate;
mod port_reply;
pub mod query;
mod query_dialog_render;
mod query_dispatch;
pub mod read_relay;
pub mod schema;
mod send_flow;
pub mod send_reconcile;
pub mod state;
mod sync_flow;
pub mod sync_increment;
pub mod sync_scheduler;
pub mod sync_session;
pub mod todo;
pub(crate) mod ws;

pub use error::ImError;
pub use module::ImModule;
pub use schema::IM_SCHEMA;
