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
pub mod channel;
pub mod error;
pub mod module;
pub mod parser;
pub mod pending_send;
pub mod state;
pub mod sync_session;

pub use error::ImError;
pub use module::ImModule;
