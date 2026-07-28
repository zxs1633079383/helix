//! UC01-16 的跨端、版本化 InteractionFrame 合同。
//!
//! 该模块只定义 IM 的语义投影契约和校验规则；不依赖 renderer、driver 或运行时。平台端
//! 以 `(surface, canonical_scope)` 建 slot，整帧校验成功后才机械提交。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const INTERACTION_FRAME_SCHEMA_VERSION: u32 = 2;
pub const INTERACTION_FRAME_EVENT: &str = "im:interaction:frame";
pub const FRAME_APPLIED_COMMAND: &str = "__helix_frame_applied";
/// Action invoke 的受理结果。`accepted` 只代表 Helix 已接管该 requestId；业务终态仍必须等同
/// requestId 的权威 Frame causation。
pub const INTERACTION_ACTION_RECEIPT_EVENT: &str = "im:interaction:action-receipt";
/// 上行 ActionBinding invoke 的独立版本；不能借用下行 Frame 的版本号来默许未知字段。
pub const ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION: u32 = 1;
/// Wire integers must remain lossless when consumed as JavaScript `number` values.
pub const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
pub const MIN_SAFE_JSON_INTEGER: i64 = -9_007_199_254_740_991;
const MAX_SURFACE_DATA_DEPTH: usize = 8;
const MAX_SURFACE_DATA_ITEMS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SurfaceId {
    MessageLeftDialogList,
    MessageUnreadSummary,
    MessageTimeline,
    MessageThreadDetail,
    MessageChannelSummary,
    MessageComposer,
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
            Self::MessageComposer => "message.composer",
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
            "message.composer" => Ok(Self::MessageComposer),
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
                    "message.composer",
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

    /// 验证 scope 是否与公开 surface 合同精确匹配。
    ///
    /// 上行 action envelope 和下行 mutation 都复用这条规则，避免某个端只校验 scope 的字段非空，
    /// 却把 thread/channel 等上下文错投给不相容的 surface。
    pub fn valid_for(&self, surface: SurfaceId) -> bool {
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
            (
                SurfaceId::MessageChannelSummary | SurfaceId::MessageComposer,
                Self::Channel { channel_id },
            ) => !channel_id.is_empty(),
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
    /// renderer 回送 envelope 时唯一可用的 epoch 真源；不得由 UI 从 revision 或本地计数猜测。
    pub surface_epoch: u64,
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

/// Renderer-to-core acknowledgement emitted only after a complete V2 frame
/// has been committed and its directives have been attempted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameApplied {
    pub transaction_id: String,
    pub applied_slots: u32,
    pub result: FrameAppliedResult,
}

impl FrameApplied {
    pub fn from_payload(payload: &[u8]) -> Result<Self, InteractionFrameError> {
        let ack: Self = serde_json::from_slice(payload)
            .map_err(|error| InteractionFrameError::Decode(error.to_string()))?;
        if ack.transaction_id.is_empty() {
            return Err(InteractionFrameError::EmptyTransactionId);
        }
        Ok(ack)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameAppliedResult {
    Applied,
    DirectiveFailed,
}

/// renderer 回送给 Helix 的版本化、surface-scoped 动作请求。
///
/// `request_id` 是端到端 causation key；`actor_id` 由受信 platform runtime 注入，而不是由 UI
/// 自行推导。`surface_epoch` 与 `observed_revision` 让旧页面/旧 Frame 的 action binding 在进入
/// 命令映射前 fail closed。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionInvocationEnvelope {
    pub schema_version: u32,
    pub request_id: String,
    pub actor_id: String,
    pub surface: SurfaceId,
    pub scope: SurfaceScope,
    pub surface_epoch: u64,
    pub observed_revision: u64,
    pub invocation: ActionInvocation,
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

impl ActionBinding {
    pub fn validate(&self) -> Result<(), InteractionFrameError> {
        if self.action_ref.is_empty() {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "actionRef must not be empty".to_string(),
            });
        }
        if self.kind.is_empty() {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "kind must not be empty".to_string(),
            });
        }
        if self.input_shape.is_empty() {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "inputShape must not be empty".to_string(),
            });
        }
        if self.surface_epoch == 0 {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "surfaceEpoch must be greater than zero".to_string(),
            });
        }
        if self.surface_epoch > MAX_SAFE_JSON_INTEGER {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "surfaceEpoch exceeds the v1 exact-safe wire range".to_string(),
            });
        }
        if self.enabled && self.disabled_reason.is_some() {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "enabled action must not carry disabledReason".to_string(),
            });
        }
        if self.disabled_reason.as_deref().is_some_and(str::is_empty) {
            return Err(InteractionFrameError::InvalidActionBinding {
                reason: "disabledReason must not be empty when present".to_string(),
            });
        }
        Ok(())
    }
}

impl ActionInvocation {
    pub fn validate(&self) -> Result<(), InteractionFrameError> {
        if self.action_ref.is_empty() {
            return Err(InteractionFrameError::InvalidActionInvocation {
                reason: "actionRef must not be empty".to_string(),
            });
        }
        if self.input.len() > MAX_SURFACE_DATA_ITEMS {
            return Err(InteractionFrameError::InvalidActionInvocation {
                reason: "input exceeds the v1 bounded contract".to_string(),
            });
        }
        for (key, value) in &self.input {
            if key.is_empty() {
                return Err(InteractionFrameError::InvalidActionInvocation {
                    reason: "input field must not be empty".to_string(),
                });
            }
            validate_surface_data(value, 0)
                .map_err(|reason| InteractionFrameError::InvalidActionInvocation { reason })?;
        }
        Ok(())
    }
}

