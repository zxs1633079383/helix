//! SyncSession 聚合 + EventEnvelope
//!
//! ## 源码印证（2026-06-08 实测）
//!
//! - /sync/notify(from_seq) 返回 4 种响应
//! - 分桶顺序铁律：type1 upsert → type2 edit → type3 revoke → type6 read
//! - ≥500 events 需续拉（needs_continuation）
//! - TooLong：cursor = reset_to-1 + Emit(SyncTooLong) + getLatestPost 覆盖式重拉；不物理删 message

use crate::state::{ChannelId, Seq};

pub(crate) const POST_FIELD_ID: u64 = 1 << 0;
pub(crate) const POST_FIELD_CHANNEL_ID: u64 = 1 << 1;
pub(crate) const POST_FIELD_USER_ID: u64 = 1 << 2;
pub(crate) const POST_FIELD_TYPE: u64 = 1 << 3;
pub(crate) const POST_FIELD_MESSAGE: u64 = 1 << 4;
pub(crate) const POST_FIELD_SIMPLE_MESSAGE: u64 = 1 << 5;
pub(crate) const POST_FIELD_PROPS: u64 = 1 << 6;
pub(crate) const POST_FIELD_USER_SNAPSHOT: u64 = 1 << 7;
pub(crate) const POST_FIELD_CREATE_AT: u64 = 1 << 8;
pub(crate) const POST_FIELD_UPDATE_AT: u64 = 1 << 9;
pub(crate) const POST_FIELD_READ_BITS: u64 = 1 << 10;
pub(crate) const POST_FIELD_SNAPSHOT_ID: u64 = 1 << 11;
pub(crate) const POST_FIELD_VIEWERS: u64 = 1 << 12;
pub(crate) const POST_FIELD_MENTIONS: u64 = 1 << 13;
pub(crate) const POST_FIELD_EXPEDITE_MAP: u64 = 1 << 14;
pub(crate) const POST_FIELD_QUICK_REPLY: u64 = 1 << 15;
pub(crate) const POST_FIELD_TOPIC: u64 = 1 << 16;
pub(crate) const POST_FIELD_REPLY_ID: u64 = 1 << 17;
pub(crate) const POST_FIELD_REPLY_ROOT_ID: u64 = 1 << 18;
pub(crate) const POST_FIELD_REPLY_FIRST_LEVEL_ID: u64 = 1 << 19;
pub(crate) const POST_FIELD_REPLIED_MESSAGE: u64 = 1 << 20;
pub(crate) const POST_FIELD_REPLY_MESSAGES: u64 = 1 << 21;
pub(crate) const POST_FIELD_REPLY_COUNT: u64 = 1 << 22;

