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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostFields {
    /// `temporary_id` ?? `temporaryId`（PK 首选，空字符串=缺省，由 event_to_upsert_op 走三级回退）。
    pub temporary_id: String,
    pub id: String,
    /// post 内的 channel_id（缺省回退到 envelope 权威 channel_id，在拼装时处理）。
    pub channel_id: String,
    pub user_id: String,
    pub msg_type: String,
    pub message: String,
    /// 短预览文本（Go wire `simpleMessage` / `simple_message`），用于 Dialog lastMessage。
    pub simple_message: String,
    /// 服务端原始 props 对象的字符串形态（业务扩展透传）。
    pub props: String,
    /// 服务端原始 userSnapshot 对象的字符串形态，用于 senderUserId 优先级与渲染透传。
    pub user_snapshot: String,
    pub create_at: i64,
    /// 服务端权威已读位（C2 / type=6 真源 post.rs:1087-1089）。
    /// 缺省空串=该 post 无已读信息；落库时**单调覆盖** message.read_bits 列（非累加）。
    pub read_bits: String,
    /// 可见性受众（A3/CAP-9：sync 应用未读 +1 的 `visible_to_user` 谓词真源
    /// message_service.rs:323-333——`viewers` 含 `"all"` 或当前 auth_id 即可见）。
    /// 缺省空 Vec（无堆分配）；NOTICE 类型恒可见不依赖此字段。
    pub viewers: Vec<String>,
    /// `mentions` 用户 id 列表，供 Rust 侧计算 Dialog mention patch。
    pub mentions: Vec<String>,
    /// 服务端原始 expediteMap / expedite_map 对象字符串，供 urgent 判定和存储透传。
    pub expedite_map: String,
    /// 被回复消息 id，以及服务端归一后的回复根/一级回复锚。
    /// 这些字段由 Helix parser 原样持久化，UI 只消费投影，不计算回复层级。
    pub reply_id: String,
    pub reply_root_id: String,
    pub reply_first_level_id: String,
}

/// 解析后的强类型入站事件（14 类 WS type 的产物）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnvelope {
    /// per-channel ID（channel_id 的内部引用，两个字段保持同步）
    pub id: ChannelId,
    pub channel_id: ChannelId,
    pub seq: Seq,
    pub kind: EventKind,
    /// 落库所需 owned typed 视图（parser 一次解析填充；HX-C005 热路径零再解析）。
    pub fields: PostFields,
    /// 事件 wire 的 `msgId`（C2 phantom 判定真源 post.rs:1064-1069）。
    ///
    /// sync `events` 路径：事件本身只带 `eventSeq/eventType/msgId/actorId`，消息内容在
    /// 分离的 `messages` map（key=msgId）。`msg_id ∈ messages` → 落内容行；`∉` → phantom
    /// （cursor 推进但不落 message 行，HX-C008 §1.6）。`None` = 无 msgId（read 事件可空 / legacy）。
    pub msg_id: Option<String>,
    /// ⑤ 未读 +1 决策（在边界 handler 用**原始帧 + auth + 完整 echo 判定**算定，gate 在
    /// apply/flush **两路**统一发出）。`Some` = 该 PostUpsert 落库时一并 `GuardedBump` unread
    /// （已含 sender 豁免 + 可见性 + echo 跳过）；`None` = 不 bump（echo / 非新消息 / 其它 kind /
    /// sync 路径——sync 未读由 CAP-9 apply 自管，故默认 None 不重复 +1）。
    ///
    /// **乱序进 gate buffer 的事件同样携带本决策**，flush 时由 gate 发出——修复「乱序 post 经
    /// buffer flush 不补未读」偏差（gate 成为未读 bump 的**单一发出点**，立即 apply 与 flush 同口径）。
    pub unread_bump: Option<crate::channel_write::PostChannelUpdate>,
}

impl EventEnvelope {
    /// 构造 EventEnvelope（统一入口，确保 id 与 channel_id 始终一致）。
    ///
    /// parser 经此入口传入已提取的 `fields`（owned typed 视图），落库路径直接读取，
    /// 不再持有 / 重解析原始 JSON 字节（HX-C005）。
    ///
    /// `msg_id` 缺省从 fields 派生（`id` ?? `temporary_id`，皆空=None）——既有调用方
    /// （WS 路径 / driver-host 测试）无须改签名；sync 路径再用 `with_msg_id` 覆盖为 wire 权威值。
    pub fn new(channel_id: ChannelId, seq: Seq, kind: EventKind, fields: PostFields) -> Self {
        let msg_id = if !fields.id.is_empty() {
            Some(fields.id.clone())
        } else if !fields.temporary_id.is_empty() {
            Some(fields.temporary_id.clone())
        } else {
            None
        };
        Self {
            id: channel_id,
            channel_id,
            seq,
            kind,
            fields,
            msg_id,
            unread_bump: None,
        }
    }

    /// 覆盖 wire 权威 `msgId`（sync `events` / WS 路径，C2 phantom 判定）。
    ///
    /// 仅当传入 `Some(非空)` 时覆盖——`None` / `Some("")` 保留 `new` 派生的 fallback
    /// （`id` ?? `temporary_id`），不抹掉。边界零信任，helix-im 不变量 4。
    pub fn with_msg_id(mut self, msg_id: Option<String>) -> Self {
        if let Some(id) = msg_id.filter(|s| !s.is_empty()) {
            self.msg_id = Some(id);
        }
        self
    }

    /// 附加 ⑤ 未读 bump 决策（仅 `post` handler 对新消息算定后调用）。
    /// gate 在 apply/flush 两路读取并发出——确保乱序经 buffer 的消息 flush 时也补未读。
    pub fn with_unread_bump(
        mut self,
        bump: Option<crate::channel_write::PostChannelUpdate>,
    ) -> Self {
        self.unread_bump = bump;
        self
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        /// C2：服务端按可见性过滤的消息内容快照（key=msgId，真源 post.rs:1047 `messages` 参数）。
        ///
        /// 落库时：type1/2 的 `event.msg_id ∈ messages` → 用此 map 内容落 message 行；
        /// `∉` → phantom（不落行，cursor 照推）。type6 → 读 `messages[msg_id].read_bits`
        /// 单调覆盖。空 map = 全 phantom（仅推 cursor）。owned `PostFields`（HX-C005）。
        messages: std::collections::HashMap<String, PostFields>,
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
