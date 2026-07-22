//! Opaque cross-platform SurfaceRuntime attach/resync entrypoint.
//!
//! Desktop and mobile submit only `surface + scope`; this module owns the mapping from that
//! request to an audited storage or HTTP fact. In particular, renderers never choose a table,
//! endpoint, action command, or raw fact payload.

use std::collections::BTreeMap;

use helix_core::effect::{Effect, GetSpec, ScanOrder, ScanSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ImError;
use crate::interaction_frame::{
    ActionBinding, ChannelAvatarRef, ChannelLifecycleVm, ChannelRole, ChannelSummaryVm,
    ChannelTitleRef, ComposerState, ComposerVm, MemberRosterVm, MemberRowVm, MembershipState,
    NotificationMode, PreviewVm, ReceiptKind, SurfaceId, SurfaceLoadState, SurfaceMutation,
    SurfaceScope, ThreadDetailVm, TopicListVm, TopicRowVm, UserRefVm, WindowPageVm,
};
use crate::projector::participant_receipts::{ParticipantReceiptsData, ParticipantReceiptsRequest};
use crate::state::CorrelationContext;
use crate::ImModule;

const MEMBER_ROSTER_ORDER: &[ScanOrder] = &[ScanOrder::asc("joined_at"), ScanOrder::asc("user_id")];
const TOPIC_LIST_ORDER: &[ScanOrder] = &[ScanOrder::desc("last_post_at"), ScanOrder::asc("id")];

/// Private host-to-core command. The FFI/native host owns the public
/// `SurfaceResyncRequest` translation and must use the typed builder in `client_api`.
pub const SURFACE_RESYNC_COMMAND: &str = "__helix_request_surface_snapshot";

/// A renderer-independent request for an authoritative typed surface replay.
///
/// It deliberately has no table, HTTP method, page SQL, or action field. The scope is already a
/// closed InteractionFrame type, so a desktop/mobile host cannot accidentally ask a channel
/// reader to populate an unrelated surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SurfaceResyncRequest {
    pub surface: SurfaceId,
    pub scope: SurfaceScope,
}

impl SurfaceResyncRequest {
    pub fn validate(&self) -> Result<(), ImError> {
        if !self.scope.valid_for(self.surface) {
            return Err(ImError::Parse(format!(
                "surface resync scope does not match {}",
                self.surface.as_str()
            )));
        }
        Ok(())
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, ImError> {
        let request: Self = serde_json::from_slice(payload)
            .map_err(|error| ImError::Parse(format!("surface resync payload: {error}")))?;
        request.validate()?;
        Ok(request)
    }
}

impl ImModule {
    pub(crate) fn handle_surface_resync(
        &mut self,
        payload: &[u8],
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = SurfaceResyncRequest::from_payload(payload)?;
        match (&request.surface, &request.scope) {
            (
                SurfaceId::MessageLeftDialogList | SurfaceId::MessageUnreadSummary,
                SurfaceScope::User { .. },
            ) => {
                // The existing dialog storage producer publishes both atomically in one Frame.
                self.handle_query_command("im_query_dialog_list", br#"{}"#, out)
            }
            (
                SurfaceId::MessageTimeline,
                SurfaceScope::Timeline {
                    channel_id,
                    window_token,
                },
            ) => {
                self.state
                    .timeline_projector
                    .register_attachment(&request.scope);
                let payload = serde_json::to_vec(&serde_json::json!({
                    "channelId": channel_id,
                    "windowToken": window_token,
                }))
                .map_err(|error| ImError::Serialize(error.to_string()))?;
                self.handle_query_command("im_query_messages_by_channel", &payload, out)
            }
            (SurfaceId::MessageThreadDetail, SurfaceScope::Thread { .. }) => {
                self.start_thread_snapshot(request, out)
            }
            (SurfaceId::MessageComposer, SurfaceScope::Channel { .. }) => {
                self.emit_composer_snapshot(request, out)
            }
            (SurfaceId::MessageChannelSummary, SurfaceScope::Channel { .. })
            | (SurfaceId::MessageMemberRoster, SurfaceScope::MemberRoster { .. })
            | (SurfaceId::MessageTopicList, SurfaceScope::TopicList { .. }) => {
                self.start_storage_snapshot(request, out)
            }
            (SurfaceId::MessageParticipantReceipts, SurfaceScope::ParticipantReceipts { .. }) => {
                self.start_receipt_snapshot(request, out)
            }
            _ => Err(ImError::Parse(
                "surface resync surface/scope mismatch".to_string(),
            )),
        }
    }

    fn emit_composer_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let SurfaceScope::Channel { channel_id } = &request.scope else {
            return Err(ImError::Parse(
                "composer snapshot requires channel scope".to_string(),
            ));
        };
        let surface_epoch = self.action_surface_epoch()?;
        let mutation = self
            .state
            .composer_projector
            .snapshot(ComposerVm {
                channel_id: channel_id.clone(),
                state: ComposerState::Ready,
                draft_policy: "message.compose.text.v1".to_string(),
                actions: vec![ActionBinding {
                    action_ref: format!("disabled:im.send-message:{channel_id}"),
                    kind: "im.send-message".to_string(),
                    input_shape: "message.compose.text.v1".to_string(),
                    surface_epoch,
                    enabled: false,
                    disabled_reason: Some("unsupportedByAuthorityV1".to_string()),
                }],
                capability_epoch: surface_epoch,
            })
            .map_err(|error| ImError::Parse(format!("composer projection: {error}")))?;
        out.push(frame_for_mutation("surface-composer", mutation)?);
        Ok(())
    }

