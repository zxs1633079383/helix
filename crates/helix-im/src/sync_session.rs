//! SyncSession 聚合 + EventEnvelope
//!
//! ## 源码印证（2026-06-08 实测）
//!
//! - /sync/notify(from_seq) 返回 4 种响应
//! - 分桶顺序铁律：type1 upsert → type2 edit → type3 revoke → type6 read
//! - ≥500 events 需续拉（needs_continuation）
//! - TooLong：cursor = reset_to-1 + Emit(SyncTooLong) → 前端清表重拉

use crate::state::{ChannelId, Seq};

/// 解析后的强类型入站事件（14 类 WS type 的产物）
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    /// per-channel ID（channel_id 的内部引用，两个字段保持同步）
    pub id: ChannelId,
    pub channel_id: ChannelId,
    pub seq: Seq,
    pub kind: EventKind,
    /// 原始 JSON payload（ACL-1 决定如何翻译为 StorageOp）
    pub raw: bytes::Bytes,
}

impl EventEnvelope {
    /// 构造 EventEnvelope（统一入口，确保 id 与 channel_id 始终一致）
    pub fn new(channel_id: ChannelId, seq: Seq, kind: EventKind, raw: bytes::Bytes) -> Self {
        Self {
            id: channel_id,
            channel_id,
            seq,
            kind,
            raw,
        }
    }
}

/// IM 事件类型（对应 WS 协议 type 字段，源码 14 类分桶）
#[derive(Debug, Clone)]
pub enum EventKind {
    /// type1：新消息 upsert
    PostUpsert,
    /// type2：消息编辑
    PostEdit,
    /// type3：消息撤回
    PostRevoke,
    /// type6：已读
    PostRead,
    /// 其他类型（暂不处理）
    Other(u8),
}

impl EventKind {
    /// 返回对应的 WS 协议 type 数字（用于落库和分桶排序）
    pub fn type_num(&self) -> u8 {
        match self {
            EventKind::PostUpsert => 1,
            EventKind::PostEdit   => 2,
            EventKind::PostRevoke => 3,
            EventKind::PostRead   => 6,
            EventKind::Other(n)   => *n,
        }
    }
}

/// sync 响应的 4 种形态
#[derive(Debug)]
pub enum SyncResponse {
    /// 无新事件（cursor 已是最新）
    NoChange,
    /// 有事件（列表，≥500 时 needs_continuation=true 需续拉）
    Events {
        events: Vec<EventEnvelope>,
        needs_continuation: bool,
    },
    /// 服务端返回快照（cursor 跳跃过大时的降级）
    Snapshot(ChannelSnapshot),
    /// cursor gap 过大，需清表重拉
    TooLong { reset_to: Seq },
}

/// 频道快照（用于 Snapshot 响应的全量重建）
#[derive(Debug)]
pub struct ChannelSnapshot {
    pub channel_id: ChannelId,
    pub reset_to: Seq,
    pub messages: Vec<EventEnvelope>,
}

/// SyncSession 状态（一次 /sync/notify 请求的生命周期）
pub struct SyncSession {
    pub channel_id: ChannelId,
    pub from_seq: Seq,
    pub corr: helix_core::Correlation,
}
