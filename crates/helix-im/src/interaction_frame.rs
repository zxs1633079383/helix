//! UC01-16 的跨端、版本化 InteractionFrame 合同。
//!
//! 该模块只定义 IM 的语义投影契约和校验规则；不依赖 renderer、driver 或运行时。平台端
//! 以 `(surface, canonical_scope)` 建 slot，整帧校验成功后才机械提交。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const INTERACTION_FRAME_SCHEMA_VERSION: u32 = 1;
pub const INTERACTION_FRAME_EVENT: &str = "im:interaction:frame";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SurfaceId {
    MessageLeftDialogList,
    MessageUnreadSummary,
    MessageTimeline,
    MessageThreadDetail,
    MessageChannelSummary,
    MessageMemberRoster,
    MessageTopicList,
    MessageParticipantReceipts,
}

impl SurfaceId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MessageLeftDialogList => "message.left-dialog-list",
            Self::MessageUnreadSummary => "message.unread-summary",
            Self::MessageTimeline => "message.timeline",
            Self::MessageThreadDetail => "message.thread-detail",
            Self::MessageChannelSummary => "message.channel-summary",
            Self::MessageMemberRoster => "message.member-roster",
            Self::MessageTopicList => "message.topic-list",
            Self::MessageParticipantReceipts => "message.participant-receipts",
        }
    }
}

impl Serialize for SurfaceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SurfaceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "message.left-dialog-list" => Ok(Self::MessageLeftDialogList),
            "message.unread-summary" => Ok(Self::MessageUnreadSummary),
            "message.timeline" => Ok(Self::MessageTimeline),
            "message.thread-detail" => Ok(Self::MessageThreadDetail),
            "message.channel-summary" => Ok(Self::MessageChannelSummary),
            "message.member-roster" => Ok(Self::MessageMemberRoster),
            "message.topic-list" => Ok(Self::MessageTopicList),
            "message.participant-receipts" => Ok(Self::MessageParticipantReceipts),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &[
                    "message.left-dialog-list",
                    "message.unread-summary",
                    "message.timeline",
                    "message.thread-detail",
                    "message.channel-summary",
                    "message.member-roster",
                    "message.topic-list",
                    "message.participant-receipts",
                ],
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SurfaceScope {
    User {
        user_id: String,
        team_id: String,
    },
    Timeline {
        channel_id: String,
        window_token: String,
    },
    Thread {
        channel_id: String,
        root_message_id: String,
    },
    Channel {
        channel_id: String,
    },
    MemberRoster {
        channel_id: String,
        cursor: String,
    },
    TopicList {
        parent_channel_id: String,
        cursor: String,
    },
    ParticipantReceipts {
        channel_id: String,
        message_id: String,
        receipt_kind: ReceiptKind,
    },
}

impl SurfaceScope {
    pub fn canonical_key(&self) -> String {
        let mut key = String::new();
        match self {
            Self::User { user_id, team_id } => {
                push_scope_fields(&mut key, "user", &[user_id, team_id])
            }
            Self::Timeline {
                channel_id,
                window_token,
            } => push_scope_fields(&mut key, "timeline", &[channel_id, window_token]),
            Self::Thread {
                channel_id,
                root_message_id,
            } => push_scope_fields(&mut key, "thread", &[channel_id, root_message_id]),
            Self::Channel { channel_id } => push_scope_fields(&mut key, "channel", &[channel_id]),
            Self::MemberRoster { channel_id, cursor } => {
                push_scope_fields(&mut key, "memberRoster", &[channel_id, cursor])
            }
            Self::TopicList {
                parent_channel_id,
                cursor,
            } => push_scope_fields(&mut key, "topicList", &[parent_channel_id, cursor]),
            Self::ParticipantReceipts {
                channel_id,
                message_id,
                receipt_kind,
            } => push_scope_fields(
                &mut key,
                "participantReceipts",
                &[channel_id, message_id, receipt_kind.as_str()],
            ),
        }
        key
    }