/// 落库所需的 owned typed 视图（HX-C005：字段提取上移 parser，热路径零再解析）。
///
/// parser（`parse_inbound` / `parse_channel_events`，`Value` 已在手）一次解析填充此结构；
/// `channel::event_to_upsert_op` 退化为零再解析的 Row 拼装——不再每事件
/// `serde_json::from_slice(raw)` 全量重解析 + ~14 次 String 堆分配。
///
/// 字段映射真源 = cses-client `From<types::Post> for Message`（PK 三级回退、snake/camel 兼容
/// 已在 parser 提取阶段处理）。`props` 保留服务端原始对象（业务扩展 / event_seq / read map 下游对账）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct PostFields {
    /// `temporary_id` ?? `temporaryId`（PK 首选，空字符串=缺省，由 event_to_upsert_op 走三级回退）。
    pub temporary_id: String,
    pub id: String,
    /// post 内的 channel_id（缺省回退到 envelope 权威 channel_id，在拼装时处理）。
    pub channel_id: String,
    pub user_id: String,
    /// Company owning the sender directory entry; identity only, not a profile.
    pub team_id: String,
    /// Wire `type` 字段在内部 typed view 中使用 `msg_type`，日志仍映射回协议名称。
    #[serde(rename = "type")]
    pub msg_type: String,
    pub message: String,
    /// 短预览文本（Go wire `simpleMessage` / `simple_message`），用于 Dialog lastMessage。
    pub simple_message: String,
    /// 服务端原始 props 对象的字符串形态（业务扩展透传）。
    pub props: String,
    /// 旧调用方兼容字段；生产入站与持久化始终为空，姓名由宿主用户目录提供。
    pub user_snapshot: String,
    pub create_at: i64,
    /// 服务端消息更新时间；缺省保持 0，禁止用本地墙钟伪造远端权威值。
    pub update_at: i64,
    /// 服务端权威已读位（C2 / type=6 真源 post.rs:1087-1089）。
    /// 缺省空串=该 post 无已读信息；落库时**单调覆盖** message.read_bits 列（非累加）。
    pub read_bits: String,
    /// 创建时成员快照权威键；缺省空串表示服务端未提供，禁止本地合成。
    pub snapshot_id: String,
    /// 可见性受众（A3/CAP-9：sync 应用未读 +1 的 `visible_to_user` 谓词真源
    /// message_service.rs:323-333——`viewers` 含 `"all"` 或当前 auth_id 即可见）。
    /// 缺省空 Vec（无堆分配）；NOTICE 类型恒可见不依赖此字段。
    pub viewers: Vec<String>,
    /// `mentions` 用户 id 列表，供 Rust 侧计算 Dialog mention patch。
    pub mentions: Vec<String>,
    /// 服务端原始 expediteMap / expedite_map 对象字符串，供 urgent 判定和存储透传。
    pub expedite_map: String,
    /// Go `post_update.quickReply` 规范化数组的 JSON 文本，独立落 message.quick_reply。
    pub quick_reply: String,
    /// 消息的话题映射对象（Go wire `topic`），原样 JSON 持久化并投影给 UI。
    pub topic: String,
    /// 被回复消息 id，以及服务端归一后的回复根/一级回复锚。
    /// 这些字段由 Helix parser 原样持久化，UI 只消费投影，不计算回复层级。
    pub reply_id: String,
    pub reply_root_id: String,
    pub reply_first_level_id: String,
    /// 被引用消息与回复摘要由服务端直接给出，Helix 只做一次解析/投影。
    /// JSON 字段保留字符串形态落库，避免 UI 再探测 snake/camel 或重组树。
    pub replied_message: String,
    pub reply_messages: String,
    pub reply_count: i64,
    /// JSON key presence mask; missing fields must not overwrite an existing readback row.
    #[serde(skip)]
    pub(crate) present_fields: u64,
}

impl PostFields {
    /// Reports whether the source payload explicitly contained the given field.
    pub(crate) fn has_field(&self, field: u64) -> bool {
        self.present_fields & field != 0
    }
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
    /// 服务端事件行 identity。撤回投影使用这些字段生成 viewer-ready 系统事件；它们与原消息
    /// 作者完全独立，缺失时保持空值/0，绝不从 `fields.user_id` 猜 actor。
    pub event_id: String,
    pub actor_id: String,
    pub occurred_at: i64,
    /// ChannelEvent.payload 原样 JSON；当前撤回合同为空也保留，防后续系统事件扩展被 parser 丢弃。
    pub event_payload: String,
    /// 同一业务效果在 stream 与 member projection 间的关联键；不参与排序或幂等。
    pub effect_id: String,
    /// ACL marker 只占据 stream 位置，不得物化消息或泄露 actor/payload。
    pub redacted: bool,
    /// ⑤ 未读 +1 决策（在边界 handler 用**原始帧 + auth + 完整 echo 判定**算定，gate 在
    /// apply/flush **两路**统一发出）。`Some` = 该 PostUpsert 落库时一并 `GuardedBump` unread
    /// （已含 sender 豁免 + 可见性 + echo 跳过）；`None` = 不 bump（echo / 非新消息 / 其它 kind /
    /// sync 路径——sync 未读由 CAP-9 apply 自管，故默认 None 不重复 +1）。
    ///
    /// **乱序进 gate buffer 的事件同样携带本决策**，flush 时由 gate 发出——修复「乱序 post 经
    /// buffer flush 不补未读」偏差（gate 成为未读 bump 的**单一发出点**，立即 apply 与 flush 同口径）。
    pub unread_bump: Option<crate::channel_write::PostChannelUpdate>,
    /// 当前投影视角用户；仅用于生成 `isSelf` 与消息动作 capability，不参与存储。
    pub viewer_user_id: String,
    /// Trusted action causation copied from the WS envelope, never from post content.
    pub causation_id: Option<String>,
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
            event_id: String::new(),
            actor_id: String::new(),
            occurred_at: 0,
            event_payload: String::new(),
            effect_id: String::new(),
            redacted: false,
            unread_bump: None,
            viewer_user_id: String::new(),
            causation_id: None,
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

