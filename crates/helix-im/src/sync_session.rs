//! SyncSession 聚合 + EventEnvelope
//!
//! ## 源码印证（2026-06-08 实测）
//!
//! - /sync/notify(from_seq) 返回 4 种响应
//! - 分桶顺序铁律：type1 upsert → type2 edit → type3 revoke → type6 read
//! - ≥500 events 需续拉（needs_continuation）
//! - TooLong：cursor = reset_to-1 + Emit(SyncTooLong) → 前端清表重拉

use crate::state::{ChannelId, Seq};

/// 落库所需的 owned typed 视图（HX-C005：字段提取上移 parser，热路径零再解析）。
///
/// parser（`parse_inbound` / `parse_channel_events`，`Value` 已在手）一次解析填充此结构；
/// `channel::event_to_upsert_op` 退化为零再解析的 Row 拼装——不再每事件
/// `serde_json::from_slice(raw)` 全量重解析 + ~14 次 String 堆分配。
///
/// 字段映射真源 = cses-client `From<types::Post> for Message`（PK 三级回退、snake/camel 兼容
/// 已在 parser 提取阶段处理）。`props` 保留服务端原始对象（业务扩展 / event_seq / read map 下游对账）。
#[derive(Debug, Clone, Default)]
pub struct PostFields {
    /// `temporary_id` ?? `temporaryId`（PK 首选，空字符串=缺省，由 event_to_upsert_op 走三级回退）。
    pub temporary_id: String,
    pub id: String,
    /// post 内的 channel_id（缺省回退到 envelope 权威 channel_id，在拼装时处理）。
    pub channel_id: String,
    pub user_id: String,
    pub msg_type: String,
    pub message: String,
    /// 服务端原始 props 对象的字符串形态（业务扩展透传）。
    pub props: String,
    pub create_at: i64,
}

/// 解析后的强类型入站事件（14 类 WS type 的产物）
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    /// per-channel ID（channel_id 的内部引用，两个字段保持同步）
    pub id: ChannelId,
    pub channel_id: ChannelId,
    pub seq: Seq,
    pub kind: EventKind,
    /// 落库所需 owned typed 视图（parser 一次解析填充；HX-C005 热路径零再解析）。
    pub fields: PostFields,
}

impl EventEnvelope {
    /// 构造 EventEnvelope（统一入口，确保 id 与 channel_id 始终一致）。
    ///
    /// parser 经此入口传入已提取的 `fields`（owned typed 视图），落库路径直接读取，
    /// 不再持有 / 重解析原始 JSON 字节（HX-C005）。
    pub fn new(channel_id: ChannelId, seq: Seq, kind: EventKind, fields: PostFields) -> Self {
        Self {
            id: channel_id,
            channel_id,
            seq,
            kind,
            fields,
        }
    }
}

/// `increment_channel` 帧 data（`IncrementChannel.ToMap()`）的解析结果（B-rest 契约）。
///
/// 真 Go data 含 id/teamId/displayName/lastEventSeq/unreadCount/needSync/mentionList/
/// urgentPostList（设计文档 §1.3）；helix 增量同步只需 `channel_id` + `last_event_seq`
/// （作 cursor 种子）+ `need_sync`（false=已追平省 sync）；`raw` 供 emit im:channel:increment 透传。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncrementChannel {
    pub channel_id: ChannelId,
    pub last_event_seq: Seq,
    pub need_sync: bool,
    pub raw: bytes::Bytes,
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
            EventKind::PostEdit => 2,
            EventKind::PostRevoke => 3,
            EventKind::PostRead => 6,
            EventKind::Other(n) => *n,
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
        /// 服务端权威水位（Go `SyncEntry.nextSeq`）：cursor 应推进到此值，**不要**用
        /// `max(event.seq)` 推导。当前契约下二者恒等（Go `next = events[末].EventSeq`），
        /// 但显式锚 nextSeq 防契约漂移（尾部 phantom seq 不进 events 数组时不落后）。
        next_seq: Seq,
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
