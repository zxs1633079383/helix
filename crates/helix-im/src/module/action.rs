//! SurfaceRuntime action 的 application 编排。
//!
//! registry 只做 lease/actor/replay 决策；本模块把已审计 binding 映射到既有 canonical command。
//! 查询型 `viewReceipt` 仅在 HTTP 权威回包投影成带同一 causation 的 Frame 后让 renderer 完成；
//! 写族则继续等待对应 WS/sync authority 与 PersistOk，不能把 HTTP ack 当作业务完成。

use std::collections::{BTreeMap, BTreeSet};

use helix_core::effect::Effect;
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::Value;

use crate::error::ImError;
use crate::interaction_frame::{
    ActionInvocation, ActionInvocationEnvelope, ChannelRole, ParticipantReceiptRowVm, ReceiptKind,
    ReceiptStatus, SurfaceDataVm, SurfaceId, SurfaceLoadState, SurfaceMutation,
    TypedSurfaceMutation, UserRefVm, MAX_SAFE_JSON_INTEGER,
};
use crate::projector::action::{
    emit_action_receipt, ActionAuthorization, ActionBindingGrant, ActionDispatch, ActionEvaluation,
};
use crate::projector::participant_receipts::{
    ParticipantReceiptsData, ParticipantReceiptsRequest, MAX_PARTICIPANT_RECEIPT_ROWS,
};
use crate::state::CorrelationContext;
use crate::ImModule;

/// action ref 是短时 renderer capability，而不是长期凭据。新 timeline Frame 会撤销同 scope 旧 ref；
/// TTL 仍保留为失焦/断线/旧页面的第二道 fail-closed 防线。
const ACTION_BINDING_TTL_MS: u64 = 60_000;

enum PreparedAction {
    Routed {
        corr: helix_core::Correlation,
        context: CorrelationContext,
        effects: Vec<Effect>,
    },
    Forward {
        payload: Vec<u8>,
    },
    Retry {
        payload: Vec<u8>,
        window_token: String,
    },
}

impl ImModule {
    /// 把已验证的 timeline mutation 中可执行的 binding 登记为 actor-scoped lease。
    ///
    /// renderer 拿到的 ref 不包含 message/channel/command；真正的 dispatch context 只在这个 registry
    /// 内由同一 Frame 的 typed VM 固定。每个新 revision 先撤销同 scope 旧 grant，因此 registry 不会
    /// 因快速重投影积累到容量上限。
    pub(crate) fn register_timeline_action_bindings(
        &mut self,
        mutation: &SurfaceMutation,
        now_ms: u64,
    ) -> Result<(), ImError> {
        if mutation.surface != SurfaceId::MessageTimeline {
            return Ok(());
        }
        let scope = mutation.scope.clone();
        if !scope.valid_for(SurfaceId::MessageTimeline) {
            return Err(ImError::Parse(
                "timeline action binding has an invalid surface scope".to_string(),
            ));
        }
        let window_token = match &scope {
            crate::interaction_frame::SurfaceScope::Timeline { window_token, .. } => window_token,
            _ => {
                return Err(ImError::Parse(
                    "timeline action binding requires a timeline scope".to_string(),
                ))
            }
        };
        let items = match &mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => &view.items,
            TypedSurfaceMutation::TimelinePatch { patch } => &patch.view.items,
            _ => {
                return Err(ImError::Parse(
                    "timeline action binding requires a timeline mutation".to_string(),
                ))
            }
        };
        let actor_id = self.config.auth_user_id.clone();
        if actor_id.is_empty() {
            // TimelineProjector must have rendered all bindings disabled in this state. There is no trusted actor
            // against which a ref could safely be bound.
            return Ok(());
        }
        let surface_epoch = self.action_surface_epoch()?;
        self.state.action_registry.revoke_scope(
            actor_id.as_str(),
            SurfaceId::MessageTimeline,
            &scope,
            surface_epoch,
        );

