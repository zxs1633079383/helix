//! `message.timeline` 的有界窗口投影。
//!
//! 输入是现有 `query::render_ready` 已经完成身份、顺序和 capability 裁决的消息行；本模块
//! 只在 Helix 内把它们收紧成 `TimelineWindowVm`。客户端不会再读取 raw row 或自行排序。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use thiserror::Error;

use crate::interaction_frame::{
    ActionBinding, DeliveryVm, InteractionFrameError, MessageContentVm, MessageQuickActionVm,
    MessageReactionVm, MessageVm, ReplaceView, ReplaceViewKind, ReplyThreadPreviewVm,
    ReplyThreadSummaryVm, SurfaceDataVm, SurfaceId, SurfaceLoadState, SurfaceMutation,
    SurfaceScope, TimelineAnchorMode, TimelineAnchorVm, TimelineWindowVm, TypedSurfaceMutation,
    UserRefVm, WindowPageVm, MAX_SAFE_JSON_INTEGER, MIN_SAFE_JSON_INTEGER,
};

/// 单个 attached timeline slot 最多保留的消息数。分页/窗口边界由 Helix 决定。
pub const MAX_TIMELINE_WINDOW_ITEMS: usize = 60;
const MAX_SURFACE_DATA_DEPTH: usize = 8;
const MAX_SURFACE_DATA_ITEMS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineWindowRequest {
    pub channel_id: String,
    pub window_token: String,
    pub anchor: TimelineAnchorVm,
    pub first_unread_message_id: Option<String>,
    pub target_message_id: Option<String>,
    pub page: WindowPageVm,
    pub state: SurfaceLoadState,
    /// 由 SurfaceRuntime 当前 session epoch 注入；所有 action binding 必须原样暴露该值给 renderer。
    pub surface_epoch: u64,
    /// runtime identity 缺失时不能下发可 invoke 的 binding；renderer 仍能看到显式禁用原因。
    pub action_authority_available: bool,
}

impl TimelineWindowRequest {
    pub fn latest(channel_id: impl Into<String>) -> Self {
        Self::latest_with_window_token(channel_id, "latest")
    }

    pub fn latest_with_window_token(
        channel_id: impl Into<String>,
        window_token: impl Into<String>,
    ) -> Self {
        let channel_id = channel_id.into();
        let window_token = window_token.into();
        Self {
            channel_id,
            window_token: window_token.clone(),
            anchor: TimelineAnchorVm {
                message_id: None,
                mode: TimelineAnchorMode::Latest,
            },
            first_unread_message_id: None,
            target_message_id: None,
            page: WindowPageVm {
                window_token,
                has_older: false,
                has_newer: false,
                has_more: false,
            },
            state: SurfaceLoadState::Ready,
            surface_epoch: 1,
            action_authority_available: true,
        }
    }
}

#[derive(Debug)]
pub struct TimelineProjector {
    slots: BTreeMap<String, TimelineSlot>,
    // An attach request may race the first durable fact for its channel. Keep
    // that intent separately from the first snapshot so terminal writes can
    // schedule the authoritative readback instead of falling back to a raw
    // post event while the initial storage query is in flight.
    attached_slots: BTreeSet<String>,
    /// 仅用于 opaque actionRef 的 Helix 内部发号；绝不包含 channel/message/kind 等业务标识。
    next_action_ref: u64,
}

#[derive(Debug, Clone)]
struct TimelineSlot {
    revision: u64,
    view: TimelineWindowVm,
}

impl Default for TimelineProjector {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
            attached_slots: BTreeSet::new(),
            next_action_ref: 1,
        }
    }
}

impl TimelineProjector {
    pub fn reset(&mut self) {
        self.slots.clear();
        self.attached_slots.clear();
    }

    pub fn register_attachment(&mut self, scope: &SurfaceScope) {
        if matches!(scope, SurfaceScope::Timeline { .. }) {
            self.attached_slots.insert(scope.canonical_key());
        }
    }

    pub fn is_attached(&self, scope: &SurfaceScope) -> bool {
        self.attached_slots.contains(&scope.canonical_key())
    }

