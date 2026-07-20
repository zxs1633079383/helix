//! `message.timeline` 的有界窗口投影。
//!
//! 输入是现有 `query::render_ready` 已经完成身份、顺序和 capability 裁决的消息行；本模块
//! 只在 Helix 内把它们收紧成 `TimelineWindowVm`。客户端不会再读取 raw row 或自行排序。

use std::collections::BTreeMap;

use serde_json::Value;
use thiserror::Error;

use crate::interaction_frame::{
    ActionBinding, DeliveryVm, InteractionFrameError, MessageContentVm, MessageVm, ReplaceView,
    ReplaceViewKind, SurfaceDataVm, SurfaceId, SurfaceLoadState, SurfaceMutation, SurfaceScope,
    TimelineAnchorMode, TimelineAnchorVm, TimelineWindowVm, TypedSurfaceMutation, UserRefVm,
    WindowPageVm, MAX_SAFE_JSON_INTEGER, MIN_SAFE_JSON_INTEGER,
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
        }
    }
}

#[derive(Debug, Default)]
pub struct TimelineProjector {
    slots: BTreeMap<String, TimelineSlot>,
}

#[derive(Debug, Clone)]
struct TimelineSlot {
    revision: u64,
    view: TimelineWindowVm,
}

impl TimelineProjector {
    pub fn reset(&mut self) {
        self.slots.clear();
    }

    /// 建立完整 snapshot。`render_ready_rows` 必须已经是时间正序；本函数不会重新排序。
    pub fn snapshot_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        render_ready_rows: &[Value],
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        let (scope, slot_key) = scope_and_slot_key(&request)?;
        let view = build_view(&request, render_ready_rows)?;
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
        let view = build_view(&request, render_ready_rows)?;
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
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(mutation)
    }

    pub fn current_view(&self, scope: &SurfaceScope) -> Option<&TimelineWindowVm> {
        self.slots
            .get(&scope.canonical_key())
            .map(|slot| &slot.view)
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

fn build_view(
    request: &TimelineWindowRequest,
    render_ready_rows: &[Value],
) -> Result<TimelineWindowVm, TimelineProjectionError> {
    let (bounded_rows, page) = bounded_rows(render_ready_rows, &request.anchor, &request.page);
    let mut items = Vec::with_capacity(bounded_rows.len());
    for row in bounded_rows {
        items.push(message_from_render_ready(row, request.channel_id.as_str())?);
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

fn message_from_render_ready(
    row: &Value,
    expected_channel_id: &str,
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

    let actions = action_bindings(row, &id);
    Ok(MessageVm {
        id,
        channel_id,
        author: UserRefVm { user_id: author_id },
        created_at: integer_at(row, &["createdAt", "createAt", "create_at"], "createdAt")?
            .unwrap_or_default(),
        content,
        delivery,
        actions,
        reply_summary: record_from_row(
            row,
            &[
                ("replyCount", "replyCount"),
                ("replyRootId", "replyRootId"),
                ("replyMessages", "replyMessages"),
            ],
        )?,
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
        topic_link: optional_data(row.get("topic"))?,
    })
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

fn action_bindings(row: &Value, message_id: &str) -> Vec<ActionBinding> {
    let Some(capabilities) = row.get("actionCapabilities").and_then(Value::as_object) else {
        return Vec::new();
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
        ("canMarkRead", "markRead"),
        ("canViewReceipt", "viewReceipt"),
        ("canTemplateConfirm", "templateConfirm"),
        ("canLocate", "locate"),
    ] {
        let enabled = capabilities
            .get(capability)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        actions.push(ActionBinding {
            action_ref: format!("message:{message_id}:{kind}"),
            kind: kind.to_string(),
            input_shape: "object".to_string(),
            enabled,
            disabled_reason: (!enabled).then(|| "notAvailable".to_string()),
        });
    }
    actions
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
}