impl ActionInvocationEnvelope {
    pub fn validate(&self) -> Result<(), InteractionFrameError> {
        if self.schema_version != ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION {
            return Err(InteractionFrameError::ActionInvocationSchemaVersion(
                self.schema_version,
            ));
        }
        if self.request_id.is_empty() {
            return Err(InteractionFrameError::InvalidActionInvocationEnvelope {
                reason: "requestId must not be empty".to_string(),
            });
        }
        if self.actor_id.is_empty() {
            return Err(InteractionFrameError::InvalidActionInvocationEnvelope {
                reason: "actorId must not be empty".to_string(),
            });
        }
        if !self.scope.valid_for(self.surface) {
            return Err(InteractionFrameError::InvalidActionInvocationEnvelope {
                reason: "scope does not match surface".to_string(),
            });
        }
        validate_wire_u64(self.surface, "surfaceEpoch", self.surface_epoch)?;
        if self.surface_epoch == 0 {
            return Err(InteractionFrameError::InvalidActionInvocationEnvelope {
                reason: "surfaceEpoch must be greater than zero".to_string(),
            });
        }
        validate_wire_u64(self.surface, "observedRevision", self.observed_revision)?;
        if self.observed_revision == 0 {
            return Err(InteractionFrameError::InvalidActionInvocationEnvelope {
                reason: "observedRevision must be greater than zero".to_string(),
            });
        }
        self.invocation.validate()
    }
}