    /// 建立完整 snapshot。`render_ready_rows` 必须已经是时间正序；本函数不会重新排序。
    pub fn snapshot_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        render_ready_rows: &[Value],
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        let (scope, slot_key) = scope_and_slot_key(&request)?;
        let view = self.build_view(&request, render_ready_rows)?;
        let revision = self
            .slots
            .get(&slot_key)
            .map_or(Ok(1), |slot| next_revision(slot.revision))?;
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: None,
            causation_id: None,
            mutation: TypedSurfaceMutation::TimelineSnapshot { view: view.clone() },
        };
        mutation.validate()?;
        self.attached_slots.insert(slot_key.clone());
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(mutation)
    }

    /// 对已 attach slot 以完整 `replaceView` 形式推进。v1 不下发客户端需解释的 row delta。
    pub fn patch_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        render_ready_rows: &[Value],
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        let (scope, slot_key) = scope_and_slot_key(&request)?;
        let slot =
            self.slots
                .get(&slot_key)
                .ok_or_else(|| TimelineProjectionError::MissingSnapshot {
                    slot: slot_key.clone(),
                })?;
        let base_revision = slot.revision;
        let view = self.build_view(&request, render_ready_rows)?;
        let revision = next_revision(base_revision)?;
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: Some(base_revision),
            causation_id,
            mutation: TypedSurfaceMutation::TimelinePatch {
                patch: ReplaceView {
                    kind: ReplaceViewKind::ReplaceView,
                    view: view.clone(),
                },
            },
        };
        mutation.validate()?;
        self.attached_slots.insert(slot_key.clone());
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(mutation)
    }

    pub fn current_view(&self, scope: &SurfaceScope) -> Option<&TimelineWindowVm> {
        self.slots
            .get(&scope.canonical_key())
            .map(|slot| &slot.view)
    }

    pub(crate) fn patch_located_target(
        &mut self,
        target_message_id: &str,
    ) -> Result<Option<SurfaceMutation>, TimelineProjectionError> {
        let Some((slot_key, slot)) = self
            .slots
            .iter()
            .find(|(_, slot)| {
                slot.view
                    .items
                    .iter()
                    .any(|item| item.id == target_message_id)
            })
            .map(|(key, slot)| (key.clone(), slot.clone()))
        else {
            return Ok(None);
        };
        let mut view = slot.view;
        view.anchor = TimelineAnchorVm {
            message_id: Some(target_message_id.to_string()),
            mode: TimelineAnchorMode::Locate,
        };
        view.target_message_id = Some(target_message_id.to_string());
        let revision = next_revision(slot.revision)?;
        let scope = SurfaceScope::Timeline {
            channel_id: view.channel_id.clone(),
            window_token: view.page.window_token.clone(),
        };
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: Some(slot.revision),
            causation_id: None,
            mutation: TypedSurfaceMutation::TimelinePatch {
                patch: ReplaceView {
                    kind: ReplaceViewKind::ReplaceView,
                    view: view.clone(),
                },
            },
        };
        mutation.validate()?;
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(Some(mutation))
    }

    pub(crate) fn patch_receipts(
        &mut self,
        receipts: &[Value],
    ) -> Result<Vec<SurfaceMutation>, TimelineProjectionError> {
        let by_post: BTreeMap<&str, &Value> = receipts
            .iter()
            .filter_map(|receipt| {
                receipt
                    .get("postId")
                    .and_then(Value::as_str)
                    .map(|id| (id, receipt))
            })
            .collect();
        if by_post.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = self.slots.keys().cloned().collect();
        let mut mutations = Vec::new();
        for slot_key in keys {
            let Some(slot) = self.slots.get(&slot_key).cloned() else {
                continue;
            };
            let mut view = slot.view;
            let mut changed = false;
            for item in &mut view.items {
                let Some(receipt) = by_post.get(item.id.as_str()) else {
                    continue;
                };
                item.read_receipt = surface_data(receipt, 0)?;
                changed = true;
            }
            if !changed {
                continue;
            }
            let revision = next_revision(slot.revision)?;
            let scope = SurfaceScope::Timeline {
                channel_id: view.channel_id.clone(),
                window_token: view.page.window_token.clone(),
            };
            let mutation = SurfaceMutation {
                surface: SurfaceId::MessageTimeline,
                scope,
                revision,
                base_revision: Some(slot.revision),
                causation_id: None,
                mutation: TypedSurfaceMutation::TimelinePatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view: view.clone(),
                    },
                },
            };
            mutation.validate()?;
            self.slots.insert(slot_key, TimelineSlot { revision, view });
            mutations.push(mutation);
        }
        Ok(mutations)
    }

    fn build_view(
        &mut self,
        request: &TimelineWindowRequest,
        render_ready_rows: &[Value],
    ) -> Result<TimelineWindowVm, TimelineProjectionError> {
        let (bounded_rows, page) = bounded_rows(render_ready_rows, &request.anchor, &request.page);
        let mut items = Vec::with_capacity(bounded_rows.len());
        for row in bounded_rows {
            items.push(self.message_from_render_ready(
                row,
                request.channel_id.as_str(),
                request.surface_epoch,
                request.action_authority_available,
            )?);
        }
        enrich_reply_threads(&mut items);
        if let Some(target) = request.target_message_id.as_deref() {
            if !items.iter().any(|item| item.id == target) {
                return Err(TimelineProjectionError::TargetNotInWindow {
                    target: target.to_string(),
                });
            }
        }

        let mut anchor = request.anchor.clone();
        if anchor.message_id.is_none() {
            anchor.message_id = items.last().map(|item| item.id.clone());
        }
        Ok(TimelineWindowVm {
            channel_id: request.channel_id.clone(),
            anchor,
            items,
            first_unread_message_id: request.first_unread_message_id.clone(),
            target_message_id: request.target_message_id.clone(),
            page,
            state: request.state.clone(),
        })
    }

    fn message_from_render_ready(
        &mut self,
        row: &Value,
        expected_channel_id: &str,
        surface_epoch: u64,
        action_authority_available: bool,
    ) -> Result<MessageVm, TimelineProjectionError> {
        message_from_render_ready(
            row,
            expected_channel_id,
            surface_epoch,
            action_authority_available,
            || self.issue_action_ref(),
        )
    }

    fn issue_action_ref(&mut self) -> Result<String, TimelineProjectionError> {
        let sequence = self.next_action_ref;
        self.next_action_ref = self
            .next_action_ref
            .checked_add(1)
            .ok_or(TimelineProjectionError::ActionRefOverflow)?;
        // 两段 reversible-mix 的序列不暴露 domain identity 或可由 renderer 拼出的 command context。
        // 授权安全仍完全由 registry 的 actor/scope/epoch/revision/TTL/replay 校验承担，而非猜测 ref。
        Ok(format!(
            "act.v1.{:016x}{:016x}",
            mix_action_ref(sequence),
            mix_action_ref(sequence ^ 0x9e37_79b9_7f4a_7c15)
        ))
    }
}

