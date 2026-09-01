//! 文字接龙 PortReply：HTTP/WS authority 先过同一 projection barrier，再发布事件。

use crate::chain::{self, ChainAuthority, ChainMutation, ChainMutationState, ChainRequest};
use crate::error::ImError;
use crate::state::{ChannelId, CorrelationContext};
use helix_core::effect::Effect;
use helix_core::tick::{PortError, PortOutcome};
use helix_core::EffectSink;

impl super::ImModule {
    /// 处理接龙 HTTP 回包：transport/business 不确定时先持久化 RECONCILING，再走 reconcile。
    pub(crate) fn handle_chain_http_reply(
        &mut self,
        request: ChainRequest,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(reply) => {
                match chain::decode_http_authority(reply.0.as_ref(), &request) {
                    Ok(authority) => self.queue_chain_authority(request, authority, out),
                    Err(chain::ChainReplyError::Rejected { code }) => {
                        if request.is_mutation() {
                            self.queue_chain_mutation_status(
                                request,
                                ChainMutationState::Rejected,
                                None,
                                Some(code.as_str()),
                                false,
                                out,
                            )?;
                        } else if let Some(req_id) = Self::chain_req_id(&request) {
                            out.push(crate::query::read_relay::emit_read_error(&req_id, &code));
                        }
                        Ok(())
                    }
                    Err(chain::ChainReplyError::Invalid { message }) => {
                        let retry_reconcile = request.command != "post_chain_reconcile";
                        if request.is_mutation() {
                            self.queue_chain_mutation_status(
                                request,
                                ChainMutationState::Reconciling,
                                None,
                                Some("AUTHORITY_UNKNOWN"),
                                retry_reconcile,
                                out,
                            )?;
                        } else if let Some(req_id) = Self::chain_req_id(&request) {
                            out.push(crate::query::read_relay::emit_read_error(
                                &req_id,
                                "AUTHORITY_INVALID",
                            ));
                        } else {
                            tracing::warn!(command = %request.command, error = %message, "chain read authority rejected");
                        }
                        Ok(())
                    }
                }
            }
            PortOutcome::Err(error) => {
                if !request.is_mutation() {
                    if let Some(req_id) = Self::chain_req_id(&request) {
                        out.push(crate::query::read_relay::emit_read_error(
                            &req_id,
                            "CHAIN_READ_FAILED",
                        ));
                    } else {
                        tracing::warn!(command = %request.command, error = ?error, "chain read HTTP failed");
                    }
                    return Ok(());
                }
                let (state, code, retry_reconcile) = match error {
                    PortError::Timeout | PortError::Network | PortError::Http(500..=599) => (
                        ChainMutationState::Reconciling,
                        "TRANSPORT_UNKNOWN",
                        request.command != "post_chain_reconcile",
                    ),
                    PortError::Http(_) => (ChainMutationState::Rejected, "HTTP_REJECTED", false),
                    PortError::Storage(_) | PortError::Other(_) => (
                        ChainMutationState::Reconciling,
                        "TRANSPORT_UNKNOWN",
                        request.command != "post_chain_reconcile",
                    ),
                };
                self.queue_chain_mutation_status(
                    request,
                    state,
                    None,
                    Some(code),
                    retry_reconcile,
                    out,
                )
            }
        }
    }

    /// 从接龙 command 的 Host-owned payload 提取结构化读回 req_id。
    fn chain_req_id(request: &ChainRequest) -> Option<String> {
        request
            .payload
            .get("req_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    /// 把 authority 编译为 ChainSummary/Entry/window/viewer/mutation 单事务写集。
    fn queue_chain_authority(
        &mut self,
        request: ChainRequest,
        authority: ChainAuthority,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if request.command != "post_chain_reconcile"
            && authority.event_id.as_ref().is_some_and(|event_id| {
                self.state.seen_chain_event_ids.contains(event_id)
                    || self.state.pending_chain_event_ids.contains(event_id)
            })
        {
            return Ok(());
        }
        if request.command != "post_chain_get"
            && request.command != "post_chain_reconcile"
            && request.command != "post_chain_update_draft"
            && request.command != "post_chain_mark_read"
            && authority.event_seq.is_none()
            && authority.revision > 0
            && self
                .state
                .chain_revisions
                .get(&authority.chain_id)
                .is_some_and(|revision| authority.revision <= *revision)
        {
            return Ok(());
        }
        let mut cursor_seq = None;
        if let Some(event_seq) = authority.event_seq {
            let channel_id = ChannelId::from_str(&authority.channel_id).ok_or_else(|| {
                ImError::Parse("chain authority has invalid channel_id".to_string())
            })?;
            let channel = self
                .state
                .channels
                .entry(channel_id)
                .or_insert_with(|| crate::channel::Channel::new(channel_id, 0));
            // Canonical channel_stream_event may already have advanced this shared cursor; in that
            // case retain the chain authority for its own projection instead of dropping it as dup.
            if event_seq > channel.cursor.value() && !channel.admit_chain_event_seq(event_seq, out)
            {
                return Ok(());
            }
            cursor_seq = Some(event_seq);
        }
        let mutation_state = request
            .client_mutation_id
            .as_ref()
            .map(|_| ChainMutationState::Confirmed);
        let mut ops = chain::persist_ops(&authority, &request, mutation_state, None);
        if let Some(event_seq) = cursor_seq {
            let channel_id = ChannelId::from_str(&authority.channel_id).ok_or_else(|| {
                ImError::Parse("chain authority has invalid channel_id".to_string())
            })?;
            // Chain 与普通 post 共用此 monotonic cursor；不创建 chain 专用 eventSeq。
            ops.push(crate::acl::to_effect::advance_cursor_op(
                channel_id, event_seq,
            ));
        }
        if ops.is_empty() {
            return Err(ImError::Parse(
                "chain authority produced empty persist set".to_string(),
            ));
        }
        if let Some(event_id) = authority.event_id.as_ref() {
            self.state.pending_chain_event_ids.insert(event_id.clone());
        }
        let corr = self.alloc_corr_internal();
        let event_name = if request.command == "post_chain_reconcile" {
            chain::success_event_name(&request.command).to_string()
        } else {
            authority
                .event_type
                .clone()
                .unwrap_or_else(|| request.command.clone())
        };
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ChainPersist {
                request: Box::new(request),
                authority: Box::new(authority),
                event_name,
                mutation_state,
                error_code: None,
            },
        );
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }

    /// 为 RECONCILING/REJECTED mutation 写入 mutation 表，成功后才允许发状态事件。
    fn queue_chain_mutation_status(
        &mut self,
        request: ChainRequest,
        state: ChainMutationState,
        entry_id: Option<String>,
        error_code: Option<&str>,
        retry_reconcile: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(client_mutation_id) = request.client_mutation_id.as_deref() else {
            return Ok(());
        };
        let corr = self.alloc_corr_internal();
        let op = chain::mutation_op(
            &request,
            client_mutation_id,
            state,
            entry_id.as_deref(),
            error_code,
        );
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ChainMutationPersist {
                request: Box::new(request),
                state,
                entry_id,
                error_code: error_code.map(str::to_string),
                retry_reconcile,
            },
        );
        out.push(Effect::PersistAtomic {
            corr,
            ops: vec![op],
        });
        Ok(())
    }

    /// PersistAtomic 成功后推进既有 Channel cursor，并发布唯一 chain event。
    pub(crate) fn handle_chain_persist_reply(
        &mut self,
        request: ChainRequest,
        authority: ChainAuthority,
        event_name: String,
        mutation_state: Option<ChainMutationState>,
        error_code: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // WS canonical stream may have committed the shared cursor before this chain authority
        // reply arrives; the chain projection still needs its own durable tables and terminal event.
        if authority
            .event_id
            .as_ref()
            .is_some_and(|event_id| self.state.seen_chain_event_ids.contains(event_id))
        {
            return Ok(());
        }
        if !matches!(outcome, PortOutcome::Ok(_)) {
            if let Some(event_id) = authority.event_id.as_ref() {
                self.state.pending_chain_event_ids.remove(event_id);
                self.state.seen_chain_event_ids.remove(event_id);
            }
            if let Some(client_mutation_id) = request.client_mutation_id.as_deref() {
                if let Some(mutation) = self.state.chain_mutations.get_mut(client_mutation_id) {
                    mutation.state = ChainMutationState::Reconciling;
                    mutation.error_code = Some("PERSIST_FAILED".to_string());
                }
            }
            tracing::warn!(chain_id = %authority.chain_id, "chain projection persist failed; suppress event");
            return Ok(());
        }
        if let Some(event_seq) = authority.event_seq {
            let channel_id = ChannelId::from_str(&authority.channel_id).ok_or_else(|| {
                ImError::Parse("chain authority has invalid channel_id".to_string())
            })?;
            let cursor_already_committed = self
                .state
                .channels
                .get(&channel_id)
                .is_some_and(|channel| event_seq <= channel.cursor.value());
            let committed = if cursor_already_committed {
                true
            } else {
                self.state
                    .channels
                    .get_mut(&channel_id)
                    .map(|channel| channel.commit_contiguous_after_atomic(event_seq, out))
                    .transpose()?
                    .unwrap_or(false)
            };
            if !committed {
                // Persist 已落库但 cursor 不再连续时，不能把 eventId 留在 pending 集合；否则
                // 后续 WS/sync 重放会被永久去重，既没有 Emit 也没有补偿入口。
                if let Some(event_id) = authority.event_id.as_ref() {
                    self.state.pending_chain_event_ids.remove(event_id);
                    self.state.seen_chain_event_ids.remove(event_id);
                }
                tracing::warn!(chain_id = %authority.chain_id, event_seq = event_seq.0, "chain cursor commit no longer contiguous");
                return Ok(());
            }
        }
        if let Some(event_id) = authority.event_id.as_ref() {
            self.state.pending_chain_event_ids.remove(event_id);
            self.state.seen_chain_event_ids.insert(event_id.clone());
        }
        if authority.revision > 0 {
            self.state
                .chain_revisions
                .insert(authority.chain_id.clone(), authority.revision);
        }
        self.update_chain_mutation(
            &request,
            mutation_state,
            authority.entry.as_ref().map(|entry| entry.entry_id.clone()),
            error_code.clone(),
        );
        if request.command == "post_chain_get" {
            if let Some(req_id) = request
                .payload
                .get("req_id")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            {
                out.push(crate::query::read_relay::emit_read_body(
                    req_id,
                    authority.raw.clone(),
                ));
            }
            return Ok(());
        }
        let event_name = if event_name.starts_with("im:post_chain:") {
            event_name.as_str()
        } else {
            chain::success_event_name(&event_name)
        };
        let bytes = chain::event_bytes(
            event_name,
            &request,
            Some(&authority),
            mutation_state,
            error_code.as_deref(),
        )?;
        out.push(Effect::Emit {
            event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(bytes)),
        });
        Ok(())
    }

    /// PersistAtomic 成功后结算 RECONCILING/REJECTED，并按需发起同 operationId reconcile。
    pub(crate) fn handle_chain_mutation_persist_reply(
        &mut self,
        request: ChainRequest,
        state: ChainMutationState,
        entry_id: Option<String>,
        error_code: Option<String>,
        retry_reconcile: bool,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            tracing::warn!(command = %request.command, "chain mutation state persist failed; suppress event");
            return Ok(());
        }
        self.update_chain_mutation(&request, Some(state), entry_id, error_code.clone());
        let event_name = if state == ChainMutationState::Rejected {
            chain::rejected_event_name(&request.command)
        } else {
            "im:post_chain:reconcile"
        };
        let bytes = chain::event_bytes(
            event_name,
            &request,
            None,
            Some(state),
            error_code.as_deref(),
        )?;
        out.push(Effect::Emit {
            event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(bytes)),
        });
        if retry_reconcile && state == ChainMutationState::Reconciling {
            let reconcile = reconcile_request(&request)?;
            self.start_chain_http(reconcile, out)?;
        }
        Ok(())
    }

    /// 只有 PortReply 成功后才改变内存 mutation 镜像，保持 durable/renderer 一致。
    fn update_chain_mutation(
        &mut self,
        request: &ChainRequest,
        state: Option<ChainMutationState>,
        entry_id: Option<String>,
        error_code: Option<String>,
    ) {
        let (Some(client_mutation_id), Some(operation_id), Some(state)) = (
            request.client_mutation_id.as_ref(),
            request.operation_id.as_ref(),
            state,
        ) else {
            return;
        };
        self.state.chain_mutations.insert(
            client_mutation_id.clone(),
            ChainMutation {
                client_mutation_id: client_mutation_id.clone(),
                operation_id: operation_id.clone(),
                state,
                entry_id,
                error_code,
            },
        );
    }
}

/// 从原始 mutation request 派生同 operationId 的 reconcile command。
fn reconcile_request(request: &ChainRequest) -> Result<ChainRequest, ImError> {
    let mut payload = MapExt::from_request(request);
    payload.insert(
        "operation_id".to_string(),
        serde_json::json!(request.operation_id.clone().unwrap_or_default()),
    );
    let bytes =
        serde_json::to_vec(&payload).map_err(|error| ImError::Serialize(error.to_string()))?;
    chain::request_from_command("post_chain_reconcile", &bytes)
}

/// 构造 reconcile 所需的最小 canonical payload。
struct MapExt;

impl MapExt {
    /// 过滤原始 append/create 字段，只保留 reconcile 合同。
    fn from_request(request: &ChainRequest) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "channel_id".to_string(),
            serde_json::json!(request.channel_id),
        );
        map.insert("chain_id".to_string(), serde_json::json!(request.chain_id));
        if let Some(value) = &request.client_mutation_id {
            map.insert("client_mutation_id".to_string(), serde_json::json!(value));
        }
        if let Some(value) = &request.device_id {
            map.insert("device_id".to_string(), serde_json::json!(value));
        }
        map
    }
}