    fn start_storage_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let operation = match (&request.surface, &request.scope) {
            (SurfaceId::MessageChannelSummary, SurfaceScope::Channel { channel_id }) => {
                StorageOp::Get(GetSpec {
                    table: "channel",
                    key_col: "id",
                    key_val: SqlValue::Text(channel_id.clone()),
                })
            }
            (SurfaceId::MessageMemberRoster, SurfaceScope::MemberRoster { channel_id, .. }) => {
                StorageOp::Scan(ScanSpec {
                    table: "channel_member",
                    limit: Some(200),
                    filter: Some(("channel_id", SqlValue::Text(channel_id.clone()))),
                    order_by: MEMBER_ROSTER_ORDER,
                })
            }
            (
                SurfaceId::MessageTopicList,
                SurfaceScope::TopicList {
                    parent_channel_id, ..
                },
            ) => StorageOp::Scan(ScanSpec {
                table: "channel",
                limit: Some(100),
                filter: Some(("root_id", SqlValue::Text(parent_channel_id.clone()))),
                order_by: TOPIC_LIST_ORDER,
            }),
            _ => {
                return Err(ImError::Parse(
                    "surface has no storage snapshot producer".to_string(),
                ))
            }
        };
        let corr = self.alloc_corr_internal();
        self.state
            .corr_map
            .insert(corr, CorrelationContext::SurfaceStorageSnapshot { request });
        out.push(Effect::Persist {
            corr,
            ops: vec![operation],
        });
        Ok(())
    }

