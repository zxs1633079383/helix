//! 三端共享的 IM 业务调用门面。
//!
//! native、FFI 与 Web 平台边界只能在这里把公开业务名和 versioned payload 转成
//! `AppCommand`；alias、payload 归一化与 capabilities 不得复制到 driver。

mod capabilities;
mod command;

pub use capabilities::{
    capabilities_json, BUS_CHANNEL, CONTRACT_VERSION, META_QUERIES, PUBLIC_EVENTS,
};
pub use command::{
    build_command, command_alias, normalize_command_payload, CommandAlias, COMMAND_ALIASES,
};