    /// 附加 wire 权威事件 identity。空 actor 保持 unavailable，调用方不得回退为消息作者。
    pub fn with_event_identity(
        mut self,
        event_id: Option<String>,
        actor_id: Option<String>,
        occurred_at: i64,
        event_payload: String,
    ) -> Self {
        self.event_id = event_id.filter(|id| !id.is_empty()).unwrap_or_default();
        self.actor_id = actor_id.filter(|id| !id.is_empty()).unwrap_or_default();
        self.occurred_at = occurred_at.max(0);
        self.event_payload = event_payload;
        self
    }

    pub fn with_effect(mut self, effect_id: Option<String>, redacted: bool) -> Self {
        self.effect_id = effect_id.filter(|id| !id.is_empty()).unwrap_or_default();
        self.redacted = redacted;
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

    pub fn with_viewer_user_id(mut self, viewer_user_id: &str) -> Self {
        self.viewer_user_id = viewer_user_id.to_string();
        self
    }

    pub fn with_causation_id(mut self, causation_id: Option<String>) -> Self {
        self.causation_id = causation_id.filter(|value| !value.is_empty());
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
    /// type7：频道终态 tombstone。
    ///
    /// 该事件只允许经 `/channel/sync/notify` 的严格 wire 进入；它不是 post，不能落到
    /// message 表或沿用普通 cursor 提交链。`sync::flow` 会把 tombstone marker 与 cursor
    /// 放进同一个 `PersistAtomic`，收到成功回执后才关闭本地 channel 并发布终态事件。
    ChannelTerminalClosed,
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
            EventKind::ChannelTerminalClosed => 7,
            EventKind::Other(n) => *n,
        }
    }
}

/// sync 响应的 4 种形态
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncPersona {
    pub membership_state: String,
    pub epoch_start_seq: Option<Seq>,
    pub epoch_end_seq: Option<Seq>,
    pub member_projection: Option<serde_json::Value>,
}

#[derive(Debug)]
pub enum SyncResponse {
    /// 无新事件（cursor 已是最新）
    NoChange { next_seq: Seq, persona: SyncPersona },
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
        persona: SyncPersona,
    },
    /// 服务端返回快照（覆盖式 upsert，不先物理删除既有 message）
    Snapshot(ChannelSnapshot),
    /// cursor gap 过大，保留旧 message 并覆盖式重拉
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

/// One reconnect/relogin authority window.  It deliberately contains only
/// correlation-free state: the actual network and storage work is still
/// represented by Effects and their matching PortReply contexts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySession {
    pub phase: RecoveryPhase,
    pub session_epoch: u64,
    pub actor_id: String,
    pub pending_commits: std::collections::BTreeMap<ChannelId, Seq>,
    completion_published: bool,
}

impl Default for RecoverySession {
    fn default() -> Self {
        Self {
            phase: RecoveryPhase::Idle,
            session_epoch: 0,
            actor_id: String::new(),
            pending_commits: std::collections::BTreeMap::new(),
            completion_published: false,
        }
    }
}

impl RecoverySession {
    /// Start a new trust boundary.  Epoch zero is intentionally never emitted
    /// on the V2 wire, so the first usable session is one.
    pub fn begin(&mut self, actor_id: &str) {
        self.session_epoch = self.session_epoch.saturating_add(1).max(1);
        self.actor_id.clear();
        self.actor_id.push_str(actor_id);
        self.pending_commits.clear();
        self.completion_published = false;
        self.phase = RecoveryPhase::Comparing;
    }

    pub fn invalidate(&mut self) {
        self.pending_commits.clear();
        self.completion_published = false;
        self.phase = RecoveryPhase::Idle;
        self.actor_id.clear();
    }

    pub fn compare(
        &mut self,
        local: CommittedRecoveryHead,
        authority: AuthorityHead,
    ) -> RecoveryComparison {
        let comparison = compare_committed_recovery(local, authority);
        self.phase = match comparison {
            RecoveryComparison::Equal => RecoveryPhase::Recovered,
            RecoveryComparison::Pull { .. } => RecoveryPhase::Pulling,
            RecoveryComparison::AuthorityReloadRequired => RecoveryPhase::Blocked,
        };
        comparison
    }

    pub fn await_commit(&mut self, channel_id: ChannelId, committed_to: Seq) {
        self.pending_commits.insert(channel_id, committed_to);
        self.phase = RecoveryPhase::AwaitingCommit;
    }

