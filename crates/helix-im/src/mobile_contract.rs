//! 旧移动端模块名的兼容入口。
//!
//! 新代码统一使用 [`crate::client_api`]；此模块只 re-export，不再持有业务契约实现。

pub use crate::client_api::{
    build_command, capabilities_json, command_alias, normalize_command_payload, CommandAlias,
    BUS_CHANNEL, COMMAND_ALIASES, CONTRACT_VERSION, META_QUERIES, PUBLIC_EVENTS,
};
