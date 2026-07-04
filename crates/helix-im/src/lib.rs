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
pub use channel::older_context;
pub use channel::write as channel_write;
pub mod commands;
pub mod error;
pub mod http_envelope;
pub mod mobile_contract;
pub mod mobile_query;
pub mod module;
pub mod outbound;
pub mod parser {
    pub use crate::ws::parser::core::*;
}
mod port_reply;
mod port_reply_emit;
pub mod query;
pub use query::read_relay;
pub use query::render_ready::core as render_ready;
pub use query::render_ready::members as render_ready_members;
pub use query::render_ready::replies as render_ready_replies;
pub mod schema;
pub mod send;
pub use send::pending as pending_send;
pub use send::pong_compensate;
pub use send::reconcile as send_reconcile;
pub mod state;
pub mod sync;
pub use sync::increment as sync_increment;
pub use sync::scheduler as sync_scheduler;
pub use sync::session as sync_session;
pub mod todo;
pub(crate) mod ws;

pub use error::ImError;
pub use module::ImModule;
pub use schema::IM_SCHEMA;
