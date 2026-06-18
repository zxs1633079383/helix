//! # ACL-1：IM 业务域 ⇆ helix-core 平台域
//!
//! 防腐层（Anti-Corruption Layer），位于 helix-im 一侧（customer 侧，DDD 标准做法）。
//!
//! ## 职责
//!
//! - `to_effect`：IM 概念 → core Effect（IM 业务参数转为通用参数）
//! - `from_tick`：core Tick → IM 事件（不透明字节解析为 IM 强类型）
//!
//! ## 不渗透原则
//!
//! core trait 签名里只有通用参数（table: &str / conflict_key: &str / scope_key: &str）。
//! `temporary_id`、`channel_event_seq`、`im:` 前缀、`15000ms` 全在此模块拼成参数值，
//! 不进 helix-core 的类型系统。

pub mod from_tick;
pub mod to_effect;
pub mod to_effect_s1;