    fn valid_for(&self, surface: SurfaceId) -> bool {
        match (surface, self) {
            (
                SurfaceId::MessageLeftDialogList | SurfaceId::MessageUnreadSummary,
                Self::User { user_id, team_id },
            ) => !user_id.is_empty() && !team_id.is_empty(),
            (
                SurfaceId::MessageTimeline,
                Self::Timeline {
                    channel_id,
                    window_token,
                },
            ) => !channel_id.is_empty() && !window_token.is_empty(),
            (
                SurfaceId::MessageThreadDetail,
                Self::Thread {
                    channel_id,
                    root_message_id,
                },
            ) => !channel_id.is_empty() && !root_message_id.is_empty(),
            (SurfaceId::MessageChannelSummary, Self::Channel { channel_id }) => {
                !channel_id.is_empty()
            }
            (SurfaceId::MessageMemberRoster, Self::MemberRoster { channel_id, cursor }) => {
                !channel_id.is_empty() && !cursor.is_empty()
            }
            (
                SurfaceId::MessageTopicList,
                Self::TopicList {
                    parent_channel_id,
                    cursor,
                },
            ) => !parent_channel_id.is_empty() && !cursor.is_empty(),
            (
                SurfaceId::MessageParticipantReceipts,
                Self::ParticipantReceipts {
                    channel_id,
                    message_id,
                    ..
                },
            ) => !channel_id.is_empty() && !message_id.is_empty(),
            _ => false,
        }
    }
}