    fn start_thread_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let SurfaceScope::Thread {
            channel_id,
            root_message_id,
        } = &request.scope
        else {
            return Err(ImError::Parse(
                "thread snapshot requires thread scope".to_string(),
            ));
        };
        let corr = self.alloc_corr_internal();
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": channel_id,
            "reply_id": root_message_id,
            "req_id": format!("surface-thread:{}", request.scope.canonical_key()),
            "page_number": 1,
            "page_size": 100,
        }))
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        let effects = crate::commands::handle_outbound(
            "im_get_replies",
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        if effects.len() != 1 || !matches!(effects.first(), Some(Effect::Http { .. })) {
            return Err(ImError::Parse(
                "im_get_replies must produce exactly one HTTP effect".to_string(),
            ));
        }
        self.state
            .corr_map
            .insert(corr, CorrelationContext::SurfaceThreadSnapshot { request });
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }

    fn start_receipt_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let SurfaceScope::ParticipantReceipts {
            channel_id: _,
            message_id,
            receipt_kind,
        } = &request.scope
        else {
            return Err(ImError::Parse(
                "receipt snapshot requires receipt scope".to_string(),
            ));
        };
        if *receipt_kind != ReceiptKind::Read {
            return self.emit_receipt_forbidden(request, out);
        }
        let corr = self.alloc_corr_internal();
        let payload = serde_json::to_vec(&serde_json::json!({
            "post_ids": [message_id],
            "req_id": format!("surface-receipt:{}", request.scope.canonical_key()),
        }))
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        let effects = crate::commands::handle_outbound(
            "im_post_read_list",
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        if effects.len() != 1 || !matches!(effects.first(), Some(Effect::Http { .. })) {
            return Err(ImError::Parse(
                "im_post_read_list must produce exactly one HTTP effect".to_string(),
            ));
        }
        self.state
            .corr_map
            .insert(corr, CorrelationContext::SurfaceReceiptSnapshot { request });
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }

    fn emit_receipt_forbidden(
        &mut self,
        request: SurfaceResyncRequest,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let SurfaceScope::ParticipantReceipts {
            channel_id,
            message_id,
            receipt_kind,
        } = &request.scope
        else {
            return Err(ImError::Parse(
                "receipt snapshot requires receipt scope".to_string(),
            ));
        };
        let request_data = ParticipantReceiptsRequest::new(
            channel_id,
            message_id,
            *receipt_kind,
            "surface-resync",
        );
        let mutation = if self
            .state
            .participant_receipts_projector
            .current_view(channel_id, message_id, *receipt_kind)
            .is_some()
        {
            self.state.participant_receipts_projector.patch(
                request_data,
                ParticipantReceiptsData::forbidden(),
                None,
            )
        } else {
            self.state
                .participant_receipts_projector
                .snapshot_forbidden(request_data)
        }
        .map_err(|error| ImError::Parse(format!("receipt unavailable projection: {error}")))?;
        out.push(frame_for_mutation("surface-receipt", mutation)?);
        Ok(())
    }

    pub(crate) fn handle_surface_storage_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let rows = storage_rows(outcome);
        let mutation = match (&request.surface, &request.scope) {
            (SurfaceId::MessageChannelSummary, SurfaceScope::Channel { channel_id }) => {
                let view =
                    channel_summary_view(channel_id, rows.first(), self.action_surface_epoch()?);
                if self
                    .state
                    .channel_summary_projector
                    .current_view(channel_id)
                    .is_some()
                {
                    self.state
                        .channel_summary_projector
                        .patch_settings_or_lifecycle(view, None)
                } else {
                    self.state.channel_summary_projector.snapshot(view)
                }
                .map_err(|error| ImError::Parse(format!("channel summary projection: {error}")))?
            }
            (SurfaceId::MessageMemberRoster, SurfaceScope::MemberRoster { channel_id, cursor }) => {
                let view =
                    member_roster_view(channel_id, cursor, &rows, self.action_surface_epoch()?);
                if self
                    .state
                    .member_roster_projector
                    .current_view(channel_id, cursor)
                    .is_some()
                {
                    self.state
                        .member_roster_projector
                        .patch_membership_lifecycle(view, None)
                } else {
                    self.state.member_roster_projector.snapshot(view)
                }
                .map_err(|error| ImError::Parse(format!("member roster projection: {error}")))?
            }
            (
                SurfaceId::MessageTopicList,
                SurfaceScope::TopicList {
                    parent_channel_id,
                    cursor,
                },
            ) => {
                let view = topic_list_view(
                    parent_channel_id,
                    cursor,
                    &rows,
                    self.action_surface_epoch()?,
                );
                if self
                    .state
                    .topic_list_projector
                    .current_view(parent_channel_id, cursor)
                    .is_some()
                {
                    self.state.topic_list_projector.patch_topic_link(view, None)
                } else {
                    self.state.topic_list_projector.snapshot(view)
                }
                .map_err(|error| ImError::Parse(format!("topic list projection: {error}")))?
            }
            _ => {
                return Err(ImError::Parse(
                    "unexpected storage surface snapshot".to_string(),
                ))
            }
        };
        out.push(frame_for_mutation("surface-storage", mutation)?);
        Ok(())
    }

    pub(crate) fn handle_surface_thread_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let SurfaceScope::Thread {
            channel_id,
            root_message_id,
        } = &request.scope
        else {
            return Err(ImError::Parse(
                "thread snapshot requires thread scope".to_string(),
            ));
        };
        let raw = match outcome {
            PortOutcome::Ok(reply) => crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref())
                .map_err(|error| ImError::Parse(format!("thread response envelope: {error}")))?,
            PortOutcome::Err(_) => return Ok(()),
        };
        let body: Value = serde_json::from_slice(raw.as_ref())
            .map_err(|error| ImError::Parse(format!("thread response JSON: {error}")))?;
        let projection = crate::render_ready_replies::finalize_projection(
            &crate::render_ready_replies::ReplyProjectionRequest {
                req_id: String::new(),
                channel_id: channel_id.clone(),
                root_hint: root_message_id.clone(),
                mode: crate::render_ready_replies::ReplyProjectionMode::Snapshot,
                revision: 1,
                viewer_user_id: self.config.auth_user_id.clone(),
            },
            crate::render_ready_replies::extract_flat_replies(
                &body,
                self.config.auth_user_id.as_str(),
            ),
        );
        if !projection.channel_id.is_empty() && projection.channel_id != *channel_id {
            return Err(ImError::Parse(
                "thread authority channel does not match scope".to_string(),
            ));
        }
        let view = thread_detail_view(
            channel_id,
            root_message_id,
            projection.nodes,
            projection.reply_ids,
            projection.reply_count,
            projection.has_more,
            self.action_surface_epoch()?,
        )?;
        let mutation = match self
            .state
            .thread_detail_projector
            .current_revision(channel_id, root_message_id)
        {
            Some(revision) => self
                .state
                .thread_detail_projector
                .replace(view, revision, None),
            None => self.state.thread_detail_projector.snapshot(view),
        }
        .map_err(|error| ImError::Parse(format!("thread projection: {error}")))?;
        out.push(frame_for_mutation("surface-thread", mutation)?);
        Ok(())
    }

    pub(crate) fn handle_surface_receipt_snapshot(
        &mut self,
        request: SurfaceResyncRequest,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let SurfaceScope::ParticipantReceipts {
            channel_id,
            message_id,
            receipt_kind,
        } = &request.scope
        else {
            return Err(ImError::Parse(
                "receipt snapshot requires receipt scope".to_string(),
            ));
        };
        let data = crate::module::action::participant_receipt_data(outcome, message_id);
        let request_data = ParticipantReceiptsRequest::new(
            channel_id,
            message_id,
            *receipt_kind,
            "surface-resync",
        );
        let mutation = if self
            .state
            .participant_receipts_projector
            .current_view(channel_id, message_id, *receipt_kind)
            .is_some()
        {
            self.state
                .participant_receipts_projector
                .patch(request_data, data, None)
        } else {
            self.state
                .participant_receipts_projector
                .snapshot(request_data, data)
        }
        .map_err(|error| ImError::Parse(format!("receipt projection: {error}")))?;
        out.push(frame_for_mutation("surface-receipt", mutation)?);
        Ok(())
    }
}