impl ActionReceipt {
    pub fn validate(&self) -> Result<(), InteractionFrameError> {
        if self.request_id.is_empty() {
            return Err(InteractionFrameError::InvalidActionReceipt {
                reason: "requestId must not be empty".to_string(),
            });
        }
        match self.status {
            ActionReceiptStatus::Accepted if self.validation_errors.is_some() => {
                Err(InteractionFrameError::InvalidActionReceipt {
                    reason: "accepted receipt must not carry validationErrors".to_string(),
                })
            }
            ActionReceiptStatus::Rejected => {
                let Some(errors) = self.validation_errors.as_ref() else {
                    return Err(InteractionFrameError::InvalidActionReceipt {
                        reason: "rejected receipt requires validationErrors".to_string(),
                    });
                };
                if errors.is_empty()
                    || errors
                        .iter()
                        .any(|error| error.field.is_empty() || error.message.is_empty())
                {
                    return Err(InteractionFrameError::InvalidActionReceipt {
                        reason: "validationErrors must contain field and message".to_string(),
                    });
                }
                Ok(())
            }
            ActionReceiptStatus::Accepted => Ok(()),
        }
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
    pub(crate) fn validate(&self) -> Result<(), InteractionFrameError> {
        if !self.scope.valid_for(self.surface) {
            return Err(InteractionFrameError::ScopeMismatch {
                surface: self.surface.as_str(),
            });
        }
        validate_wire_u64(self.surface, "revision", self.revision)?;
        if let Some(base_revision) = self.base_revision {
            validate_wire_u64(self.surface, "baseRevision", base_revision)?;
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
        if self.causation_id.as_deref().is_some_and(str::is_empty) {
            return Err(InteractionFrameError::EmptyCausationId {
                surface: self.surface.as_str(),
            });
        }
        self.mutation.validate_view(&self.scope)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionFrameV2 {
    pub schema_version: u32,
    pub transaction_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    pub runtime: InteractionRuntimeVm,
    pub mutations: Vec<SurfaceMutationV2>,
    #[serde(default)]
    pub directives: Vec<UiDirective>,
}

impl InteractionFrameV2 {
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
        if self.causation_id.as_deref().is_some_and(str::is_empty) {
            return Err(InteractionFrameError::EmptyFrameCausationId);
        }
        if self.mutations.is_empty() {
            return Err(InteractionFrameError::EmptyMutations);
        }
        self.runtime.validate()?;
        let mut slots = BTreeSet::new();
        for mutation in &self.mutations {
            mutation.validate()?;
            let slot = (
                mutation.mutation.surface,
                mutation.mutation.scope.canonical_key(),
            );
            if !slots.insert(slot.clone()) {
                return Err(InteractionFrameError::DuplicateSlot {
                    surface: slot.0.as_str(),
                    scope: slot.1,
                });
            }
        }
        for directive in &self.directives {
            directive.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceMutationV2 {
    #[serde(flatten)]
    pub mutation: SurfaceMutation,
    pub surface_epoch: u64,
}

impl SurfaceMutationV2 {
    fn validate(&self) -> Result<(), InteractionFrameError> {
        self.mutation.validate()?;
        validate_wire_u64(self.mutation.surface, "surfaceEpoch", self.surface_epoch)?;
        if self.surface_epoch == 0 {
            return Err(InteractionFrameError::ZeroSurfaceEpoch {
                surface: self.mutation.surface.as_str(),
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
    #[error("unsupported action invocation schema version: {0}")]
    ActionInvocationSchemaVersion(u32),
    #[error("interaction frame transactionId must not be empty")]
    EmptyTransactionId,
    #[error("interaction frame must contain at least one mutation")]
    EmptyMutations,
    #[error("interaction frame causationId must not be empty")]
    EmptyFrameCausationId,
    #[error("schema v2 interaction frame requires runtime")]
    MissingV2Runtime,
    #[error("interaction frame mutation requires surfaceEpoch for surface {surface}")]
    MissingSurfaceEpoch { surface: &'static str },
    #[error("surface {surface} has surfaceEpoch 0")]
    ZeroSurfaceEpoch { surface: &'static str },
    #[error("surface {surface} has an invalid scope")]
    ScopeMismatch { surface: &'static str },
    #[error("surface {surface} has revision 0")]
    ZeroRevision { surface: &'static str },
    #[error("surface {surface} field {field} exceeds the JavaScript exact-safe numeric range")]
    UnsafeWireNumber {
        surface: &'static str,
        field: &'static str,
    },
    #[error("surface {surface} mutation kind does not match its surface")]
    MutationMismatch { surface: &'static str },
    #[error("surface {surface} patch has no baseRevision")]
    MissingBaseRevision { surface: &'static str },
    #[error("surface {surface} patch revision must advance its baseRevision")]
    InvalidPatchRevision { surface: &'static str },
    #[error("surface {surface} snapshot must not carry baseRevision")]
    SnapshotHasBaseRevision { surface: &'static str },
    #[error("surface {surface} causationId must not be empty when present")]
    EmptyCausationId { surface: &'static str },
    #[error("surface {surface} has an invalid typed view: {reason}")]
    InvalidView {
        surface: &'static str,
        reason: String,
    },
    #[error("invalid ActionBinding: {reason}")]
    InvalidActionBinding { reason: String },
    #[error("invalid ActionInvocation: {reason}")]
    InvalidActionInvocation { reason: String },
    #[error("invalid ActionInvocationEnvelope: {reason}")]
    InvalidActionInvocationEnvelope { reason: String },
    #[error("invalid ActionReceipt: {reason}")]
    InvalidActionReceipt { reason: String },
    #[error("invalid runtime: {reason}")]
    InvalidRuntime { reason: String },
    #[error("invalid UI directive: {reason}")]
    InvalidDirective { reason: String },
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
    #[serde(default)]
    pub user_ids: Vec<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_message_id: Option<String>,
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
    pub member_count: u32,
    pub member_leaves: Vec<String>,
    pub has_scheduled_message: bool,
    pub is_top: bool,
    pub notification_mode: NotificationMode,
    pub lifecycle: ChannelLifecycleVm,
    pub settings_version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
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
/// `serde_json::Value` 作为领域旁路。整数必须在 JavaScript exact-safe 范围内；小数以
/// 有限 IEEE-754 binary64 语义传输，要求无损十进制的值由调用方编码为字符串。
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaItemVm {
    pub file_id: String,
    pub file_name: String,
    pub size: u64,
    pub content_type: String,
    pub bucket: String,
    pub uri: String,
    pub resource: SurfaceDataVm,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<SurfaceDataVm>,
    pub availability: SurfaceDataVm,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<MediaItemVm>,
        #[serde(skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        image_position: Option<String>,
    },
    Forwarded {
        summary: String,
        items: Vec<SurfaceDataVm>,
    },
    SystemEvent {
        event_kind: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        member_id: Option<String>,
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
pub enum ReferenceAvailabilityVm {
    Available,
    Recalled { text: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ReplyPreviewVm {
    Text {
        text: String,
    },
    Document {
        title: String,
    },
    Media {
        media_type: String,
        file_name: String,
    },
    Forwarded {
        summary: String,
    },
    System {
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
pub enum ReplySummaryVm {
    None,
    Available {
        target_message_id: String,
        author: UserRefVm,
        preview: ReplyPreviewVm,
        availability: ReferenceAvailabilityVm,
    },
    Unavailable {
        target_message_id: String,
        availability: ReferenceAvailabilityVm,
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
    #[serde(default)]
    pub event_seq: u64,
    pub author: UserRefVm,
    pub created_at: i64,
    pub content: MessageContentVm,
    pub delivery: DeliveryVm,
    pub actions: Vec<ActionBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick_actions: Vec<MessageQuickActionVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_root_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_first_level_id: Option<String>,
    pub reply_summary: ReplySummaryVm,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_thread: Option<ReplyThreadSummaryVm>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<MessageReactionVm>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageQuickActionVm {
    React,
    QuoteReply,
    CreateTopic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplyThreadPreviewVm {
    pub id: String,
    pub author: UserRefVm,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplyThreadSummaryVm {
    pub items: Vec<ReplyThreadPreviewVm>,
    pub reply_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageReactionVm {
    pub emoji: String,
    pub count: u32,
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
    pub detail_context: DetailContextVm,
    pub root: MessageVm,
    pub ordered_reply_ids: Vec<String>,
    pub nodes: BTreeMap<String, MessageVm>,
    pub reply_count: u32,
    pub page: WindowPageVm,
    pub state: SurfaceLoadState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DetailContextVm {
    Reply,
    Topic {
        parent_channel_id: String,
        topic_channel_id: String,
        root_message_id: String,
        title: String,
    },
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
    pub viewer_role: ChannelRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online_count: Option<u32>,
    pub topic_count: u32,
    pub notification_mode: NotificationMode,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_scheduled_message: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_owner_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_message_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_send_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_status: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub schedule_revision: u64,
    pub actions: Vec<ActionBinding>,
    pub settings_version: u64,
}

/// 仅在数值为零时省略新增的向后兼容 projection 字段。
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// 仅在布尔值为 false 时省略新增的向后兼容 projection 字段。
fn is_false(value: &bool) -> bool {
    !*value
}

/// Runtime belongs to the same atomic frame publication as the changed surfaces. It is not a
/// renderer-derived sync state and must never be delivered as a raw event fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionRuntimeVm {
    pub connection: RuntimeConnectionState,
    pub initial_sync: RuntimeSyncState,
    pub recovery: RuntimeRecoveryState,
}

impl InteractionRuntimeVm {
    fn validate(&self) -> Result<(), InteractionFrameError> {
        if matches!(self.recovery, RuntimeRecoveryState::Recovered)
            && !matches!(self.connection, RuntimeConnectionState::Online)
        {
            return Err(InteractionFrameError::InvalidRuntime {
                reason: "recovered runtime requires an online connection".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeConnectionState {
    Connecting,
    Online,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeSyncState {
    Idle,
    Syncing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeRecoveryState {
    Idle,
    Syncing,
    Recovered,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiDirective {
    pub directive_id: String,
    pub kind: String,
}

impl UiDirective {
    fn validate(&self) -> Result<(), InteractionFrameError> {
        if self.directive_id.is_empty() || self.kind.is_empty() {
            return Err(InteractionFrameError::InvalidDirective {
                reason: "directiveId and kind must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComposerState {
    Hydrating,
    Ready,
    Forbidden,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComposerVm {
    pub channel_id: String,
    pub state: ComposerState,
    pub draft_policy: String,
    pub actions: Vec<ActionBinding>,
    pub capability_epoch: u64,
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
    ComposerSnapshot {
        view: ComposerVm,
    },
    ComposerPatch {
        patch: ReplaceView<ComposerVm>,
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
            Self::ComposerSnapshot { .. } | Self::ComposerPatch { .. } => {
                SurfaceId::MessageComposer
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
                | Self::ComposerPatch { .. }
                | Self::MemberRosterPatch { .. }
                | Self::TopicListPatch { .. }
                | Self::ParticipantReceiptsPatch { .. }
        )
    }

    fn validate_view(&self, scope: &SurfaceScope) -> Result<(), InteractionFrameError> {
        match self {
            Self::DialogListSnapshot { view } => validate_dialog_list(view),
            Self::DialogListPatch { patch } => validate_dialog_list(&patch.view),
            Self::UnreadSummarySnapshot { .. } | Self::UnreadSummaryPatch { .. } => Ok(()),
            Self::TimelineSnapshot { view } => validate_timeline(view, scope),
            Self::TimelinePatch { patch } => validate_timeline(&patch.view, scope),
            Self::ThreadDetailSnapshot { view } => validate_thread_detail(view, scope),
            Self::ThreadDetailPatch { patch } => validate_thread_detail(&patch.view, scope),
            Self::ChannelSummarySnapshot { view } => validate_channel_summary(view, scope),
            Self::ChannelSummaryPatch { patch } => validate_channel_summary(&patch.view, scope),
            Self::ComposerSnapshot { view } => validate_composer(view, scope),
            Self::ComposerPatch { patch } => validate_composer(&patch.view, scope),
            Self::MemberRosterSnapshot { view } => validate_member_roster(view, scope),
            Self::MemberRosterPatch { patch } => validate_member_roster(&patch.view, scope),
            Self::TopicListSnapshot { view } => validate_topic_list(view, scope),
            Self::TopicListPatch { patch } => validate_topic_list(&patch.view, scope),
            Self::ParticipantReceiptsSnapshot { view } => {
                validate_participant_receipts(view, scope)
            }
            Self::ParticipantReceiptsPatch { patch } => {
                validate_participant_receipts(&patch.view, scope)
            }
        }
    }
}

fn validate_dialog_list(view: &DialogListVm) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageLeftDialogList;
    let mut channel_ids = BTreeSet::new();
    for section in &view.sections {
        validate_page(surface, &section.page)?;
        for row in &section.items {
            require_nonempty(surface, "dialog row channelId", &row.channel_id)?;
            if !channel_ids.insert(row.channel_id.as_str()) {
                return invalid_view(surface, "dialog row channelId must be unique");
            }
            validate_wire_i64(surface, "lastActivityAt", row.last_activity_at)?;
            validate_lifecycle(surface, &row.lifecycle)?;
            validate_action_bindings(surface, &row.actions)?;
            validate_preview(surface, &row.preview)?;
        }
    }
    Ok(())
}

fn validate_timeline(
    view: &TimelineWindowVm,
    scope: &SurfaceScope,
) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageTimeline;
    let SurfaceScope::Timeline {
        channel_id,
        window_token,
    } = scope
    else {
        return invalid_view(surface, "timeline requires a timeline scope");
    };
    if view.channel_id != *channel_id {
        return invalid_view(surface, "view channelId must match scope channelId");
    }
    validate_page(surface, &view.page)?;
    if view.page.window_token != *window_token {
        return invalid_view(surface, "page windowToken must match scope windowToken");
    }
    validate_optional_id(
        surface,
        "anchor messageId",
        view.anchor.message_id.as_deref(),
    )?;
    validate_optional_id(
        surface,
        "firstUnreadMessageId",
        view.first_unread_message_id.as_deref(),
    )?;
    validate_optional_id(
        surface,
        "targetMessageId",
        view.target_message_id.as_deref(),
    )?;
    let mut ids = BTreeSet::new();
    for message in &view.items {
        validate_message(surface, message, channel_id)?;
        if !ids.insert(message.id.as_str()) {
            return invalid_view(surface, "timeline message id must be unique");
        }
    }
    Ok(())
}

fn validate_thread_detail(
    view: &ThreadDetailVm,
    scope: &SurfaceScope,
) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageThreadDetail;
    let SurfaceScope::Thread {
        channel_id,
        root_message_id,
    } = scope
    else {
        return invalid_view(surface, "thread requires a thread scope");
    };
    if view.channel_id != *channel_id || view.root.id != *root_message_id {
        return invalid_view(surface, "thread root and channel must match scope");
    }
    if let DetailContextVm::Topic {
        parent_channel_id,
        topic_channel_id,
        root_message_id: context_root_id,
        title,
    } = &view.detail_context
    {
        if parent_channel_id.is_empty()
            || topic_channel_id != channel_id
            || context_root_id != root_message_id
            || title.is_empty()
        {
            return invalid_view(surface, "topic detail context must match thread scope");
        }
    }
    validate_page(surface, &view.page)?;
    validate_message(surface, &view.root, channel_id)?;
    let mut ordered = BTreeSet::new();
    for reply_id in &view.ordered_reply_ids {
        require_nonempty(surface, "ordered reply id", reply_id)?;
        if reply_id == root_message_id || !ordered.insert(reply_id.as_str()) {
            return invalid_view(surface, "ordered reply ids must be unique non-root nodes");
        }
        if !view.nodes.contains_key(reply_id) {
            return invalid_view(surface, "ordered reply id must exist in nodes");
        }
    }
    for (node_id, node) in &view.nodes {
        require_nonempty(surface, "thread node id", node_id)?;
        if node.id != *node_id || node_id == root_message_id || !ordered.contains(node_id.as_str())
        {
            return invalid_view(surface, "thread nodes must exactly match ordered reply ids");
        }
        validate_message(surface, node, channel_id)?;
    }
    if view.reply_count < view.ordered_reply_ids.len() as u32 {
        return invalid_view(
            surface,
            "replyCount cannot be smaller than visible reply ids",
        );
    }
    Ok(())
}

fn validate_channel_summary(
    view: &ChannelSummaryVm,
    scope: &SurfaceScope,
) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageChannelSummary;
    let SurfaceScope::Channel { channel_id } = scope else {
        return invalid_view(surface, "channel summary requires a channel scope");
    };
    if view.channel_id != *channel_id {
        return invalid_view(surface, "view channelId must match scope channelId");
    }
    validate_wire_u64(surface, "settingsVersion", view.settings_version)?;
    validate_lifecycle(surface, &view.lifecycle)?;
    validate_action_bindings(surface, &view.actions)
}

fn validate_composer(view: &ComposerVm, scope: &SurfaceScope) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageComposer;
    let SurfaceScope::Channel { channel_id } = scope else {
        return invalid_view(surface, "composer requires a channel scope");
    };
    if view.channel_id != *channel_id || view.draft_policy.is_empty() || view.capability_epoch == 0
    {
        return invalid_view(
            surface,
            "composer channelId, draftPolicy and capabilityEpoch must be valid",
        );
    }
    validate_wire_u64(surface, "capabilityEpoch", view.capability_epoch)?;
    validate_action_bindings(surface, &view.actions)?;
    if view
        .actions
        .iter()
        .any(|binding| binding.surface_epoch != view.capability_epoch)
    {
        return invalid_view(surface, "composer actions must use capabilityEpoch");
    }
    if !matches!(view.state, ComposerState::Ready)
        && view.actions.iter().any(|binding| binding.enabled)
    {
        return invalid_view(surface, "non-ready composer must not enable actions");
    }
    Ok(())
}

fn validate_member_roster(
    view: &MemberRosterVm,
    scope: &SurfaceScope,
) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageMemberRoster;
    let SurfaceScope::MemberRoster { channel_id, cursor } = scope else {
        return invalid_view(surface, "member roster requires a memberRoster scope");
    };
    if view.channel_id != *channel_id {
        return invalid_view(surface, "view channelId must match scope channelId");
    }
    validate_page(surface, &view.page)?;
    if view.page.window_token != *cursor {
        return invalid_view(surface, "page windowToken must match scope cursor");
    }
    if view.member_count < view.items.len() as u32 {
        return invalid_view(surface, "memberCount cannot be smaller than visible items");
    }
    let mut users = BTreeSet::new();
    for member in &view.items {
        validate_user(surface, &member.user)?;
        if !users.insert(member.user.user_id.as_str()) {
            return invalid_view(surface, "member roster users must be unique");
        }
        validate_action_bindings(surface, &member.actions)?;
    }
    Ok(())
}

fn validate_topic_list(
    view: &TopicListVm,
    scope: &SurfaceScope,
) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageTopicList;
    let SurfaceScope::TopicList {
        parent_channel_id,
        cursor,
    } = scope
    else {
        return invalid_view(surface, "topic list requires a topicList scope");
    };
    if view.parent_channel_id != *parent_channel_id {
        return invalid_view(surface, "parentChannelId must match scope");
    }
    validate_page(surface, &view.page)?;
    if view.page.window_token != *cursor {
        return invalid_view(surface, "page windowToken must match scope cursor");
    }
    if view.topic_count < view.items.len() as u32 {
        return invalid_view(surface, "topicCount cannot be smaller than visible items");
    }
    let mut topic_ids = BTreeSet::new();
    for topic in &view.items {
        require_nonempty(surface, "topicChannelId", &topic.topic_channel_id)?;
        require_nonempty(surface, "rootMessageId", &topic.root_message_id)?;
        if !topic_ids.insert(topic.topic_channel_id.as_str()) {
            return invalid_view(surface, "topic channel ids must be unique");
        }
        validate_wire_i64(surface, "lastActivityAt", topic.last_activity_at)?;
        validate_preview(surface, &topic.root_message_preview)?;
        validate_lifecycle(surface, &topic.lifecycle)?;
        validate_action_bindings(surface, &topic.actions)?;
    }
    Ok(())
}

fn validate_participant_receipts(
    view: &ParticipantReceiptsVm,
    scope: &SurfaceScope,
) -> Result<(), InteractionFrameError> {
    let surface = SurfaceId::MessageParticipantReceipts;
    let SurfaceScope::ParticipantReceipts {
        channel_id,
        message_id,
        receipt_kind,
    } = scope
    else {
        return invalid_view(
            surface,
            "participant receipts require a participantReceipts scope",
        );
    };
    if view.channel_id != *channel_id
        || view.message_id != *message_id
        || view.kind != *receipt_kind
    {
        return invalid_view(
            surface,
            "receipt channel, message and kind must match scope",
        );
    }
    validate_page(surface, &view.page)?;
    validate_data_for_surface(surface, &view.summary)?;
    let mut users = BTreeSet::new();
    for row in &view.completed {
        validate_user(surface, &row.user)?;
        if row.status != ReceiptStatus::Completed {
            return invalid_view(surface, "completed receipt rows require completed status");
        }
        if let Some(completed_at) = row.completed_at {
            validate_wire_i64(surface, "completedAt", completed_at)?;
        } else if *receipt_kind != ReceiptKind::Read {
            return invalid_view(
                surface,
                "completed receipt rows without timestamps are only valid for read receipts",
            );
        }
        // `post/read/list` authoritatively exposes readBits but no per-recipient read instant. For read receipts
        // the omitted optional field means "timestamp unknown", never a fabricated observation time.
        if !users.insert(row.user.user_id.as_str()) {
            return invalid_view(
                surface,
                "receipt users must be unique across completed and pending",
            );
        }
    }
    for row in &view.pending {
        validate_user(surface, &row.user)?;
        if row.status != ReceiptStatus::Pending || row.completed_at.is_some() {
            return invalid_view(
                surface,
                "pending receipt rows must not carry completed timestamp",
            );
        }
        if !users.insert(row.user.user_id.as_str()) {
            return invalid_view(
                surface,
                "receipt users must be unique across completed and pending",
            );
        }
    }
    Ok(())
}

fn validate_message(
    surface: SurfaceId,
    message: &MessageVm,
    expected_channel_id: &str,
) -> Result<(), InteractionFrameError> {
    require_nonempty(surface, "message id", &message.id)?;
    require_nonempty(surface, "message channelId", &message.channel_id)?;
    if message.channel_id != expected_channel_id {
        return invalid_view(surface, "message channelId must match surface channel");
    }
    validate_wire_i64(surface, "createdAt", message.created_at)?;
    validate_user(surface, &message.author)?;
    validate_action_bindings(surface, &message.actions)?;
    for (field, value) in [
        ("replyId", message.reply_id.as_deref()),
        ("replyRootId", message.reply_root_id.as_deref()),
        ("replyFirstLevelId", message.reply_first_level_id.as_deref()),
    ] {
        if value.is_some_and(str::is_empty) {
            return invalid_view(surface, format!("message {field} must not be empty"));
        }
    }
    if message.quick_actions.len() > 3
        || message
            .quick_actions
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != message.quick_actions.len()
    {
        return invalid_view(
            surface,
            "message quick actions must be unique and capped at three items",
        );
    }
    validate_reply_summary(surface, &message.reply_summary)?;
    if let Some(thread) = &message.reply_thread {
        if thread.items.len() > 3 || thread.reply_count < thread.items.len() as u32 {
            return invalid_view(
                surface,
                "reply thread preview must be capped at three items",
            );
        }
        for item in &thread.items {
            require_nonempty(surface, "reply preview id", &item.id)?;
            validate_user(surface, &item.author)?;
        }
    }
    for reaction in &message.reactions {
        require_nonempty(surface, "reaction emoji", &reaction.emoji)?;
        if reaction.count == 0 {
            return invalid_view(surface, "reaction count must be positive");
        }
    }
    validate_data_for_surface(surface, &message.read_receipt)?;
    if let Some(data) = &message.template_confirmation {
        validate_data_for_surface(surface, data)?;
    }
    if let Some(data) = &message.urgency {
        validate_data_for_surface(surface, data)?;
    }
    if let Some(data) = &message.topic_link {
        validate_data_for_surface(surface, data)?;
    }
    match &message.content {
        MessageContentVm::Document { access, .. } => validate_data_for_surface(surface, access),
        MessageContentVm::Media {
            resource,
            thumbnail,
            size,
            availability,
            items,
            image_position,
            ..
        } => {
            validate_wire_u64(surface, "size", *size)?;
            validate_data_for_surface(surface, resource)?;
            if let Some(data) = thumbnail {
                validate_data_for_surface(surface, data)?;
            }
            validate_data_for_surface(surface, availability)?;
            if let Some(position) = image_position {
                if !matches!(position.as_str(), "top" | "bottom") {
                    return invalid_view(surface, "image position must be top or bottom");
                }
            }
            for item in items {
                require_nonempty(surface, "media item file id", &item.file_id)?;
                require_nonempty(surface, "media item file name", &item.file_name)?;
                require_nonempty(surface, "media item content type", &item.content_type)?;
                require_nonempty(surface, "media item bucket", &item.bucket)?;
                require_nonempty(surface, "media item uri", &item.uri)?;
                validate_wire_u64(surface, "media item size", item.size)?;
                validate_data_for_surface(surface, &item.resource)?;
                if let Some(thumbnail) = &item.thumbnail {
                    validate_data_for_surface(surface, thumbnail)?;
                }
                validate_data_for_surface(surface, &item.availability)?;
            }
            Ok(())
        }
        MessageContentVm::Forwarded { items, .. } => {
            for item in items {
                validate_data_for_surface(surface, item)?;
            }
            Ok(())
        }
        MessageContentVm::Text { .. }
        | MessageContentVm::SystemEvent { .. }
        | MessageContentVm::Recalled { .. } => Ok(()),
    }
}

fn validate_reply_summary(
    surface: SurfaceId,
    summary: &ReplySummaryVm,
) -> Result<(), InteractionFrameError> {
    match summary {
        ReplySummaryVm::None => Ok(()),
        ReplySummaryVm::Available {
            target_message_id,
            author,
            preview,
            availability,
        } => {
            require_nonempty(surface, "reply target message id", target_message_id)?;
            validate_user(surface, author)?;
            if !matches!(availability, ReferenceAvailabilityVm::Available) {
                return invalid_view(
                    surface,
                    "available reply summary requires available reference state",
                );
            }
            match preview {
                ReplyPreviewVm::Text { text }
                | ReplyPreviewVm::System { text }
                | ReplyPreviewVm::Recalled { text } => {
                    require_nonempty(surface, "reply preview text", text)?;
                }
                ReplyPreviewVm::Document { title } => {
                    require_nonempty(surface, "reply preview document title", title)?;
                }
                ReplyPreviewVm::Media {
                    media_type,
                    file_name,
                } => {
                    require_nonempty(surface, "reply preview media type", media_type)?;
                    require_nonempty(surface, "reply preview file name", file_name)?;
                }
                ReplyPreviewVm::Forwarded { summary } => {
                    require_nonempty(surface, "reply preview forwarded summary", summary)?;
                }
            }
            Ok(())
        }
        ReplySummaryVm::Unavailable {
            target_message_id,
            availability,
        } => {
            require_nonempty(surface, "reply target message id", target_message_id)?;
            match availability {
                ReferenceAvailabilityVm::Recalled { text } => {
                    require_nonempty(surface, "recalled reply label", text)
                }
                ReferenceAvailabilityVm::Unavailable { reason } => {
                    require_nonempty(surface, "unavailable reply reason", reason)
                }
                ReferenceAvailabilityVm::Available => invalid_view(
                    surface,
                    "unavailable reply summary cannot carry available reference state",
                ),
            }
        }
    }
}

fn validate_preview(surface: SurfaceId, preview: &PreviewVm) -> Result<(), InteractionFrameError> {
    match preview {
        PreviewVm::Text { author, .. }
        | PreviewVm::Document { author, .. }
        | PreviewVm::Media { author, .. }
        | PreviewVm::Forwarded { author, .. } => {
            if let Some(author) = author {
                validate_user(surface, author)?;
            }
        }
        PreviewVm::Recalled { actor, .. } => {
            if let Some(actor) = actor {
                validate_user(surface, actor)?;
            }
        }
        PreviewVm::System { .. } | PreviewVm::Empty => {}
    }
    Ok(())
}

fn validate_action_bindings(
    surface: SurfaceId,
    bindings: &[ActionBinding],
) -> Result<(), InteractionFrameError> {
    let mut refs = BTreeSet::new();
    for binding in bindings {
        binding.validate().map_err(|error| match error {
            InteractionFrameError::InvalidActionBinding { reason } => {
                InteractionFrameError::InvalidView {
                    surface: surface.as_str(),
                    reason,
                }
            }
            unexpected => unexpected,
        })?;
        if !refs.insert(binding.action_ref.as_str()) {
            return invalid_view(surface, "actionRef must be unique within one view item");
        }
    }
    Ok(())
}

fn validate_page(surface: SurfaceId, page: &WindowPageVm) -> Result<(), InteractionFrameError> {
    require_nonempty(surface, "page windowToken", &page.window_token)
}

fn validate_lifecycle(
    surface: SurfaceId,
    lifecycle: &ChannelLifecycleVm,
) -> Result<(), InteractionFrameError> {
    require_nonempty(surface, "lifecycle state", &lifecycle.state)
}

fn validate_user(surface: SurfaceId, user: &UserRefVm) -> Result<(), InteractionFrameError> {
    require_nonempty(surface, "userId", &user.user_id)
}

fn validate_optional_id(
    surface: SurfaceId,
    label: &str,
    value: Option<&str>,
) -> Result<(), InteractionFrameError> {
    if value.is_some_and(str::is_empty) {
        return invalid_view(surface, format!("{label} must not be empty when present"));
    }
    Ok(())
}

fn require_nonempty(
    surface: SurfaceId,
    label: &str,
    value: &str,
) -> Result<(), InteractionFrameError> {
    if value.is_empty() {
        return invalid_view(surface, format!("{label} must not be empty"));
    }
    Ok(())
}

fn validate_data_for_surface(
    surface: SurfaceId,
    value: &SurfaceDataVm,
) -> Result<(), InteractionFrameError> {
    validate_surface_data(value, 0).map_err(|reason| InteractionFrameError::InvalidView {
        surface: surface.as_str(),
        reason,
    })
}

fn validate_wire_u64(
    surface: SurfaceId,
    field: &'static str,
    value: u64,
) -> Result<(), InteractionFrameError> {
    if value > MAX_SAFE_JSON_INTEGER {
        return Err(InteractionFrameError::UnsafeWireNumber {
            surface: surface.as_str(),
            field,
        });
    }
    Ok(())
}

fn validate_wire_i64(
    surface: SurfaceId,
    field: &'static str,
    value: i64,
) -> Result<(), InteractionFrameError> {
    if !(MIN_SAFE_JSON_INTEGER..=MAX_SAFE_JSON_INTEGER as i64).contains(&value) {
        return Err(InteractionFrameError::UnsafeWireNumber {
            surface: surface.as_str(),
            field,
        });
    }
    Ok(())
}

fn validate_surface_data(value: &SurfaceDataVm, depth: usize) -> Result<(), String> {
    if depth > MAX_SURFACE_DATA_DEPTH {
        return Err("surface data exceeds maximum nesting depth".to_string());
    }
    match value {
        SurfaceDataVm::Null(_) | SurfaceDataVm::Bool(_) | SurfaceDataVm::String(_) => Ok(()),
        SurfaceDataVm::Number(number) => validate_surface_data_number(number),
        SurfaceDataVm::Array(items) => {
            if items.len() > MAX_SURFACE_DATA_ITEMS {
                return Err("surface data array exceeds maximum item count".to_string());
            }
            for item in items {
                validate_surface_data(item, depth + 1)?;
            }
            Ok(())
        }
        SurfaceDataVm::Record(fields) => {
            if fields.len() > MAX_SURFACE_DATA_ITEMS {
                return Err("surface data record exceeds maximum field count".to_string());
            }
            for (key, value) in fields {
                if key.is_empty() {
                    return Err("surface data record key must not be empty".to_string());
                }
                validate_surface_data(value, depth + 1)?;
            }
            Ok(())
        }
    }
}

fn validate_surface_data_number(number: &serde_json::Number) -> Result<(), String> {
    if let Some(value) = number.as_u64() {
        if value > MAX_SAFE_JSON_INTEGER {
            return Err("surface data integer exceeds the v1 exact-safe numeric range".to_string());
        }
        return Ok(());
    }
    if let Some(value) = number.as_i64() {
        if !(MIN_SAFE_JSON_INTEGER..=MAX_SAFE_JSON_INTEGER as i64).contains(&value) {
            return Err("surface data integer exceeds the v1 exact-safe numeric range".to_string());
        }
        return Ok(());
    }

    // Fractions use finite IEEE-754 binary64 semantics across the v1 wire. Values
    // requiring lossless decimal or identifier semantics must be represented as strings.
    let Some(value) = number.as_f64() else {
        return Err("surface data number is not representable as finite binary64".to_string());
    };
    if !value.is_finite() {
        return Err("surface data number must be finite".to_string());
    }
    if value.abs() > MAX_SAFE_JSON_INTEGER as f64 {
        return Err("surface data decimal exceeds the v1 exact-safe numeric magnitude".to_string());
    }
    Ok(())
}

fn invalid_view(
    surface: SurfaceId,
    reason: impl Into<String>,
) -> Result<(), InteractionFrameError> {
    Err(InteractionFrameError::InvalidView {
        surface: surface.as_str(),
        reason: reason.into(),
    })
}