        let expires_at_ms = now_ms.checked_add(ACTION_BINDING_TTL_MS).ok_or_else(|| {
            ImError::Parse("timeline action binding TTL overflowed Tick time".to_string())
        })?;
        for item in items {
            for binding in &item.actions {
                if !binding.enabled {
                    continue;
                }
                if binding.surface_epoch != surface_epoch {
                    return Err(ImError::Parse(
                        "timeline action binding surfaceEpoch does not match the current runtime epoch"
                            .to_string(),
                    ));
                }
                let dispatch = ActionDispatch::from_timeline_binding(
                    binding.kind.as_str(),
                    item.channel_id.as_str(),
                    item.id.as_str(),
                    window_token.as_str(),
                )
                .ok_or_else(|| {
                    ImError::Parse(format!(
                        "timeline enabled action '{}' has no authority mapping",
                        binding.kind
                    ))
                })?;
                self.state
                    .action_registry
                    .grant(
                        ActionBindingGrant {
                            actor_id: actor_id.clone(),
                            surface: SurfaceId::MessageTimeline,
                            scope: scope.clone(),
                            surface_epoch: binding.surface_epoch,
                            revision: mutation.revision,
                            expires_at_ms,
                            binding: binding.clone(),
                            dispatch,
                        },
                        now_ms,
                    )
                    .map_err(|error| ImError::Parse(format!("action binding grant: {error}")))?;
            }
        }
        Ok(())
    }

    /// 在非 query 路径发出 timeline Frame 前，同步更新 registry 的 revision lease。
    ///
    /// `TimelineProjector` 的 locate/receipt patch 会复用现有 view 里的 opaque actionRef；虽然 ref
    /// 不变，但 renderer 观察到的是新 revision。Frame 与 registry 必须在同一个编排边界推进，不能
    /// 让端侧回退到旧 revision 猜测授权。
    pub(crate) fn build_registered_timeline_frame(
        &mut self,
        frame_id: String,
        mutations: Vec<SurfaceMutation>,
        now_ms: u64,
    ) -> Result<Effect, ImError> {
        for mutation in &mutations {
            self.register_timeline_action_bindings(mutation, now_ms)?;
        }
        let frame = crate::projector::build_interaction_frame(frame_id, mutations)
            .map_err(|error| ImError::Serialize(error.to_string()))?;
        crate::projector::emit_interaction_frame(frame)
            .map_err(|error| ImError::Serialize(error.to_string()))
    }

    /// 受理 FFI/Web 送入的严格 envelope。`accepted` 只代表 canonical work 已成功建模并登记；
    /// 它绝不直接下发 participant receipt VM，也不让端侧把 accepted 当作 action 完成。
    pub(crate) fn handle_action_invocation(
        &mut self,
        payload: &[u8],
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let envelope: ActionInvocationEnvelope = serde_json::from_slice(payload)
            .map_err(|error| ImError::Parse(format!("action invocation payload: {error}")))?;
        envelope
            .validate()
            .map_err(|error| ImError::Parse(format!("action invocation payload: {error}")))?;

        let trusted_actor_id = self.config.auth_user_id.clone();
        let authorization = self
            .state
            .action_registry
            .authorize_for_actor(&envelope, trusted_actor_id.as_str(), now_ms)
            .map_err(|error| ImError::Parse(format!("action authorization: {error}")))?;

        match authorization {
            ActionAuthorization::Rejected(receipt) => {
                let evaluation = {
                    let (registry, receipts) = (
                        &mut self.state.action_registry,
                        &mut self.state.action_receipts,
                    );
                    registry
                        .commit_authorization(ActionAuthorization::Rejected(receipt), receipts)
                        .map_err(|error| ImError::Parse(format!("action receipt: {error}")))?
                };
                debug_assert!(matches!(evaluation, ActionEvaluation::Rejected(_)));
                self.emit_pending_action_receipts(out)
            }
            ActionAuthorization::Accepted(authorized) => {
                // Prepare before accepting: an unsupported/malformed canonical command cannot consume a requestId
                // and leave an accepted action with no eventual authoritative reply.
                let prepared = self.prepare_action(
                    &authorized.dispatch,
                    &envelope.invocation,
                    authorized.accepted.causation_id.as_str(),
                )?;
                let evaluation = {
                    let (registry, receipts) = (
                        &mut self.state.action_registry,
                        &mut self.state.action_receipts,
                    );
                    registry
                        .commit_authorization(ActionAuthorization::Accepted(authorized), receipts)
                        .map_err(|error| ImError::Parse(format!("action receipt: {error}")))?
                };
                self.emit_pending_action_receipts(out)?;
                if matches!(evaluation, ActionEvaluation::Accepted(_)) {
                    match prepared {
                        PreparedAction::Routed {
                            corr,
                            context,
                            effects,
                        } => {
                            if let CorrelationContext::InteractionMutationAck { pending } = &context
                            {
                                self.state.pending_authority_actions.insert(
                                    (pending.kind, pending.message_id.clone()),
                                    pending.clone(),
                                );
                            }
                            self.state.corr_map.insert(corr, context);
                            for effect in effects {
                                out.push(effect);
                            }
                        }
                        PreparedAction::Forward { payload } => {
                            crate::forward::start(self, &payload, now_ms, out)?;
                        }
                        PreparedAction::Retry {
                            payload,
                            window_token,
                        } => {
                            crate::send::retry_send::handle_retry_send_for_window(
                                self,
                                &payload,
                                now_ms,
                                window_token,
                                out,
                            )?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// `post/read/list` 回包同时保留既有 legacy emits，并以同一 raw authority 数据产生 typed Frame。
    /// 即便远端失败/结构异常，也发布 `Failed`/`Forbidden` surface；这才是当前 action 的权威候选
    /// 终态，而非把 HTTP success 误当完成。
    pub(crate) fn handle_action_view_receipt_reply(
        &mut self,
        request_id: &str,
        channel_id: String,
        message_id: String,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        crate::port_reply_emit::emit_post_readers(request_id, outcome, out);

        let data = participant_receipt_data(outcome, message_id.as_str());
        let request =
            ParticipantReceiptsRequest::new(channel_id, message_id, ReceiptKind::Read, "action");
        let mutation = self
            .state
            .participant_receipts_projector
            .snapshot_with_causation(request, data, Some(request_id.to_string()))
            .map_err(|error| ImError::Parse(format!("action receipt projection: {error}")))?;
        let scope_key = mutation.scope.canonical_key();
        let revision = mutation.revision;
        let frame = crate::projector::build_interaction_frame(
            format!("action-receipt-{scope_key}-{revision}"),
            vec![mutation],
        )
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        out.push(
            crate::projector::emit_interaction_frame(frame)
                .map_err(|error| ImError::Serialize(error.to_string()))?,
        );
        if !self
            .state
            .action_registry
            .complete_after_terminal_frame(request_id)
        {
            tracing::warn!(
                request_id,
                "terminal action frame had no in-flight request ledger entry"
            );
        }
        Ok(())
    }

    pub(crate) fn action_surface_epoch(&self) -> Result<u64, ImError> {
        self.state
            .query_session_epoch
            .checked_add(1)
            .filter(|epoch| *epoch <= MAX_SAFE_JSON_INTEGER)
            .ok_or_else(|| {
                ImError::Parse("surface epoch exceeded the v1 exact-safe wire range".to_string())
            })
    }

    fn prepare_action(
        &mut self,
        dispatch: &ActionDispatch,
        invocation: &ActionInvocation,
        request_id: &str,
    ) -> Result<PreparedAction, ImError> {
        match dispatch {
            ActionDispatch::ViewReceipt {
                channel_id,
                message_id,
            } => {
                let corr = self.alloc_corr_internal();
                let payload = serde_json::to_vec(&serde_json::json!({
                    "post_ids": [message_id],
                    "req_id": request_id,
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
                Ok(PreparedAction::Routed {
                    corr,
                    context: CorrelationContext::InteractionViewReceipt {
                        request_id: request_id.to_string(),
                        channel_id: channel_id.clone(),
                        message_id: message_id.clone(),
                    },
                    effects,
                })
            }
            ActionDispatch::RevokeMessage { message_id, .. } => self.prepare_outbound_action(
                "im_revoke",
                serde_json::json!({ "post_id": message_id, "req_id": request_id }),
                dispatch
                    .pending_authority_action(request_id)
                    .expect("revoke dispatch must create pending authority"),
            ),
            ActionDispatch::RetryMessage {
                message_id,
                window_token,
                ..
            } => {
                let payload =
                    crate::send::retry_send::prepare_retry_send_payload(message_id, request_id)?;
                Ok(PreparedAction::Retry {
                    payload,
                    window_token: window_token.clone(),
                })
            }
            ActionDispatch::ToggleReaction { message_id, .. } => self.prepare_outbound_action(
                "im_quick_reply",
                serde_json::json!({
                    "post_id": message_id,
                    "emoji": action_input_string(invocation, "emoji")?,
                    "req_id": request_id,
                }),
                dispatch
                    .pending_authority_action(request_id)
                    .expect("reaction dispatch must create pending authority"),
            ),
            ActionDispatch::UrgentMessage { message_id, .. } => {
                let mut payload = serde_json::json!({
                    "post_id": message_id,
                    "target_ids": action_input_strings(invocation, "targetIds")?,
                    "req_id": request_id,
                });
                if let Some(message) = action_optional_input_string(invocation, "message")? {
                    payload["message"] = serde_json::Value::String(message);
                }
                self.prepare_outbound_action(
                    "im_urgent_post",
                    payload,
                    dispatch
                        .pending_authority_action(request_id)
                        .expect("urgent dispatch must create pending authority"),
                )
            }
            ActionDispatch::ConfirmUrgent { message_id, .. } => self.prepare_outbound_action(
                "im_urgent_confirm",
                serde_json::json!({ "post_id": message_id, "req_id": request_id }),
                dispatch
                    .pending_authority_action(request_id)
                    .expect("urgent confirm dispatch must create pending authority"),
            ),
            ActionDispatch::CancelUrgent { message_id, .. } => self.prepare_outbound_action(
                "im_urgent_cancel",
                serde_json::json!({ "post_id": message_id, "req_id": request_id }),
                dispatch
                    .pending_authority_action(request_id)
                    .expect("urgent cancel dispatch must create pending authority"),
            ),
            ActionDispatch::ForwardMessages {
                source_message_ids, ..
            } => {
                let payload = serde_json::to_vec(&serde_json::json!({
                    "post_ids": source_message_ids,
                    "target_channel_ids": action_input_strings(invocation, "targetChannelIds")?,
                    "mode": action_input_string(invocation, "mode")?,
                    "req_id": request_id,
                }))
                .map_err(|error| ImError::Serialize(error.to_string()))?;
                Ok(PreparedAction::Forward { payload })
            }
            ActionDispatch::Unsupported => Err(ImError::Parse(
                "action dispatch has no canonical authority mapping".to_string(),
            )),
        }
    }

    fn prepare_outbound_action(
        &mut self,
        command: &'static str,
        payload: serde_json::Value,
        pending: crate::projector::action::PendingAuthorityAction,
    ) -> Result<PreparedAction, ImError> {
        let corr = self.alloc_corr_internal();
        let payload =
            serde_json::to_vec(&payload).map_err(|error| ImError::Serialize(error.to_string()))?;
        let effects = crate::commands::handle_outbound(
            command,
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        if effects.len() != 1 || !matches!(effects.first(), Some(Effect::Http { .. })) {
            return Err(ImError::Parse(format!(
                "{command} must produce exactly one HTTP effect"
            )));
        }
        Ok(PreparedAction::Routed {
            corr,
            // HTTP is only transport acceptance. Until the authority event arrives this route intentionally
            // has no business-frame completion; the dedicated WS/sync handler owns that terminal barrier.
            context: CorrelationContext::InteractionMutationAck { pending },
            effects,
        })
    }

    fn emit_pending_action_receipts(&mut self, out: &mut EffectSink) -> Result<(), ImError> {
        for receipt in self.state.action_receipts.drain() {
            out.push(
                emit_action_receipt(receipt).map_err(|error| {
                    ImError::Serialize(format!("action receipt encode: {error}"))
                })?,
            );
        }
        Ok(())
    }
}

fn action_input_string(invocation: &ActionInvocation, key: &str) -> Result<String, ImError> {
    match invocation.input.get(key) {
        Some(SurfaceDataVm::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(ImError::Parse(format!(
            "action input '{key}' must be a non-empty string"
        ))),
    }
}

fn action_optional_input_string(
    invocation: &ActionInvocation,
    key: &str,
) -> Result<Option<String>, ImError> {
    match invocation.input.get(key) {
        None => Ok(None),
        Some(SurfaceDataVm::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(ImError::Parse(format!(
            "action input '{key}' must be a non-empty string"
        ))),
    }
}

fn action_input_strings(invocation: &ActionInvocation, key: &str) -> Result<Vec<String>, ImError> {
    match invocation.input.get(key) {
        Some(SurfaceDataVm::Array(values)) => values
            .iter()
            .map(|value| match value {
                SurfaceDataVm::String(value) if !value.is_empty() => Ok(value.clone()),
                _ => Err(ImError::Parse(format!(
                    "action input '{key}' must contain non-empty strings"
                ))),
            })
            .collect(),
        _ => Err(ImError::Parse(format!(
            "action input '{key}' must be an array"
        ))),
    }
}

pub(crate) fn participant_receipt_data(
    outcome: &PortOutcome,
    message_id: &str,
) -> ParticipantReceiptsData {
    let raw_body = match outcome {
        PortOutcome::Ok(reply) => {
            match crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw) => raw,
                Err(_) => return failed_receipt_data("response envelope decode failed"),
            }
        }
        PortOutcome::Err(_) => return failed_receipt_data("http request failed"),
    };
    let body: Value = match serde_json::from_slice(raw_body.as_ref()) {
        Ok(body) => body,
        Err(_) => return failed_receipt_data("response body was not valid JSON"),
    };
    if body.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        return failed_receipt_data("receipt authority did not return SUCCESS");
    }
    let Some(item) = body
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("postId")
                    .and_then(Value::as_str)
                    .is_some_and(|post_id| post_id == message_id)
            })
        })
    else {
        return failed_receipt_data("receipt authority omitted the requested message");
    };
    if item.get("status").and_then(Value::as_str) == Some("unavailable") {
        return forbidden_receipt_data(
            item.get("unavailableReason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .unwrap_or("receipt unavailable"),
        );
    }
    ready_receipt_data(item).unwrap_or_else(|reason| failed_receipt_data(reason.as_str()))
}

fn ready_receipt_data(item: &Value) -> Result<ParticipantReceiptsData, String> {
    let author_id = required_item_string(item, "authorUserId")?;
    let snapshot_id = required_item_string(item, "snapshotId")?;
    let snapshot_version = item
        .get("snapshotVersion")
        .and_then(Value::as_u64)
        .filter(|version| *version > 0 && *version <= MAX_SAFE_JSON_INTEGER)
        .ok_or_else(|| "receipt snapshotVersion is invalid".to_string())?;
    let member_values = item
        .get("memberIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "receipt memberIds is missing".to_string())?;
    let read_bits = item
        .get("readBits")
        .and_then(Value::as_str)
        .ok_or_else(|| "receipt readBits is missing".to_string())?;
    if member_values.len() > MAX_PARTICIPANT_RECEIPT_ROWS.saturating_add(1)
        || read_bits.len() != member_values.len()
        || !read_bits.bytes().all(|bit| bit == b'0' || bit == b'1')
    {
        return Err("receipt member snapshot and readBits are inconsistent".to_string());
    }

    let mut members = Vec::with_capacity(member_values.len());
    let mut unique = BTreeSet::new();
    let mut author_count = 0usize;
    for member in member_values {
        let member_id = member
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "receipt memberIds contains an invalid member".to_string())?;
        if !unique.insert(member_id) {
            return Err("receipt memberIds contains duplicate members".to_string());
        }
        if member_id == author_id {
            author_count += 1;
        }
        members.push(member_id);
    }
    if author_count > 1 {
        return Err("receipt memberIds contains the author more than once".to_string());
    }

    let mut completed = Vec::new();
    let mut pending = Vec::new();
    for (index, member_id) in members.into_iter().enumerate() {
        if member_id == author_id {
            continue;
        }
        // post/read/list only proves the message-time recipient snapshot, not current channel role. `Member` is
        // therefore the closed VM's neutral snapshot role, never a claim about current roster authority.
        let row = ParticipantReceiptRowVm {
            user: UserRefVm {
                user_id: member_id.to_string(),
            },
            member_role: ChannelRole::Member,
            status: if read_bits.as_bytes()[index] == b'1' {
                ReceiptStatus::Completed
            } else {
                ReceiptStatus::Pending
            },
            completed_at: None,
        };
        if row.status == ReceiptStatus::Completed {
            completed.push(row);
        } else {
            pending.push(row);
        }
    }
    if completed.len().saturating_add(pending.len()) > MAX_PARTICIPANT_RECEIPT_ROWS {
        return Err("receipt window exceeds the bounded surface contract".to_string());
    }
    let mut summary = BTreeMap::new();
    summary.insert(
        "state".to_string(),
        SurfaceDataVm::String("ready".to_string()),
    );
    summary.insert("snapshotId".to_string(), SurfaceDataVm::String(snapshot_id));
    summary.insert(
        "snapshotVersion".to_string(),
        SurfaceDataVm::Number(serde_json::Number::from(snapshot_version)),
    );
    summary.insert(
        "recipientCount".to_string(),
        SurfaceDataVm::Number(serde_json::Number::from(
            completed.len().saturating_add(pending.len()) as u64,
        )),
    );
    Ok(ParticipantReceiptsData {
        summary: SurfaceDataVm::Record(summary),
        completed,
        pending,
        state: SurfaceLoadState::Ready,
    })
}

fn required_item_string(item: &Value, field: &str) -> Result<String, String> {
    item.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("receipt {field} is missing"))
}

fn failed_receipt_data(reason: &str) -> ParticipantReceiptsData {
    receipt_state_data("failed", reason, SurfaceLoadState::Failed)
}

fn forbidden_receipt_data(reason: &str) -> ParticipantReceiptsData {
    receipt_state_data("forbidden", reason, SurfaceLoadState::Forbidden)
}

fn receipt_state_data(
    state: &str,
    reason: &str,
    surface_state: SurfaceLoadState,
) -> ParticipantReceiptsData {
    let mut summary = BTreeMap::new();
    summary.insert(
        "state".to_string(),
        SurfaceDataVm::String(state.to_string()),
    );
    summary.insert(
        "reason".to_string(),
        SurfaceDataVm::String(reason.to_string()),
    );
    ParticipantReceiptsData {
        summary: SurfaceDataVm::Record(summary),
        completed: Vec::new(),
        pending: Vec::new(),
        state: surface_state,
    }
}