fn frame_for_mutation(prefix: &str, mutation: SurfaceMutation) -> Result<Effect, ImError> {
    let transaction_id = format!(
        "{prefix}-{}-{}",
        mutation.scope.canonical_key(),
        mutation.revision
    );
    let frame = crate::projector::build_interaction_frame(transaction_id, vec![mutation])
        .map_err(|error| ImError::Serialize(error.to_string()))?;
    crate::projector::emit_interaction_frame(frame)
        .map_err(|error| ImError::Serialize(error.to_string()))
}

fn storage_rows(outcome: &PortOutcome) -> Vec<Value> {
    let PortOutcome::Ok(reply) = outcome else {
        return Vec::new();
    };
    serde_json::from_slice::<Value>(reply.0.as_ref())
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

fn channel_summary_view(
    channel_id: &str,
    row: Option<&Value>,
    surface_epoch: u64,
) -> ChannelSummaryVm {
    let row = row.unwrap_or(&Value::Null);
    let title = text(row, &["display_name", "displayName"]);
    let title = if title.is_empty() {
        channel_id.to_string()
    } else {
        title
    };
    ChannelSummaryVm {
        channel_id: channel_id.to_string(),
        title: ChannelTitleRef {
            text: title.clone(),
        },
        avatar: ChannelAvatarRef {
            resource: None,
            user_ids: Vec::new(),
            fallback_text: title,
        },
        notice_summary: optional_text(row, &["header"]),
        purpose: optional_text(row, &["purpose"]),
        lifecycle: lifecycle(row),
        member_count: nonnegative_u32(row, &["member_count", "memberCount"]),
        online_count: None,
        topic_count: nonnegative_u32(row, &["thread_count", "topic_count", "topicCount"]),
        notification_mode: NotificationMode::All,
        actions: vec![disabled_action(
            "channel-settings",
            channel_id,
            surface_epoch,
        )],
        settings_version: nonnegative_u64(row, &["updated_at", "updateAt"]),
    }
}

fn member_roster_view(
    channel_id: &str,
    cursor: &str,
    rows: &[Value],
    surface_epoch: u64,
) -> MemberRosterVm {
    let items: Vec<MemberRowVm> = rows
        .iter()
        .filter_map(|row| {
            let user_id = text(row, &["user_id", "userId", "id"]);
            (!user_id.is_empty()).then(|| MemberRowVm {
                user: UserRefVm {
                    user_id: user_id.clone(),
                },
                nickname: optional_text(row, &["nick_name", "nickName", "user_name"]),
                role: member_role(text(row, &["role"]).as_str()),
                presence: crate::interaction_frame::PresenceState::Offline,
                membership_state: MembershipState::Active,
                actions: vec![disabled_action(
                    "member-manage",
                    user_id.as_str(),
                    surface_epoch,
                )],
            })
        })
        .collect();
    MemberRosterVm {
        channel_id: channel_id.to_string(),
        member_count: items.len() as u32,
        online_count: None,
        items,
        page: page(cursor),
        state: SurfaceLoadState::Ready,
    }
}

fn topic_list_view(
    parent_channel_id: &str,
    cursor: &str,
    rows: &[Value],
    surface_epoch: u64,
) -> TopicListVm {
    let items: Vec<TopicRowVm> = rows
        .iter()
        .filter_map(|row| {
            let topic_channel_id = text(row, &["id", "channel_id", "channelId"]);
            let root_message_id = text(row, &["root_post_id", "rootPostId"]);
            if topic_channel_id.is_empty() || root_message_id.is_empty() {
                return None;
            }
            let title = text(row, &["display_name", "displayName"]);
            let title = if title.is_empty() {
                topic_channel_id.clone()
            } else {
                title
            };
            Some(TopicRowVm {
                topic_channel_id: topic_channel_id.clone(),
                root_message_id: root_message_id.clone(),
                title: ChannelTitleRef {
                    text: title.clone(),
                },
                root_message_preview: PreviewVm::Text {
                    author: None,
                    text: root_message_id,
                },
                last_activity_at: integer(row, &["last_post_at", "lastPostAt"]),
                message_count: nonnegative_u32(row, &["topic_msg_count", "topicMsgCount"]),
                avatar: ChannelAvatarRef {
                    resource: None,
                    user_ids: Vec::new(),
                    fallback_text: title,
                },
                lifecycle: lifecycle(row),
                actions: vec![disabled_action(
                    "topic-open",
                    topic_channel_id.as_str(),
                    surface_epoch,
                )],
            })
        })
        .collect();
    TopicListVm {
        parent_channel_id: parent_channel_id.to_string(),
        topic_count: items.len() as u32,
        items,
        page: page(cursor),
        state: SurfaceLoadState::Ready,
    }
}

fn thread_detail_view(
    channel_id: &str,
    root_message_id: &str,
    nodes: Vec<Value>,
    ordered_reply_ids: Vec<String>,
    reply_count: usize,
    has_more: bool,
    surface_epoch: u64,
) -> Result<ThreadDetailVm, ImError> {
    let mut action_sequence = 0u64;
    let mut message = |row: &Value| {
        let mut vm = crate::projector::timeline::message_from_render_ready(
            row,
            channel_id,
            surface_epoch,
            true,
            || {
                action_sequence = action_sequence.saturating_add(1);
                Ok(format!("disabled-thread-{action_sequence}"))
            },
        )
        .map_err(|error| ImError::Parse(format!("thread message projection: {error}")))?;
        for binding in &mut vm.actions {
            binding.enabled = false;
            binding.disabled_reason = Some("unsupportedByAuthorityV1".to_string());
        }
        Ok(vm)
    };
    let root_row = nodes
        .iter()
        .find(|row| text(row, &["msgId", "id"]) == root_message_id);
    let root = root_row.map(&mut message).transpose()?.ok_or_else(|| {
        ImError::Parse("thread authority response omitted root message".to_string())
    })?;
    let mut thread_nodes = BTreeMap::new();
    for reply_id in &ordered_reply_ids {
        let Some(row) = nodes
            .iter()
            .find(|row| text(row, &["msgId", "id"]) == *reply_id)
        else {
            continue;
        };
        thread_nodes.insert(reply_id.clone(), message(row)?);
    }
    let visible_ids: Vec<String> = ordered_reply_ids
        .into_iter()
        .filter(|id| thread_nodes.contains_key(id))
        .collect();
    Ok(ThreadDetailVm {
        channel_id: channel_id.to_string(),
        root,
        reply_count: reply_count.max(visible_ids.len()) as u32,
        ordered_reply_ids: visible_ids,
        nodes: thread_nodes,
        page: WindowPageVm {
            window_token: format!("thread:{root_message_id}"),
            has_older: has_more,
            has_newer: false,
            has_more,
        },
        state: SurfaceLoadState::Ready,
    })
}

fn disabled_action(kind: &str, identity: &str, surface_epoch: u64) -> ActionBinding {
    ActionBinding {
        action_ref: format!("disabled:{kind}:{identity}"),
        kind: kind.to_string(),
        input_shape: "object".to_string(),
        surface_epoch,
        enabled: false,
        disabled_reason: Some("unsupportedByAuthorityV1".to_string()),
    }
}

fn page(cursor: &str) -> WindowPageVm {
    WindowPageVm {
        window_token: cursor.to_string(),
        has_older: false,
        has_newer: false,
        has_more: false,
    }
}

fn lifecycle(row: &Value) -> ChannelLifecycleVm {
    let closed = integer(row, &["delete_at", "deleteAt"]) > 0 || integer(row, &["is_remove"]) > 0;
    ChannelLifecycleVm {
        state: if closed { "closed" } else { "active" }.to_string(),
        reason: None,
    }
}

fn member_role(role: &str) -> ChannelRole {
    match role {
        "OWNER" | "CREATOR" => ChannelRole::Owner,
        "ADMIN" | "MANAGER" | "BOSS" => ChannelRole::Admin,
        _ => ChannelRole::Member,
    }
}

fn text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn optional_text(value: &Value, keys: &[&str]) -> Option<String> {
    let value = text(value, keys);
    (!value.is_empty()).then_some(value)
}

fn integer(value: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
        .unwrap_or_default()
}

fn nonnegative_u32(value: &Value, keys: &[&str]) -> u32 {
    integer(value, keys).clamp(0, u32::MAX as i64) as u32
}

fn nonnegative_u64(value: &Value, keys: &[&str]) -> u64 {
    integer(value, keys).clamp(0, crate::interaction_frame::MAX_SAFE_JSON_INTEGER as i64) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_resync_registers_attachment_before_the_first_snapshot() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let scope = SurfaceScope::Timeline {
            channel_id: "ch00000000000000000000000a".to_string(),
            window_token: "latest".to_string(),
        };
        let request = SurfaceResyncRequest {
            surface: SurfaceId::MessageTimeline,
            scope: scope.clone(),
        };
        let mut sink = EffectSink::new();

        module
            .handle_surface_resync(
                &serde_json::to_vec(&request).expect("surface request serializes"),
                1,
                &mut sink,
            )
            .expect("timeline resync starts an authoritative local query");

        assert!(module.state.timeline_projector.is_attached(&scope));
        assert!(sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })));
    }

    #[test]
    fn composer_resync_emits_one_typed_snapshot_without_io() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let request = SurfaceResyncRequest {
            surface: SurfaceId::MessageComposer,
            scope: SurfaceScope::Channel {
                channel_id: "ch00000000000000000000000b".to_string(),
            },
        };
        let mut sink = EffectSink::new();

        module
            .handle_surface_resync(
                &serde_json::to_vec(&request).expect("surface request serializes"),
                1,
                &mut sink,
            )
            .expect("composer resync emits an authoritative snapshot");

        assert_eq!(
            sink.as_slice().len(),
            1,
            "composer attach must remain O(1) and I/O-free"
        );
        let Effect::Emit { event } = &sink.as_slice()[0] else {
            panic!("composer attach must emit exactly one typed Frame")
        };
        let envelope: Value = serde_json::from_slice(event.0.as_ref()).expect("frame JSON");
        let mutation = &envelope["data"]["mutations"][0];
        assert_eq!(mutation["surface"], "message.composer");
        assert_eq!(mutation["scope"]["kind"], "channel");
        assert_eq!(mutation["revision"], 1);
        assert_eq!(mutation["mutation"]["kind"], "composerSnapshot");
        assert_eq!(mutation["mutation"]["view"]["state"], "ready");
        assert_eq!(
            mutation["mutation"]["view"]["draftPolicy"],
            "message.compose.text.v1"
        );
        assert_eq!(mutation["mutation"]["view"]["actions"][0]["enabled"], false);
    }
}