    /// A storage acknowledgement is meaningful only when it matches exactly
    /// the batch that this session registered.  Late A-session acknowledgements
    /// are therefore a no-op rather than a renderer-visible completion.
    pub fn commit_ok(&mut self, channel_id: ChannelId, committed_to: Seq) -> bool {
        if self.pending_commits.remove(&channel_id) != Some(committed_to) {
            return false;
        }
        self.phase = if self.pending_commits.is_empty() {
            RecoveryPhase::Recovered
        } else {
            RecoveryPhase::AwaitingCommit
        };
        true
    }

    pub fn commit_failed(&mut self, channel_id: ChannelId) {
        self.pending_commits.remove(&channel_id);
        self.phase = RecoveryPhase::Failed;
    }

    pub fn is_active_for(&self, actor_id: &str) -> bool {
        self.session_epoch != 0 && self.actor_id == actor_id && !self.actor_id.is_empty()
    }

    pub fn is_collecting_for(&self, actor_id: &str) -> bool {
        self.is_active_for(actor_id) && !self.completion_published
    }

    pub fn is_collecting(&self) -> bool {
        self.session_epoch != 0 && !self.actor_id.is_empty() && !self.completion_published
    }

    pub fn has_pending_commits(&self) -> bool {
        !self.pending_commits.is_empty()
    }

    pub fn mark_completion_published(&mut self) {
        self.completion_published = true;
    }
}

/// Recovery is deliberately a pure comparison model. The caller obtains both
/// inputs through ports, persists any chosen batch, and only then advances the
/// in-memory head; this type cannot manufacture a UI completion on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPhase {
    Idle,
    Comparing,
    Pulling,
    AwaitingCommit,
    Recovered,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityHead {
    pub event_seq: Seq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommittedRecoveryHead {
    pub cursor: Seq,
    pub ledger_to_seq: Seq,
    pub coverage_to_seq: Seq,
}

impl CommittedRecoveryHead {
    pub const fn is_coherent(self) -> bool {
        self.cursor.0 == self.ledger_to_seq.0 && self.cursor.0 == self.coverage_to_seq.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryComparison {
    Equal,
    Pull {
        from_exclusive: Seq,
        to_inclusive: Seq,
    },
    AuthorityReloadRequired,
}

/// A normalized sync batch whose sequence evidence has already been checked.
/// The parser stays responsible for wire decoding; this type owns the
/// cross-entry invariants shared by reconnect and incremental recovery.
#[derive(Debug, Clone)]
pub struct SyncBatchFacts {
    pub channel_id: ChannelId,
    pub from_exclusive: Seq,
    pub authority_head: AuthorityHead,
    pub events: Vec<EventEnvelope>,
}

impl SyncBatchFacts {
    pub fn from_events(
        channel_id: ChannelId,
        from_exclusive: Seq,
        authority_head: Seq,
        events: Vec<EventEnvelope>,
    ) -> Result<Self, &'static str> {
        if events.is_empty() {
            return Err("sync batch facts require at least one event");
        }
        let mut previous = from_exclusive;
        for event in &events {
            if event.channel_id != channel_id || event.id != channel_id {
                return Err("sync batch event channel differs from request channel");
            }
            // sync/notify is viewer-filtered: invisible channel events create
            // legitimate gaps. The returned facts must still be strictly
            // increasing and newer than the committed cursor.
            if event.seq <= previous {
                return Err("sync batch event sequence is not strictly increasing");
            }
            previous = event.seq;
        }
        let last = events.last().map(|event| event.seq).unwrap_or(Seq(0));
        if authority_head < last {
            return Err("sync batch authority head precedes final event");
        }
        Ok(Self {
            channel_id,
            from_exclusive,
            authority_head: AuthorityHead {
                event_seq: authority_head,
            },
            events,
        })
    }
}

/// Compare only committed local evidence with an authority head. An incoherent
/// local ledger/coverage or local-ahead state is a fail-closed authority reload,
/// never a client-side attempt to fill a missing sequence.
pub const fn compare_committed_recovery(
    local: CommittedRecoveryHead,
    authority: AuthorityHead,
) -> RecoveryComparison {
    if !local.is_coherent() || local.cursor.0 > authority.event_seq.0 {
        return RecoveryComparison::AuthorityReloadRequired;
    }
    if local.cursor.0 == authority.event_seq.0 {
        RecoveryComparison::Equal
    } else {
        RecoveryComparison::Pull {
            from_exclusive: local.cursor,
            to_inclusive: authority.event_seq,
        }
    }
}