fn push_scope_fields(out: &mut String, kind: &str, fields: &[&str]) {
    out.push_str(kind);
    for field in fields {
        out.push('|');
        out.push_str(&field.len().to_string());
        out.push(':');
        out.push_str(field);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptKind {
    Read,
    TemplateConfirmation,
    UrgentConfirmation,
}

impl ReceiptKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::TemplateConfirmation => "templateConfirmation",
            Self::UrgentConfirmation => "urgentConfirmation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionBinding {
    pub action_ref: String,
    pub kind: String,
    pub input_shape: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionInvocation {
    pub action_ref: String,
    pub input: BTreeMap<String, SurfaceDataVm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionReceipt {
    pub request_id: String,
    pub status: ActionReceiptStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_errors: Option<Vec<ActionValidationError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionReceiptStatus {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionValidationError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionFrame {
    pub schema_version: u32,
    pub transaction_id: String,
    pub mutations: Vec<SurfaceMutation>,
}

impl InteractionFrame {
    pub fn decode(bytes: &[u8]) -> Result<Self, InteractionFrameError> {
        let frame: Self = serde_json::from_slice(bytes)
            .map_err(|error| InteractionFrameError::Decode(error.to_string()))?;
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), InteractionFrameError> {
        if self.schema_version != INTERACTION_FRAME_SCHEMA_VERSION {
            return Err(InteractionFrameError::SchemaVersion(self.schema_version));
        }
        if self.transaction_id.is_empty() {
            return Err(InteractionFrameError::EmptyTransactionId);
        }
        if self.mutations.is_empty() {
            return Err(InteractionFrameError::EmptyMutations);
        }

        let mut slots = BTreeSet::new();
        for mutation in &self.mutations {
            mutation.validate()?;
            let slot = (mutation.surface, mutation.scope.canonical_key());
            if !slots.insert(slot.clone()) {
                return Err(InteractionFrameError::DuplicateSlot {
                    surface: slot.0.as_str(),
                    scope: slot.1,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceMutation {
    pub surface: SurfaceId,
    pub scope: SurfaceScope,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    pub mutation: TypedSurfaceMutation,
}

impl SurfaceMutation {
    fn validate(&self) -> Result<(), InteractionFrameError> {
        if !self.scope.valid_for(self.surface) {
            return Err(InteractionFrameError::ScopeMismatch {
                surface: self.surface.as_str(),
            });
        }
        if self.revision == 0 {
            return Err(InteractionFrameError::ZeroRevision {
                surface: self.surface.as_str(),
            });
        }
        if self.mutation.surface() != self.surface {
            return Err(InteractionFrameError::MutationMismatch {
                surface: self.surface.as_str(),
            });
        }
        if self.mutation.is_patch() {
            let Some(base_revision) = self.base_revision else {
                return Err(InteractionFrameError::MissingBaseRevision {
                    surface: self.surface.as_str(),
                });
            };
            if base_revision == 0 || self.revision <= base_revision {
                return Err(InteractionFrameError::InvalidPatchRevision {
                    surface: self.surface.as_str(),
                });
            }
        } else if self.base_revision.is_some() {
            return Err(InteractionFrameError::SnapshotHasBaseRevision {
                surface: self.surface.as_str(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InteractionFrameError {
    #[error("interaction frame JSON decode failed: {0}")]
    Decode(String),
    #[error("unsupported interaction frame schema version: {0}")]
    SchemaVersion(u32),
    #[error("interaction frame transactionId must not be empty")]
    EmptyTransactionId,
    #[error("interaction frame must contain at least one mutation")]
    EmptyMutations,
    #[error("surface {surface} has an invalid scope")]
    ScopeMismatch { surface: &'static str },
    #[error("surface {surface} has revision 0")]
    ZeroRevision { surface: &'static str },
    #[error("surface {surface} mutation kind does not match its surface")]
    MutationMismatch { surface: &'static str },
    #[error("surface {surface} patch has no baseRevision")]
    MissingBaseRevision { surface: &'static str },
    #[error("surface {surface} patch revision must advance its baseRevision")]
    InvalidPatchRevision { surface: &'static str },
    #[error("surface {surface} snapshot must not carry baseRevision")]
    SnapshotHasBaseRevision { surface: &'static str },
    #[error("duplicate surface slot {surface}@{scope}")]
    DuplicateSlot {
        surface: &'static str,
        scope: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceView<T> {
    pub kind: ReplaceViewKind,
    pub view: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplaceViewKind {
    ReplaceView,
}

// The concrete VMs intentionally carry render-ready business values rather than raw WS/DB rows.
// They are shared by desktop and mobile; user display data remains a Ucenter concern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserRefVm {
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelTitleRef {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelAvatarRef {
    pub resource: Option<String>,
    pub fallback_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SurfaceLoadState {
    Loading,
    Ready,
    Empty,
    Forbidden,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelKind {
    Direct,
    Group,
    Topic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationMode {
    All,
    Mentions,
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelLifecycleVm {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogSection {
    Pinned,
    Normal,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PreviewVm {
    Text {
        author: Option<UserRefVm>,
        text: String,
    },
    Document {
        author: Option<UserRefVm>,
        title: String,
    },
    Media {
        author: Option<UserRefVm>,
        media_kind: String,
        label: String,
    },
    Forwarded {
        author: Option<UserRefVm>,
        summary: String,
    },
    System {
        text: String,
    },
    Recalled {
        actor: Option<UserRefVm>,
        text: String,
    },
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogListVm {
    pub sections: Vec<DialogSectionVm>,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogSectionVm {
    pub kind: DialogSection,
    pub items: Vec<DialogRowVm>,
    pub page: WindowPageVm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogRowVm {
    pub channel_id: String,
    pub channel_kind: ChannelKind,
    pub title: ChannelTitleRef,
    pub avatar: ChannelAvatarRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_tag: Option<String>,
    pub preview: PreviewVm,
    pub last_activity_at: i64,
    pub unread_count: u32,
    pub mention_count: u32,
    pub urgent_count: u32,
    pub topic_count: u32,
    pub has_scheduled_message: bool,
    pub is_top: bool,
    pub notification_mode: NotificationMode,
    pub lifecycle: ChannelLifecycleVm,
    pub actions: Vec<ActionBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnreadSummaryVm {
    pub unread_total: u32,
    pub mention_total: u32,
    pub urgent_total: u32,
    pub unread_channel_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowPageVm {
    pub window_token: String,
    pub has_older: bool,
    pub has_newer: bool,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimelineAnchorMode {
    Latest,
    Locate,
    Unread,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineAnchorVm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub mode: TimelineAnchorMode,
}

/// 有界的递归展示资料，不允许 raw WS/HTTP payload 泄漏到 surface 合同。
///
/// 4.3/4.8 未细化的摘要值采用此类型；它只允许 JSON 的基础展示数据，不接受任意
/// `serde_json::Value` 作为领域旁路。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SurfaceDataVm {
    Null(()),
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<SurfaceDataVm>),
    Record(BTreeMap<String, SurfaceDataVm>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MessageContentVm {
    Text {
        text: String,
    },
    Document {
        document_id: String,
        title: String,
        summary: String,
        access: SurfaceDataVm,
    },
    Media {
        media_type: String,
        resource: SurfaceDataVm,
        thumbnail: Option<SurfaceDataVm>,
        file_name: String,
        size: u64,
        availability: SurfaceDataVm,
    },
    Forwarded {
        summary: String,
        items: Vec<SurfaceDataVm>,
    },
    SystemEvent {
        event_kind: String,
        text: String,
    },
    Recalled {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DeliveryVm {
    Uploading { progress_percent: u8 },
    Sending,
    Sent,
    Failed { reason: String, retryable: bool },
    Recalled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageVm {
    pub id: String,
    pub channel_id: String,
    pub author: UserRefVm,
    pub created_at: i64,
    pub content: MessageContentVm,
    pub delivery: DeliveryVm,
    pub actions: Vec<ActionBinding>,
    pub reply_summary: SurfaceDataVm,
    pub read_receipt: SurfaceDataVm,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_confirmation: Option<SurfaceDataVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub urgency: Option<SurfaceDataVm>,
    pub pinned: bool,
    pub bookmarked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_link: Option<SurfaceDataVm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineWindowVm {
    pub channel_id: String,
    pub anchor: TimelineAnchorVm,
    pub items: Vec<MessageVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_unread_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_message_id: Option<String>,
    pub page: WindowPageVm,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadDetailVm {
    pub channel_id: String,
    pub root: MessageVm,
    pub ordered_reply_ids: Vec<String>,
    pub nodes: BTreeMap<String, MessageVm>,
    pub reply_count: u32,
    pub page: WindowPageVm,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelSummaryVm {
    pub channel_id: String,
    pub title: ChannelTitleRef,
    pub avatar: ChannelAvatarRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub lifecycle: ChannelLifecycleVm,
    pub member_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online_count: Option<u32>,
    pub topic_count: u32,
    pub notification_mode: NotificationMode,
    pub actions: Vec<ActionBinding>,
    pub settings_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChannelRole {
    Owner,
    Admin,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresenceState {
    Online,
    Away,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MembershipState {
    Active,
    Left,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberRosterVm {
    pub channel_id: String,
    pub items: Vec<MemberRowVm>,
    pub page: WindowPageVm,
    pub member_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online_count: Option<u32>,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberRowVm {
    pub user: UserRefVm,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    pub role: ChannelRole,
    pub presence: PresenceState,
    pub membership_state: MembershipState,
    pub actions: Vec<ActionBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopicListVm {
    pub parent_channel_id: String,
    pub items: Vec<TopicRowVm>,
    pub page: WindowPageVm,
    pub topic_count: u32,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopicRowVm {
    pub topic_channel_id: String,
    pub root_message_id: String,
    pub title: ChannelTitleRef,
    pub root_message_preview: PreviewVm,
    pub last_activity_at: i64,
    pub message_count: u32,
    pub avatar: ChannelAvatarRef,
    pub lifecycle: ChannelLifecycleVm,
    pub actions: Vec<ActionBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticipantReceiptsVm {
    pub channel_id: String,
    pub message_id: String,
    pub kind: ReceiptKind,
    pub summary: SurfaceDataVm,
    pub completed: Vec<ParticipantReceiptRowVm>,
    pub pending: Vec<ParticipantReceiptRowVm>,
    pub page: WindowPageVm,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptStatus {
    Pending,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticipantReceiptRowVm {
    pub user: UserRefVm,
    pub member_role: ChannelRole,
    pub status: ReceiptStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum TypedSurfaceMutation {
    DialogListSnapshot {
        view: DialogListVm,
    },
    DialogListPatch {
        patch: ReplaceView<DialogListVm>,
    },
    UnreadSummarySnapshot {
        view: UnreadSummaryVm,
    },
    UnreadSummaryPatch {
        patch: ReplaceView<UnreadSummaryVm>,
    },
    TimelineSnapshot {
        view: TimelineWindowVm,
    },
    TimelinePatch {
        patch: ReplaceView<TimelineWindowVm>,
    },
    ThreadDetailSnapshot {
        view: ThreadDetailVm,
    },
    ThreadDetailPatch {
        patch: ReplaceView<ThreadDetailVm>,
    },
    ChannelSummarySnapshot {
        view: ChannelSummaryVm,
    },
    ChannelSummaryPatch {
        patch: ReplaceView<ChannelSummaryVm>,
    },
    MemberRosterSnapshot {
        view: MemberRosterVm,
    },
    MemberRosterPatch {
        patch: ReplaceView<MemberRosterVm>,
    },
    TopicListSnapshot {
        view: TopicListVm,
    },
    TopicListPatch {
        patch: ReplaceView<TopicListVm>,
    },
    ParticipantReceiptsSnapshot {
        view: ParticipantReceiptsVm,
    },
    ParticipantReceiptsPatch {
        patch: ReplaceView<ParticipantReceiptsVm>,
    },
}

impl TypedSurfaceMutation {
    pub const fn surface(&self) -> SurfaceId {
        match self {
            Self::DialogListSnapshot { .. } | Self::DialogListPatch { .. } => {
                SurfaceId::MessageLeftDialogList
            }
            Self::UnreadSummarySnapshot { .. } | Self::UnreadSummaryPatch { .. } => {
                SurfaceId::MessageUnreadSummary
            }
            Self::TimelineSnapshot { .. } | Self::TimelinePatch { .. } => {
                SurfaceId::MessageTimeline
            }
            Self::ThreadDetailSnapshot { .. } | Self::ThreadDetailPatch { .. } => {
                SurfaceId::MessageThreadDetail
            }
            Self::ChannelSummarySnapshot { .. } | Self::ChannelSummaryPatch { .. } => {
                SurfaceId::MessageChannelSummary
            }
            Self::MemberRosterSnapshot { .. } | Self::MemberRosterPatch { .. } => {
                SurfaceId::MessageMemberRoster
            }
            Self::TopicListSnapshot { .. } | Self::TopicListPatch { .. } => {
                SurfaceId::MessageTopicList
            }
            Self::ParticipantReceiptsSnapshot { .. } | Self::ParticipantReceiptsPatch { .. } => {
                SurfaceId::MessageParticipantReceipts
            }
        }
    }
    pub const fn is_patch(&self) -> bool {
        matches!(
            self,
            Self::DialogListPatch { .. }
                | Self::UnreadSummaryPatch { .. }
                | Self::TimelinePatch { .. }
                | Self::ThreadDetailPatch { .. }
                | Self::ChannelSummaryPatch { .. }
                | Self::MemberRosterPatch { .. }
                | Self::TopicListPatch { .. }
                | Self::ParticipantReceiptsPatch { .. }
        )
    }
}
