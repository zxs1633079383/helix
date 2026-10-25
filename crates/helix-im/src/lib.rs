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
pub(crate) mod category_chain;
pub(crate) mod chain;
pub mod channel;
mod channel_kind;
pub(crate) mod channel_sync;
mod channel_update;
pub use channel::older_context;
pub use channel::write as channel_write;
pub mod client_api;
pub mod commands;
/// MessageV3 Gate MV3-G02e 草稿域（`im_save_draft` / `im_query_draft`）。
mod diagnostics;
pub mod draft;
pub mod error;
pub mod event;
mod failure_context;
mod forward;
pub mod http_envelope;
mod http_outcome;
mod identity;
mod increment_hydration;
mod increment_pull;
mod message_summary;
pub mod mobile_contract;
pub mod module;
pub mod outbound;
mod presence;
pub mod sync_observation;
pub mod parser {
    pub use crate::ws::parser::core::*;
}
mod port_reply;
mod port_reply_emit;
pub mod query;
mod render_scope;
mod scope_hydration;
pub use query::read_relay;
pub use query::render_ready::candidates as render_ready_candidates;
pub use query::render_ready::core as render_ready;
pub use query::render_ready::locate as render_ready_locate;
pub use query::render_ready::members as render_ready_members;
pub use query::render_ready::replies as render_ready_replies;
mod schedule_media;
pub mod schema;
pub mod send;
pub mod timeline_navigation;
pub mod timeline_state;
mod translation;
pub use send::pending as pending_send;
pub use send::pong_compensate;
pub use send::reconcile as send_reconcile;
pub mod state;
pub mod sync;
pub use sync::increment as sync_increment;
pub use sync::scheduler as sync_scheduler;
pub use sync::session as sync_session;
pub mod todo;
mod topic_activity;
mod topic_inventory;
pub(crate) mod ws;

pub use acl::EVENT_NET_RECONNECT_REQUESTED;
pub use error::ImError;
pub use http_outcome::{http_outcome_event, EVENT_HTTP_UNAUTHORIZED, EVENT_NET_OFFLINE};
pub use module::ImModule;
pub use module::{ClientPlatform, ImConfig};
mod message_identity;
pub use query::{IM_TIMELINE_SEQ_SCHEMA, IM_TIMELINE_SEQ_SCHEMA_VERSION};
pub use schema::{SparseTableSpec, IM_SCHEMA, IM_SCHEMA_MIGRATIONS, IM_SPARSE_TABLE_SPECS};

/// 用户级共库要求公司渲染硬闸门和同公司 create_at 水位同时可用。
/// 宿主在启用物理库迁移前必须绑定本合同，禁止以旧 IM 内核打开跨公司合并库。
pub const USER_SCOPED_STORE_RENDER_CONTRACT_VERSION: u32 = 1;

pub mod recent_history;

mod bookmark_count;