fn enrich_reply_threads(items: &mut [MessageVm]) {
    let mut children: BTreeMap<String, Vec<ReplyThreadPreviewVm>> = BTreeMap::new();
    for item in items.iter() {
        let root_id = item.reply_root_id.as_deref().or(item.reply_id.as_deref());
        let Some(root_id) = root_id.filter(|id| !id.is_empty() && *id != item.id) else {
            continue;
        };
        let text = match &item.content {
            MessageContentVm::Text { text }
            | MessageContentVm::SystemEvent { text, .. }
            | MessageContentVm::Recalled { text } => text.clone(),
            MessageContentVm::Document { title, .. } => title.clone(),
            MessageContentVm::Media { file_name, .. } => file_name.clone(),
            MessageContentVm::Forwarded { summary, .. } => summary.clone(),
        };
        children
            .entry(root_id.to_string())
            .or_default()
            .push(ReplyThreadPreviewVm {
                id: item.id.clone(),
                author: item.author.clone(),
                text,
            });
    }
    for item in items.iter_mut() {
        let Some(replies) = children.get(&item.id) else {
            continue;
        };
        let projected_count = replies.len().min(u32::MAX as usize) as u32;
        let existing_count = item
            .reply_thread
            .as_ref()
            .map_or(0, |thread| thread.reply_count);
        item.reply_thread = Some(ReplyThreadSummaryVm {
            items: replies
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            reply_count: existing_count.max(projected_count),
        });
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimelineProjectionError {
    #[error("timeline channelId must not be empty")]
    EmptyChannelId,
    #[error("timeline windowToken must not be empty")]
    EmptyWindowToken,
    #[error("timeline row is missing required {field}")]
    MissingRowField { field: &'static str },
    #[error("timeline row channelId {actual} does not match scope channelId {expected}")]
    ChannelMismatch { expected: String, actual: String },
    #[error("timeline patch requires a snapshot for slot {slot}")]
    MissingSnapshot { slot: String },
    #[error("timeline surface data exceeds the v1 bounded contract")]
    SurfaceDataLimit,
    #[error("timeline revision reached the exact-safe wire limit")]
    RevisionOverflow,
    #[error("timeline actionRef allocator exhausted its unique sequence")]
    ActionRefOverflow,
    #[error("timeline locate target {target} is absent from the bounded window")]
    TargetNotInWindow { target: String },
    #[error(transparent)]
    InvalidInteractionFrame(#[from] InteractionFrameError),
}

fn scope_and_slot_key(
    request: &TimelineWindowRequest,
) -> Result<(SurfaceScope, String), TimelineProjectionError> {
    if request.channel_id.is_empty() {
        return Err(TimelineProjectionError::EmptyChannelId);
    }
    if request.window_token.is_empty() {
        return Err(TimelineProjectionError::EmptyWindowToken);
    }
    let scope = SurfaceScope::Timeline {
        channel_id: request.channel_id.clone(),
        window_token: request.window_token.clone(),
    };
    Ok((scope.clone(), scope.canonical_key()))
}

fn bounded_rows<'a>(
    rows: &'a [Value],
    anchor: &TimelineAnchorVm,
    requested_page: &WindowPageVm,
) -> (&'a [Value], WindowPageVm) {
    if rows.len() <= MAX_TIMELINE_WINDOW_ITEMS {
        return (rows, requested_page.clone());
    }

    let start = match anchor.mode {
        TimelineAnchorMode::Latest => rows.len() - MAX_TIMELINE_WINDOW_ITEMS,
        TimelineAnchorMode::Locate | TimelineAnchorMode::Unread => anchor
            .message_id
            .as_deref()
            .and_then(|id| row_id_index(rows, id))
            .map(|index| {
                index
                    .saturating_sub(20)
                    .min(rows.len() - MAX_TIMELINE_WINDOW_ITEMS)
            })
            .unwrap_or(rows.len() - MAX_TIMELINE_WINDOW_ITEMS),
    };
    let end = start + MAX_TIMELINE_WINDOW_ITEMS;
    let mut page = requested_page.clone();
    page.has_older |= start > 0;
    page.has_newer |= end < rows.len();
    page.has_more |= page.has_older || page.has_newer;
    (&rows[start..end], page)
}

fn row_id_index(rows: &[Value], target: &str) -> Option<usize> {
    rows.iter()
        .position(|row| row_string(row, &["msgId", "id"]) == target)
}

/// Converts an already render-ready message fact into the shared closed `MessageVm`.
///
/// Non-timeline surfaces reuse this only with `action_authority_available=false`, so every
/// capability without an audited authority dispatch remains explicitly disabled rather than
/// becoming a renderer-selected command.
pub(crate) fn message_from_render_ready(
    row: &Value,
    expected_channel_id: &str,
    surface_epoch: u64,
    action_authority_available: bool,
    mut issue_action_ref: impl FnMut() -> Result<String, TimelineProjectionError>,
) -> Result<MessageVm, TimelineProjectionError> {
    let id = required_string(row, &["msgId", "id"], "id")?;
    let channel_id = required_string(row, &["channelId", "channel_id"], "channelId")?;
    if channel_id != expected_channel_id {
        return Err(TimelineProjectionError::ChannelMismatch {
            expected: expected_channel_id.to_string(),
            actual: channel_id,
        });
    }
    let author_id = required_string(row, &["userId", "user_id"], "author.userId")?;
    let revoked = bool_at(row, &["revoked", "revoke"]);
    let props = row.get("props").unwrap_or(&Value::Null);
    let content = content_from_row(row, props, revoked)?;
    let delivery = delivery_from_row(row, revoked)?;
    let (actions, quick_actions, reply_summary, reply_thread, reactions, topic_link) = if revoked {
        (
            Vec::new(),
            Vec::new(),
            SurfaceDataVm::Record(BTreeMap::new()),
            None,
            Vec::new(),
            None,
        )
    } else {
        (
            action_bindings(
                row,
                surface_epoch,
                action_authority_available,
                &mut issue_action_ref,
            )?,
            quick_actions_from_row(row),
            record_from_row(
                row,
                &[
                    ("replyCount", "replyCount"),
                    ("replyRootId", "replyRootId"),
                    ("replyMessages", "replyMessages"),
                ],
            )?,
            reply_thread_from_row(row),
            reactions_from_props(props),
            optional_data(row.get("topic"))?,
        )
    };
    Ok(MessageVm {
        id,
        channel_id,
        event_seq: unsigned_integer_at(row, &["eventSeq", "event_seq"], "eventSeq")?
            .unwrap_or_default(),
        author: UserRefVm { user_id: author_id },
        created_at: integer_at(row, &["createdAt", "createAt", "create_at"], "createdAt")?
            .unwrap_or_default(),
        content,
        delivery,
        actions,
        quick_actions,
        reply_id: optional_non_empty_string(row, &["replyId", "reply_id"]),
        reply_root_id: optional_non_empty_string(row, &["replyRootId", "reply_root_id"]),
        reply_first_level_id: optional_non_empty_string(
            row,
            &["replyFirstLevelId", "reply_first_level_id"],
        ),
        reply_summary,
        reply_thread,
        reactions,
        read_receipt: record_from_row(
            row,
            &[("readBits", "readBits"), ("readerIds", "readerIds")],
        )?,
        template_confirmation: optional_data(
            row.get("templateConfirmation")
                .or_else(|| props.get("template")),
        )?,
        urgency: urgency_data(row)?,
        pinned: bool_at(row, &["pinned", "isTop"]),
        bookmarked: bool_at(row, &["bookmarked"]),
        topic_link,
    })
}

fn quick_actions_from_row(row: &Value) -> Vec<MessageQuickActionVm> {
    let Some(capabilities) = row.get("actionCapabilities").and_then(Value::as_object) else {
        return Vec::new();
    };
    [
        ("canReact", MessageQuickActionVm::React),
        ("canReply", MessageQuickActionVm::QuoteReply),
        ("canCreateTopic", MessageQuickActionVm::CreateTopic),
    ]
    .into_iter()
    .filter_map(|(key, action)| {
        capabilities
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then_some(action)
    })
    .collect()
}

fn reply_thread_from_row(row: &Value) -> Option<ReplyThreadSummaryVm> {
    let reply_count = row
        .get("replyCount")
        .or_else(|| row.get("reply_count"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(u32::MAX as u64) as u32;
    let source = row
        .get("replyMessages")
        .or_else(|| row.get("reply_messages"));
    let mut rows: Vec<&Value> = match source {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    rows.sort_by_key(|item| {
        (
            item.get("createAt")
                .or_else(|| item.get("createdAt"))
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            row_string(item, &["msgId", "id"]),
        )
    });
    let items: Vec<ReplyThreadPreviewVm> = rows
        .into_iter()
        .filter_map(|item| {
            let id = row_string(item, &["msgId", "id"]);
            let user_id = row_string(item, &["userId", "user_id"]);
            if id.is_empty() || user_id.is_empty() {
                return None;
            }
            Some(ReplyThreadPreviewVm {
                id,
                author: UserRefVm { user_id },
                text: row_string(item, &["text", "message"]),
            })
        })
        .take(3)
        .collect();
    let reply_count = reply_count.max(items.len() as u32);
    (reply_count > 0).then_some(ReplyThreadSummaryVm { items, reply_count })
}

fn reactions_from_props(props: &Value) -> Vec<MessageReactionVm> {
    let mut reactions = Vec::new();
    match props.get("quickReply") {
        Some(Value::Array(items)) => {
            for item in items {
                let emoji = row_string(item, &["emoji"]);
                let count = item
                    .get("userIds")
                    .or_else(|| item.get("user_ids"))
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or_default();
                if !emoji.is_empty() && count > 0 {
                    reactions.push(MessageReactionVm {
                        emoji,
                        count: count.min(u32::MAX as usize) as u32,
                    });
                }
            }
        }
        Some(Value::Object(items)) => {
            for (emoji, users) in items {
                let count = users.as_array().map(Vec::len).unwrap_or_default();
                if !emoji.is_empty() && count > 0 {
                    reactions.push(MessageReactionVm {
                        emoji: emoji.clone(),
                        count: count.min(u32::MAX as usize) as u32,
                    });
                }
            }
        }
        _ => {}
    }
    reactions
}

fn content_from_row(
    row: &Value,
    props: &Value,
    revoked: bool,
) -> Result<MessageContentVm, TimelineProjectionError> {
    let text = row_string(row, &["text", "message"]);
    if revoked {
        return Ok(MessageContentVm::Recalled { text });
    }
    let kind = row_string(row, &["type"]).to_ascii_uppercase();
    if kind.contains("FORWARD")
        || row
            .get("forwardDetail")
            .is_some_and(|value| !value.is_null())
    {
        return Ok(MessageContentVm::Forwarded {
            summary: text,
            items: data_array(row.get("forwardDetail").unwrap_or(&Value::Null))?,
        });
    }
    if kind.contains("DOCUMENT") {
        return Ok(MessageContentVm::Document {
            document_id: row_string(row, &["documentId", "id", "msgId"]),
            title: row_string(row, &["fileName", "title", "text"]),
            summary: text,
            access: surface_data(props, 0)?,
        });
    }
    if matches!(
        kind.as_str(),
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "MEDIA"
    ) {
        return Ok(MessageContentVm::Media {
            media_type: kind.to_ascii_lowercase(),
            resource: surface_data(props, 0)?,
            thumbnail: optional_data(row.get("thumbnail"))?,
            file_name: row_string(row, &["fileName", "title", "text"]),
            size: integer_at(row, &["size"], "size")?
                .unwrap_or_default()
                .max(0) as u64,
            availability: SurfaceDataVm::Record(BTreeMap::new()),
        });
    }
    if kind.contains("SYSTEM") {
        return Ok(MessageContentVm::SystemEvent {
            event_kind: kind.to_ascii_lowercase(),
            text,
        });
    }
    Ok(MessageContentVm::Text { text })
}

fn delivery_from_row(row: &Value, revoked: bool) -> Result<DeliveryVm, TimelineProjectionError> {
    if revoked {
        return Ok(DeliveryVm::Recalled);
    }
    match row_string(row, &["sendStatus", "send_status"]).as_str() {
        "uploading" => Ok(DeliveryVm::Uploading {
            progress_percent: integer_at(
                row,
                &["progressPercent", "progress_percent"],
                "progressPercent",
            )?
            .unwrap_or_default()
            .clamp(0, 100) as u8,
        }),
        "sending" => Ok(DeliveryVm::Sending),
        "failed" | "unsend" => Ok(DeliveryVm::Failed {
            reason: row_string(row, &["failureReason", "error"]),
            retryable: bool_at(row, &["canRetry"]),
        }),
        _ => Ok(DeliveryVm::Sent),
    }
}

fn action_bindings(
    row: &Value,
    surface_epoch: u64,
    action_authority_available: bool,
    issue_action_ref: &mut impl FnMut() -> Result<String, TimelineProjectionError>,
) -> Result<Vec<ActionBinding>, TimelineProjectionError> {
    let Some(capabilities) = row.get("actionCapabilities").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut actions = Vec::new();
    for (capability, kind) in [
        ("canReply", "reply"),
        ("canForward", "forward"),
        ("canReact", "react"),
        ("canUrgent", "urgent"),
        ("canCreateTopic", "createTopic"),
        ("canRevoke", "revoke"),
        ("canRetry", "retry"),
        ("canUrgentConfirm", "urgentConfirm"),
        ("canUrgentCancel", "urgentCancel"),
        ("canMarkRead", "markRead"),
        ("canViewReceipt", "viewReceipt"),
        ("canTemplateConfirm", "templateConfirm"),
        ("canLocate", "locate"),
    ] {
        let capability_enabled = capabilities
            .get(capability)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let enabled = crate::projector::action::ActionDispatch::supports_timeline_kind(kind)
            && capability_enabled
            && action_authority_available;
        let disabled_reason = if enabled {
            None
        } else if !capability_enabled {
            Some("notAvailable".to_string())
        } else if !action_authority_available {
            Some("runtimeIdentityRequired".to_string())
        } else {
            Some("unsupportedByAuthorityV1".to_string())
        };
        actions.push(ActionBinding {
            action_ref: issue_action_ref()?,
            kind: kind.to_string(),
            input_shape: "object".to_string(),
            surface_epoch,
            enabled,
            disabled_reason,
        });
    }
    Ok(actions)
}

fn mix_action_ref(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn record_from_row(
    row: &Value,
    fields: &[(&str, &str)],
) -> Result<SurfaceDataVm, TimelineProjectionError> {
    let mut record = BTreeMap::new();
    for (output, input) in fields {
        if let Some(value) = row.get(*input) {
            record.insert((*output).to_string(), surface_data(value, 0)?);
        }
    }
    Ok(SurfaceDataVm::Record(record))
}

fn urgency_data(row: &Value) -> Result<Option<SurfaceDataVm>, TimelineProjectionError> {
    let mut record = BTreeMap::new();
    for key in [
        "urgent",
        "urgentTargetCount",
        "urgentConfirmedCount",
        "urgentRequesterId",
        "urgentTargetIds",
        "urgentConfirmedIds",
        "urgentRequiredCount",
        "urgentState",
        "canConfirmUrgent",
    ] {
        if let Some(value) = row.get(key) {
            record.insert(key.to_string(), surface_data(value, 0)?);
        }
    }
    if record.is_empty() {
        Ok(None)
    } else {
        Ok(Some(SurfaceDataVm::Record(record)))
    }
}

fn optional_data(value: Option<&Value>) -> Result<Option<SurfaceDataVm>, TimelineProjectionError> {
    match value.filter(|value| !value.is_null()) {
        Some(value) => Ok(Some(surface_data(value, 0)?)),
        None => Ok(None),
    }
}

fn data_array(value: &Value) -> Result<Vec<SurfaceDataVm>, TimelineProjectionError> {
    match value {
        Value::Array(items) => items.iter().map(|item| surface_data(item, 0)).collect(),
        Value::Null => Ok(Vec::new()),
        other => Ok(vec![surface_data(other, 0)?]),
    }
}

fn surface_data(value: &Value, depth: usize) -> Result<SurfaceDataVm, TimelineProjectionError> {
    if depth > MAX_SURFACE_DATA_DEPTH {
        return Err(TimelineProjectionError::SurfaceDataLimit);
    }
    match value {
        Value::Null => Ok(SurfaceDataVm::Null(())),
        Value::Bool(value) => Ok(SurfaceDataVm::Bool(*value)),
        Value::Number(value) => Ok(SurfaceDataVm::Number(value.clone())),
        Value::String(value) => Ok(SurfaceDataVm::String(value.clone())),
        Value::Array(items) => {
            if items.len() > MAX_SURFACE_DATA_ITEMS {
                return Err(TimelineProjectionError::SurfaceDataLimit);
            }
            items
                .iter()
                .map(|item| surface_data(item, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(SurfaceDataVm::Array)
        }
        Value::Object(values) => {
            if values.len() > MAX_SURFACE_DATA_ITEMS {
                return Err(TimelineProjectionError::SurfaceDataLimit);
            }
            let mut record = BTreeMap::new();
            for (key, nested) in values {
                record.insert(key.clone(), surface_data(nested, depth + 1)?);
            }
            Ok(SurfaceDataVm::Record(record))
        }
    }
}

fn required_string(
    row: &Value,
    keys: &[&str],
    field: &'static str,
) -> Result<String, TimelineProjectionError> {
    let value = row_string(row, keys);
    (!value.is_empty())
        .then_some(value)
        .ok_or(TimelineProjectionError::MissingRowField { field })
}

fn row_string(row: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn optional_non_empty_string(row: &Value, keys: &[&str]) -> Option<String> {
    let value = row_string(row, keys);
    (!value.is_empty()).then_some(value)
}

fn integer_at(
    row: &Value,
    keys: &[&str],
    field: &'static str,
) -> Result<Option<i64>, TimelineProjectionError> {
    for key in keys {
        let Some(Value::Number(number)) = row.get(*key) else {
            continue;
        };
        let Some(value) = number.as_i64() else {
            return Err(unsafe_wire_number(field));
        };
        if !(MIN_SAFE_JSON_INTEGER..=MAX_SAFE_JSON_INTEGER as i64).contains(&value) {
            return Err(unsafe_wire_number(field));
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn unsigned_integer_at(
    row: &Value,
    keys: &[&str],
    field: &'static str,
) -> Result<Option<u64>, TimelineProjectionError> {
    for key in keys {
        let Some(Value::Number(number)) = row.get(*key) else {
            continue;
        };
        let Some(value) = number.as_u64() else {
            return Err(unsafe_wire_number(field));
        };
        if value > MAX_SAFE_JSON_INTEGER {
            return Err(unsafe_wire_number(field));
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn unsafe_wire_number(field: &'static str) -> TimelineProjectionError {
    TimelineProjectionError::InvalidInteractionFrame(InteractionFrameError::UnsafeWireNumber {
        surface: "message.timeline",
        field,
    })
}

fn bool_at(row: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| row.get(*key))
        .and_then(|value| match value {
            Value::Bool(value) => Some(*value),
            Value::Number(value) => value.as_i64().map(|value| value != 0),
            _ => None,
        })
        .unwrap_or(false)
}

fn next_revision(current: u64) -> Result<u64, TimelineProjectionError> {
    current
        .checked_add(1)
        .filter(|revision| *revision <= MAX_SAFE_JSON_INTEGER)
        .ok_or(TimelineProjectionError::RevisionOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: usize) -> Value {
        serde_json::json!({
            "msgId": format!("message-{index}"),
            "channelId": "channel-1",
            "eventSeq": index,
            "userId": "user-1",
            "createAt": index,
            "text": format!("text-{index}"),
            "type": "TEXT",
            "sendStatus": "sent",
            "props": {},
            "actionCapabilities": {},
        })
    }

    #[test]
    fn attachment_is_visible_before_the_first_snapshot_arrives() {
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let mut projector = TimelineProjector::default();

        projector.register_attachment(&scope);

        assert!(projector.is_attached(&scope));
        assert!(projector.current_view(&scope).is_none());
    }

    #[test]
    fn locate_and_receipt_authority_patch_the_existing_timeline_vm() {
        let mut projector = TimelineProjector::default();
        projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &[row(1)])
            .unwrap();

        let locate = projector
            .patch_located_target("message-1")
            .unwrap()
            .unwrap();
        assert_eq!(locate.revision, 2);
        let receipts = serde_json::json!([{
            "postId": "message-1", "state": "ready", "readBits": "1",
            "readerCount": 1, "unreadCount": 0, "snapshotId": "snapshot-1",
            "snapshotVersion": 3, "authorExcluded": true, "contractVersion": 2
        }]);
        let receipt_patches = projector
            .patch_receipts(receipts.as_array().unwrap())
            .unwrap();
        assert_eq!(receipt_patches.len(), 1);
        assert_eq!(receipt_patches[0].base_revision, Some(2));
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let view = projector.current_view(&scope).unwrap();
        assert_eq!(view.target_message_id.as_deref(), Some("message-1"));
        let SurfaceDataVm::Record(receipt) = &view.items[0].read_receipt else {
            panic!("receipt record")
        };
        assert_eq!(
            receipt.get("snapshotId"),
            Some(&SurfaceDataVm::String("snapshot-1".to_string()))
        );
    }

    #[test]
    fn bounded_timeline_derives_reply_preview_from_authoritative_reply_anchors() {
        let root = row(1);
        let mut reply = row(2);
        reply["replyId"] = serde_json::json!("message-1");
        reply["replyRootId"] = serde_json::json!("message-1");
        let mut projector = TimelineProjector::default();
        let mutation = projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &[root, reply])
            .unwrap();
        let TypedSurfaceMutation::TimelineSnapshot { view } = mutation.mutation else {
            panic!("timeline")
        };
        let thread = view.items[0].reply_thread.as_ref().expect("reply thread");
        assert_eq!(thread.reply_count, 1);
        assert_eq!(thread.items[0].id, "message-2");
    }

    #[test]
    fn message_projection_caps_reply_preview_and_preserves_authoritative_count_and_reactions() {
        let mut source = row(1);
        source["replyCount"] = serde_json::json!(5);
        source["replyMessages"] = serde_json::json!([
            {"id":"reply-1","userId":"user-1","message":"一","createAt":1},
            {"id":"reply-2","userId":"user-2","message":"二","createAt":2},
            {"id":"reply-3","userId":"user-3","message":"三","createAt":3},
            {"id":"reply-4","userId":"user-4","message":"四","createAt":4}
        ]);
        source["props"] = serde_json::json!({
            "quickReply": [
                {"emoji":"😊","userIds":["user-1","user-2"]},
                {"emoji":"👍","userIds":["user-3"]}
            ]
        });
        source["actionCapabilities"] = serde_json::json!({
            "canReact": true,
            "canReply": true,
            "canCreateTopic": true
        });

        let message = message_from_render_ready(&source, "channel-1", 1, false, || {
            Ok("action-ref".to_string())
        })
        .expect("render-ready message");

        assert_eq!(serde_json::to_value(&message).unwrap()["eventSeq"], 1);
        let thread = message.reply_thread.expect("reply summary");
        assert_eq!(thread.items.len(), 3);
        assert_eq!(thread.reply_count, 5);
        assert_eq!(thread.items[2].text, "三");
        assert_eq!(message.reactions[0].emoji, "😊");
        assert_eq!(message.reactions[0].count, 2);
        assert_eq!(message.reactions[1].count, 1);
        assert_eq!(message.event_seq, 1);
        assert_eq!(
            message.quick_actions,
            vec![
                MessageQuickActionVm::React,
                MessageQuickActionVm::QuoteReply,
                MessageQuickActionVm::CreateTopic
            ]
        );
    }

    #[test]
    fn recalled_message_is_a_terminal_non_interactive_system_row() {
        let mut source = row(1);
        source["revoke"] = serde_json::json!(true);
        source["replyCount"] = serde_json::json!(1);
        source["replyMessages"] = serde_json::json!([
            {"id":"reply-1","userId":"user-2","message":"reply","createAt":2}
        ]);
        source["props"] = serde_json::json!({
            "quickReply": [{"emoji":"👍","userIds":["user-2"]}]
        });
        source["topic"] = serde_json::json!({"topicChannelId":"topic-1"});
        source["actionCapabilities"] = serde_json::json!({
            "canReact": true,
            "canReply": true,
            "canForward": true,
            "canRevoke": true
        });
        let mut issued_refs = 0;

        let message = message_from_render_ready(&source, "channel-1", 1, true, || {
            issued_refs += 1;
            Ok(format!("action-ref-{issued_refs}"))
        })
        .expect("recalled render-ready message");
        let encoded = serde_json::to_value(&message).expect("serialize recalled message");

        assert!(matches!(message.content, MessageContentVm::Recalled { .. }));
        assert!(matches!(message.delivery, DeliveryVm::Recalled));
        assert!(message.actions.is_empty());
        assert!(message.quick_actions.is_empty());
        assert!(message.reply_thread.is_none());
        assert!(message.reactions.is_empty());
        assert!(message.topic_link.is_none());
        assert_eq!(
            message.reply_summary,
            SurfaceDataVm::Record(BTreeMap::new())
        );
        assert_eq!(
            issued_refs, 0,
            "recalled rows must not mint executable action refs"
        );
        assert!(encoded.get("quickActions").is_none());
        assert!(encoded.get("replyThread").is_none());
        assert!(encoded.get("reactions").is_none());
        assert!(encoded.get("topicLink").is_none());
    }

    #[test]
    fn snapshot_bounds_latest_window_and_patch_replaces_complete_view() {
        let mut projector = TimelineProjector::default();
        let rows = (0..80).map(row).collect::<Vec<_>>();
        let snapshot = projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &rows)
            .expect("valid timeline snapshot");
        assert_eq!(snapshot.revision, 1);
        let TypedSurfaceMutation::TimelineSnapshot { view } = snapshot.mutation else {
            panic!("timeline snapshot mutation")
        };
        assert_eq!(view.items.len(), MAX_TIMELINE_WINDOW_ITEMS);
        assert_eq!(view.items.first().unwrap().id, "message-20");
        assert_eq!(view.items.last().unwrap().id, "message-79");
        assert!(view.page.has_older);

        let patch = projector
            .patch_from_render_ready(
                TimelineWindowRequest::latest("channel-1"),
                &rows[1..],
                Some("post:80".to_string()),
            )
            .expect("existing slot can advance");
        assert_eq!(patch.base_revision, Some(1));
        assert_eq!(patch.revision, 2);
        assert_eq!(patch.causation_id.as_deref(), Some("post:80"));
        assert!(matches!(
            patch.mutation,
            TypedSurfaceMutation::TimelinePatch {
                patch: ReplaceView {
                    kind: ReplaceViewKind::ReplaceView,
                    ..
                }
            }
        ));
    }

    #[test]
    fn revision_overflow_is_rejected_without_wrapping_the_timeline_slot() {
        let mut projector = TimelineProjector::default();
        let rows = vec![row(1)];
        projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &rows)
            .unwrap();
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        projector
            .slots
            .get_mut(&scope.canonical_key())
            .expect("snapshot slot")
            .revision = MAX_SAFE_JSON_INTEGER - 1;
        let boundary_patch = projector
            .patch_from_render_ready(TimelineWindowRequest::latest("channel-1"), &rows, None)
            .expect("the exact-safe maximum itself remains representable");
        assert_eq!(boundary_patch.revision, MAX_SAFE_JSON_INTEGER);
        assert!(matches!(
            projector.patch_from_render_ready(
                TimelineWindowRequest::latest("channel-1"),
                &rows,
                None,
            ),
            Err(TimelineProjectionError::RevisionOverflow)
        ));
    }

    #[test]
    fn unsafe_persisted_number_is_rejected_before_timeline_slot_commits() {
        let mut projector = TimelineProjector::default();
        let request = TimelineWindowRequest::latest("channel-1");
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let mut just_outside_safe_range = row(1);
        just_outside_safe_range["createAt"] = serde_json::json!(MAX_SAFE_JSON_INTEGER as i64 + 1);

        assert!(matches!(
            projector.snapshot_from_render_ready(request.clone(), &[just_outside_safe_range]),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber {
                    field: "createdAt",
                    ..
                }
            ))
        ));
        assert!(projector.current_view(&scope).is_none());

        let valid = row(2);
        projector
            .snapshot_from_render_ready(request.clone(), std::slice::from_ref(&valid))
            .expect("valid row attaches the timeline slot");
        let before = projector.current_view(&scope).cloned();
        let mut unsafe_created_at = row(3);
        unsafe_created_at["createAt"] = serde_json::json!(u64::MAX);

        assert!(matches!(
            projector.patch_from_render_ready(request.clone(), &[unsafe_created_at], None),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber {
                    field: "createdAt",
                    ..
                }
            ))
        ));
        assert_eq!(projector.current_view(&scope).cloned(), before);

        let mut unsafe_media_size = row(4);
        unsafe_media_size["type"] = serde_json::json!("FILE");
        unsafe_media_size["size"] = serde_json::json!(u64::MAX);
        assert!(matches!(
            projector.patch_from_render_ready(request.clone(), &[unsafe_media_size], None),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber { field: "size", .. }
            ))
        ));
        assert_eq!(projector.current_view(&scope).cloned(), before);

        let mut unsafe_upload_progress = row(5);
        unsafe_upload_progress["sendStatus"] = serde_json::json!("uploading");
        unsafe_upload_progress["progressPercent"] = serde_json::json!(u64::MAX);
        assert!(matches!(
            projector.patch_from_render_ready(request, &[unsafe_upload_progress], None),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber {
                    field: "progressPercent",
                    ..
                }
            ))
        ));
        assert_eq!(projector.current_view(&scope).cloned(), before);
    }

    #[test]
    fn action_refs_are_opaque_and_only_the_live_authority_mapping_is_enabled() {
        let mut projector = TimelineProjector::default();
        let mut capability_row = row(1);
        capability_row["actionCapabilities"] = serde_json::json!({
            "canReply": true,
            "canCreateTopic": true,
            "canMarkRead": true,
            "canViewReceipt": true,
            "canTemplateConfirm": true,
            "canLocate": true,
        });

        let mut request = TimelineWindowRequest::latest("channel-1");
        request.surface_epoch = 42;
        let snapshot = projector
            .snapshot_from_render_ready(request, &[capability_row])
            .expect("valid capability facts project");
        let TypedSurfaceMutation::TimelineSnapshot { view } = snapshot.mutation else {
            panic!("timeline snapshot expected")
        };
        let receipt = view.items[0]
            .actions
            .iter()
            .find(|action| action.kind == "viewReceipt")
            .expect("receipt binding is present");
        assert!(receipt.enabled);
        assert!(receipt.disabled_reason.is_none());
        assert_eq!(receipt.surface_epoch, 42);
        assert!(!receipt.action_ref.contains("message-1"));
        assert!(!receipt.action_ref.contains("viewReceipt"));

        for kind in [
            "reply",
            "createTopic",
            "markRead",
            "templateConfirm",
            "locate",
        ] {
            let action = view.items[0]
                .actions
                .iter()
                .find(|action| action.kind == kind)
                .unwrap_or_else(|| panic!("{kind} binding is present"));
            assert!(!action.enabled, "{kind}");
            assert_eq!(
                action.disabled_reason.as_deref(),
                Some("unsupportedByAuthorityV1"),
                "{kind} cannot be enabled by a row capability before the terminal authority Frame exists"
            );
        }
    }

    #[test]
    fn locate_target_must_be_present_in_the_complete_bounded_vm() {
        let mut projector = TimelineProjector::default();
        let mut request = TimelineWindowRequest::latest_with_window_token("channel-1", "locate-1");
        request.anchor = TimelineAnchorVm {
            message_id: Some("message-2".to_string()),
            mode: TimelineAnchorMode::Locate,
        };
        request.target_message_id = Some("message-2".to_string());
        let mutation = projector
            .snapshot_from_render_ready(request.clone(), &[row(1), row(2), row(3)])
            .unwrap();
        let TypedSurfaceMutation::TimelineSnapshot { view } = mutation.mutation else {
            panic!("timeline snapshot expected")
        };
        assert_eq!(view.target_message_id.as_deref(), Some("message-2"));

        request.window_token = "locate-missing".to_string();
        request.page.window_token = "locate-missing".to_string();
        request.target_message_id = Some("message-99".to_string());
        assert!(matches!(
            projector.snapshot_from_render_ready(request, &[row(1), row(2)]),
            Err(TimelineProjectionError::TargetNotInWindow { target })
                if target == "message-99"
        ));
    }
}
