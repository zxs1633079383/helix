//! Outbound 应用命令入口（P2 起 = `outbound::registry` 的薄 re-export shim）。
//!
//! P2-C4：原本本文件持 `is_outbound` 单 `match` + `handle_outbound` 大 `match`，已拆成
//! `outbound/registry.rs`（distributed_slice 注册表）+ 各命令独立注册文件（urgent/quick_reply/
//! post_props/template_received/posts_existing/channel_existing）。本文件仅保留对外路径
//! `crate::commands::{is_outbound, handle_outbound}` 不变（accepts_tick / module.rs dispatch /
//! 既有集成测试导入路径稳定），实现全部下沉 registry。
//!
//! `im_send_message` 因带乐观落库 / timer / PendingSend 对账，仍留 `module.rs`，不在本表。

pub use crate::outbound::registry::{handle_outbound, is_outbound, outbound_command_count};
