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
mod projection_control_effects;
/// 连接控制事件的唯一真源：helix-im 负责 emit，driver 侧按此常量观察（禁止各自写字面量）。
pub use projection_control_effects::EVENT_NET_RECONNECT_REQUESTED;
mod projection_message_effects;
mod storage_effects;
pub mod sync_effects;
pub(crate) mod sync_http_effects;
pub mod to_effect;
pub mod to_effect_s1;
mod transport_effects;
