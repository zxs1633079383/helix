//! 最近消息 local-first 状态机。
//!
//! 这里只编排业务意图：本地覆盖判定、远端 `getLatestPost`、合并、缓存与统一投影。
//! Storage/HTTP 的物理执行仍由平台 driver 兑现。

use helix_core::effect::Effect;
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::Value;

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};

use super::MessageQueryRequest;

/// Internal causation sentinel carried through the existing correlated local-first pipeline.
/// It is consumed before Frame publication and never becomes renderer-visible causation.
const SURFACE_TIMELINE_RESYNC_CAUSATION: &str = "__helix_surface_timeline_resync";

mod data;

#[cfg(test)]
pub(crate) use data::parse_latest_posts_body;
#[cfg(test)]
use data::server_id;
use data::{
    classify_local_read, dedup_recent_rows, merge_recent_rows, message_key,
    parse_latest_posts_reply, parse_local_rows, sort_recent_rows_desc,
    stale_local_server_rows_delete_op, visible_remote_rows_and_cache_ops,
};

/// 平台提供的本地数据能力。它描述物理数据寿命，不承诺某次查询已经完整。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LocalStoreMode {
    #[default]
    Durable,
    Session,
    Disabled,
}

/// 某次本地读相对“最近消息窗口”的业务覆盖结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalReadCoverage {
    Complete,
    Partial,
    Miss,
    Unsupported,
}

/// `getLatestPost` 的权威基础窗口。服务端可能为同 segment 补回更多行，但不会因此证明更早窗口。
const REMOTE_RECENT_WINDOW: usize = 20;

/// 当前连接会话内、成功远端读取且成功缓存后的覆盖证明。
///
/// 只记消息身份，不拿“本地有 N 行”冒充完整。hello/disconnect 会清空；cache Persist 失败不记录。
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct RecentMessageCoverage {
    remote_keys_desc: Vec<String>,
    remote_exhausted: bool,
}

impl RecentMessageCoverage {
    fn from_remote(rows_desc: &[Value], received_count: usize) -> Option<Self> {
        let remote_exhausted = received_count < REMOTE_RECENT_WINDOW;
        // getLatestPost 满 20 后可能把同 segment 一并补回；多出来的行可用于本次展示，
        // 但不能把基础 20 窗口扩张成“服务端证明了 21+ 条完整”。
        let proven_len = if remote_exhausted {
            rows_desc.len()
        } else {
            rows_desc.len().min(REMOTE_RECENT_WINDOW)
        };
        let remote_keys_desc: Vec<String> = rows_desc
            .iter()
            .take(proven_len)
            .filter_map(message_key)
            .collect();
        if remote_keys_desc.is_empty() {
            return None;
        }
        Some(Self {
            remote_keys_desc,
            remote_exhausted,
        })
    }
}

impl ImModule {
    /// A durable mutation may change an already attached latest timeline without
    /// another renderer intent. Re-enter the existing local-first projector
    /// after the write acknowledgement instead of leaking a legacy `im:post:*`
    /// payload to a client-side reducer.
    pub(crate) fn refresh_attached_latest_timeline(
        &mut self,
        channel_id: ChannelId,
        causation_id: Option<String>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.refresh_attached_timeline(channel_id, "latest", causation_id, out)
    }

    /// Action authority is scope-bound: its causal terminal Frame must return to the exact
    /// timeline window that granted the opaque actionRef.
    pub(crate) fn refresh_attached_timeline(
        &mut self,
        channel_id: ChannelId,
        window_token: &str,
        causation_id: Option<String>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.refresh_attached_timeline_window_with_deferred_send(
            channel_id,
            window_token,
            causation_id,
            None,
            out,
        )
        .map(|_| ())
    }

    /// P1 may only release an ordinary text send's HTTP after the attached
    /// timeline has emitted a typed Frame from its durable local fact. When no
    /// timeline is attached there is no renderer transition to protect, so the
    /// caller receives `false` and may start HTTP directly after P1.
    pub(crate) fn refresh_attached_latest_timeline_with_deferred_send(
        &mut self,
        channel_id: ChannelId,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        self.refresh_attached_timeline_window_with_deferred_send(
            channel_id,
            "latest",
            causation_id,
            deferred_send_http,
            out,
        )
    }

    fn refresh_attached_timeline_window_with_deferred_send(
        &mut self,
        channel_id: ChannelId,
        window_token: &str,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: window_token.to_string(),
        };
        if !self.state.timeline_projector.is_attached(&scope) {
            return Ok(false);
        }
        let payload = serde_json::json!({
            "channel_id": channel_id.as_str(),
            "window_token": window_token,
            "limit": super::QUERY_MESSAGES_DEFAULT,
        });
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| ImError::Serialize(format!("attached timeline refresh: {error}")))?;
        self.dispatch_message_query_with_causation_and_deferred_send(
            &bytes,
            false,
            causation_id,
            deferred_send_http,
            out,
        )?;
        Ok(true)
    }

    /// `im_query_messages_by_channel` 的唯一异步入口。
    pub(crate) fn dispatch_message_query(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.dispatch_message_query_with_causation_and_deferred_send(payload, true, None, None, out)
    }

    /// Runs the normal authoritative local-first read but forces its terminal
    /// publication to be a complete snapshot. A detached renderer has no
    /// retained base revision, even when the in-memory projector still does.
    pub(crate) fn dispatch_surface_timeline_resync(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.dispatch_message_query_with_causation_and_deferred_send(
            payload,
            true,
            Some(SURFACE_TIMELINE_RESYNC_CAUSATION.to_string()),
            None,
            out,
        )
    }

    /// Runs the same local-first query as an explicit renderer request, while
    /// preserving command-owned causation and an optional internal send continuation.
    /// The renderer never supplies either value.
    fn dispatch_message_query_with_causation_and_deferred_send(
        &mut self,
        payload: &[u8],
        allow_remote_fallback: bool,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = super::parse_message_query(payload)?;
        let query_generation = self
            .state
            .begin_message_query_generation(request.channel_id, request.window_token.as_str());
        if self.local_store_mode == LocalStoreMode::Disabled {
            return self.start_remote_message_query(
                request,
                Vec::new(),
                query_generation,
                causation_id,
                deferred_send_http,
                out,
            );
        }

        let corr = self.alloc_corr_internal();
        out.push(super::build_message_query_from_request(&request, corr));
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryLocal {
                request: Box::new(request),
                query_session_epoch: self.state.query_session_epoch,
                query_generation,
                allow_remote_fallback,
                causation_id,
                deferred_send_http,
            },
        );
        Ok(())
    }

    pub(crate) fn handle_message_query_local_reply(
        &mut self,
        request: MessageQueryRequest,
        query_session_epoch: u64,
        query_generation: u64,
        allow_remote_fallback: bool,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let explicit_surface_resync =
            causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION);
        let causal_action_readback = causation_id.is_some() && !explicit_surface_resync;
        if !explicit_surface_resync
            && !causal_action_readback
            && !self.is_current_message_query(
                request.channel_id,
                &request.window_token,
                query_session_epoch,
                query_generation,
            )
        {
            return Ok(());
        }
        let mut local_rows_desc = match outcome {
            PortOutcome::Ok(reply) => match parse_local_rows(reply.0.as_ref()) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        channel_id = request.channel_id.as_str(),
                        error = ?error,
                        allow_remote_fallback,
                        "message query local scan reply malformed"
                    );
                    if !allow_remote_fallback {
                        out.push(self.emit_timeline_failed(
                            &request,
                            now_ms,
                            false,
                            causation_id,
                        )?);
                        self.emit_deferred_posts_create_after_timeline_frame(
                            deferred_send_http,
                            out,
                        )?;
                        return Ok(());
                    }
                    Vec::new()
                }
            },
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    allow_remote_fallback,
                    "message query local scan failed"
                );
                if !allow_remote_fallback {
                    out.push(self.emit_timeline_failed(&request, now_ms, false, causation_id)?);
                    self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
                    return Ok(());
                }
                Vec::new()
            }
        };
        sort_recent_rows_desc(&mut local_rows_desc);

        if !allow_remote_fallback {
            out.push(self.emit_timeline_snapshot_with_causation(
                &request,
                &local_rows_desc,
                now_ms,
                causation_id,
            )?);
            self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
            return Ok(());
        }

        let coverage = classify_local_read(
            self.local_store_mode,
            &request,
            &local_rows_desc,
            self.state.recent_message_coverage.get(&request.channel_id),
            self.message_query_has_known_gap(request.channel_id),
        );
        tracing::debug!(
            channel_id = request.channel_id.as_str(),
            ?coverage,
            local_rows = local_rows_desc.len(),
            "message query local coverage classified"
        );

        if coverage == LocalReadCoverage::Complete {
            out.push(self.emit_timeline_snapshot_with_causation(
                &request,
                &local_rows_desc,
                now_ms,
                causation_id,
            )?);
            self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
            return Ok(());
        }
        self.start_remote_message_query(
            request,
            local_rows_desc,
            query_generation,
            causation_id,
            deferred_send_http,
            out,
        )
    }

    pub(crate) fn handle_message_query_remote_reply(
        &mut self,
        request: MessageQueryRequest,
        local_rows_desc: Vec<Value>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let explicit_surface_resync =
            causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION);
        if !explicit_surface_resync
            && !self.is_current_message_query(
                request.channel_id,
                &request.window_token,
                query_session_epoch,
                query_generation,
            )
        {
            return Ok(());
        }
        let reply = match outcome {
            PortOutcome::Ok(reply) => reply,
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback failed"
                );
                out.push(self.emit_timeline_failed(
                    &request,
                    now_ms,
                    causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION),
                    causation_id,
                )?);
                self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
                return Ok(());
            }
        };

        let remote_posts = match parse_latest_posts_reply(reply) {
            Ok(posts) => posts,
            Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback returned invalid response"
                );
                out.push(self.emit_timeline_failed(
                    &request,
                    now_ms,
                    causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION),
                    causation_id,
                )?);
                self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
                return Ok(());
            }
        };
        let received_count = remote_posts.len();
        let (mut remote_rows_desc, mut cache_ops) = match visible_remote_rows_and_cache_ops(
            request.channel_id,
            remote_posts,
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback contained invalid posts"
                );
                out.push(self.emit_timeline_failed(
                    &request,
                    now_ms,
                    causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION),
                    causation_id,
                )?);
                self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
                return Ok(());
            }
        };
        dedup_recent_rows(&mut remote_rows_desc);
        sort_recent_rows_desc(&mut remote_rows_desc);

        let remote_exhausted = received_count < REMOTE_RECENT_WINDOW;
        let coverage = RecentMessageCoverage::from_remote(&remote_rows_desc, received_count);
        if remote_exhausted {
            if let Some(delete) = stale_local_server_rows_delete_op(
                request.channel_id,
                &local_rows_desc,
                &remote_rows_desc,
            ) {
                cache_ops.push(delete);
            }
        }
        let merged_rows_desc = merge_recent_rows(
            local_rows_desc,
            remote_rows_desc,
            remote_exhausted,
            request.limit as usize,
        );

        if self.local_store_mode == LocalStoreMode::Disabled || cache_ops.is_empty() {
            out.push(self.emit_timeline_snapshot_with_causation(
                &request,
                &merged_rows_desc,
                now_ms,
                causation_id,
            )?);
            self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
            return Ok(());
        }

        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: cache_ops,
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryCache {
                request: Box::new(request),
                merged_rows_desc: Box::new(merged_rows_desc),
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            },
        );
        Ok(())
    }

    pub(crate) fn handle_message_query_cache_reply(
        &mut self,
        request: MessageQueryRequest,
        merged_rows_desc: Vec<Value>,
        coverage: Option<RecentMessageCoverage>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let explicit_surface_resync =
            causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION);
        if !explicit_surface_resync
            && !self.is_current_message_query(
                request.channel_id,
                &request.window_token,
                query_session_epoch,
                query_generation,
            )
        {
            return Ok(());
        }
        match outcome {
            PortOutcome::Ok(_) => {
                if self.is_current_message_query(
                    request.channel_id,
                    &request.window_token,
                    query_session_epoch,
                    query_generation,
                ) {
                    if let Some(coverage) = coverage {
                        self.state
                            .recent_message_coverage
                            .insert(request.channel_id, coverage);
                    }
                }
            }
            PortOutcome::Err(error) => tracing::warn!(
                channel_id = request.channel_id.as_str(),
                error = ?error,
                "message query remote cache failed; returning remote result without coverage"
            ),
        }
        out.push(self.emit_timeline_snapshot_with_causation(
            &request,
            &merged_rows_desc,
            now_ms,
            causation_id,
        )?);
        self.emit_deferred_posts_create_after_timeline_frame(deferred_send_http, out)?;
        Ok(())
    }

    fn start_remote_message_query(
        &mut self,
        request: MessageQueryRequest,
        local_rows_desc: Vec<Value>,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": request.channel_id.as_str(),
            "timestamp": 0,
        }))
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        let mut effects = crate::commands::handle_outbound(
            "im_get_latest_post",
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        if effects.len() != 1 {
            return Err(ImError::Parse(format!(
                "im_get_latest_post expected one HTTP effect, got {}",
                effects.len()
            )));
        }
        let effect = effects
            .pop()
            .ok_or_else(|| ImError::Parse("im_get_latest_post produced no effect".to_string()))?;
        if !matches!(&effect, Effect::Http { .. }) {
            return Err(ImError::Parse(
                "im_get_latest_post did not produce Effect::Http".to_string(),
            ));
        }
        out.push(effect);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryRemote {
                request: Box::new(request),
                local_rows_desc: Box::new(local_rows_desc),
                query_session_epoch: self.state.query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            },
        );
        Ok(())
    }

    fn emit_deferred_posts_create_after_timeline_frame(
        &mut self,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(temporary_id) = deferred_send_http else {
            return Ok(());
        };
        let (channel_id, body) = self
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| {
                pending.body.as_ref().and_then(|body| {
                    body.get("channelId")
                        .and_then(serde_json::Value::as_str)
                        .and_then(ChannelId::from_str)
                        .map(|channel_id| (channel_id, body.clone()))
                })
            })
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "deferred posts/create missing pending send body: {}",
                    temporary_id.0
                ))
            })?;
        self.emit_posts_create_http(channel_id, temporary_id, &body, out)
    }

    fn message_query_has_known_gap(&self, channel_id: ChannelId) -> bool {
        let Some(channel) = self.state.channels.get(&channel_id) else {
            return false;
        };
        let behind_target = self
            .state
            .increment_target
            .get(&channel_id)
            .is_some_and(|target| channel.cursor.value() < *target);
        behind_target || channel.inflight_sync.is_some() || !channel.buffer.is_empty()
    }

    fn is_current_message_query(
        &self,
        channel_id: ChannelId,
        window_token: &str,
        query_session_epoch: u64,
        query_generation: u64,
    ) -> bool {
        query_session_epoch == self.state.query_session_epoch
            && self.state.is_current_message_query_generation(
                channel_id,
                window_token,
                query_generation,
            )
    }

    /// local-first 已完成排序、去重、权限与 render-ready shaping 后的唯一 timeline 出口。
    /// `rows_desc` 是当前有界窗口的倒序查询结果；这里反转一次并交给 typed projector，端侧不再
    /// 消费 `im:messages:query_result` 或 raw storage row。
    fn emit_timeline_snapshot_with_causation(
        &mut self,
        request: &MessageQueryRequest,
        rows_desc: &[Value],
        now_ms: u64,
        causation_id: Option<String>,
    ) -> Result<Effect, ImError> {
        let explicit_resync = causation_id.as_deref() == Some(SURFACE_TIMELINE_RESYNC_CAUSATION);
        let terminal_action_request = if explicit_resync {
            None
        } else {
            causation_id.clone()
        };
        let rows_asc = Value::Array(rows_desc.iter().rev().cloned().collect());
        let mut shaped = crate::render_ready::shape_message_rows_for_viewer(
            &rows_asc,
            self.config.auth_user_id.as_str(),
        );
        close_topic_links(
            &mut shaped,
            request.channel_id.as_str(),
            &self.state.dialog_list_frame,
        );
        let rows = shaped.as_array().ok_or_else(|| {
            ImError::Parse("render-ready timeline rows must be an array".to_string())
        })?;
        let mut timeline_request =
            crate::projector::timeline::TimelineWindowRequest::latest_with_window_token(
                request.channel_id.as_str(),
                request.window_token.as_str(),
            );
        if self
            .state
            .recent_message_coverage
            .get(&request.channel_id)
            .is_some_and(|coverage| !coverage.remote_exhausted)
        {
            // A full authoritative recent window proves only that older rows
            // may exist. Preserve that proof in the render contract so shells
            // can expose the bounded load-older action without guessing from
            // the visible row count.
            timeline_request.page.has_older = true;
            timeline_request.page.has_more = true;
        }
        timeline_request.surface_epoch = self.action_surface_epoch()?;
        timeline_request.action_authority_available = !self.config.auth_user_id.is_empty();
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: timeline_request.channel_id.clone(),
            window_token: timeline_request.window_token.clone(),
        };
        let mutation = if explicit_resync {
            self.state
                .timeline_projector
                .snapshot_from_render_ready(timeline_request, rows)
        } else if self.state.timeline_projector.current_view(&scope).is_some() {
            self.state.timeline_projector.patch_from_render_ready(
                timeline_request,
                rows,
                causation_id,
            )
        } else {
            self.state
                .timeline_projector
                .snapshot_from_render_ready_with_causation(timeline_request, rows, causation_id)
        }
        .map_err(|error| ImError::Parse(format!("timeline projection: {error}")))?;
        self.register_timeline_action_bindings(&mutation, now_ms)?;
        let scope_key = mutation.scope.canonical_key();
        let revision = mutation.revision;
        let frame = crate::projector::build_interaction_frame(
            format!("timeline-{scope_key}-{revision}"),
            vec![mutation],
        )
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        let effect = crate::projector::emit_interaction_frame(frame)
            .map_err(|error| ImError::Serialize(error.to_string()))?;
        if let Some(request_id) = terminal_action_request {
            let is_forward_action = self
                .state
                .pending_forward_deliveries
                .contains_request(&request_id);
            let terminal_action_frame = if is_forward_action {
                self.state
                    .pending_forward_deliveries
                    .complete_target(&request_id, request.channel_id)
            } else {
                true
            };
            if terminal_action_frame {
                let action_completed = self
                    .state
                    .action_registry
                    .complete_after_terminal_frame(&request_id);
                if is_forward_action && !action_completed {
                    tracing::warn!(
                        request_id,
                        "timeline action frame had no in-flight request ledger entry"
                    );
                }
            }
        }
        Ok(effect)
    }

    fn emit_timeline_failed(
        &mut self,
        request: &MessageQueryRequest,
        now_ms: u64,
        explicit_resync: bool,
        causation_id: Option<String>,
    ) -> Result<Effect, ImError> {
        let mut failed =
            crate::projector::timeline::TimelineWindowRequest::latest_with_window_token(
                request.channel_id.as_str(),
                request.window_token.as_str(),
            );
        failed.surface_epoch = self.action_surface_epoch()?;
        failed.state = crate::interaction_frame::SurfaceLoadState::Failed;
        failed.action_authority_available = false;
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: failed.channel_id.clone(),
            window_token: failed.window_token.clone(),
        };
        let mutation = if explicit_resync {
            self.state
                .timeline_projector
                .snapshot_from_render_ready(failed, &[])
        } else if self.state.timeline_projector.current_view(&scope).is_some() {
            self.state
                .timeline_projector
                .patch_from_render_ready(failed, &[], causation_id.clone())
        } else {
            self.state
                .timeline_projector
                .snapshot_from_render_ready_with_causation(failed, &[], causation_id.clone())
        }
        .map_err(|error| ImError::Parse(format!("timeline failure projection: {error}")))?;
        self.register_timeline_action_bindings(&mutation, now_ms)?;
        let scope_key = mutation.scope.canonical_key();
        let revision = mutation.revision;
        let frame = crate::projector::build_interaction_frame(
            format!("timeline-{scope_key}-{revision}"),
            vec![mutation],
        )
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        let effect = crate::projector::emit_interaction_frame(frame)
            .map_err(|error| ImError::Serialize(error.to_string()))?;
        if let Some(request_id) =
            causation_id.filter(|request_id| request_id != SURFACE_TIMELINE_RESYNC_CAUSATION)
        {
            self.state
                .action_registry
                .complete_after_terminal_frame(&request_id);
        }
        Ok(effect)
    }
}

fn close_topic_links(
    shaped: &mut Value,
    parent_channel_id: &str,
    dialogs: &crate::dialog_frame::DialogListFrameState,
) {
    let Some(rows) = shaped.as_array_mut() else {
        return;
    };
    for row in rows {
        let Some(row_object) = row.as_object_mut() else {
            continue;
        };
        let Some(topic_channel_id) = row_object
            .get("topic")
            .and_then(Value::as_object)
            .and_then(|topic| {
                topic
                    .get("topicChannelId")
                    .or_else(|| topic.get("channelId"))
            })
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let Some((root_message_id, title)) = dialogs.topic_link_metadata(&topic_channel_id) else {
            continue;
        };
        let row_message_id = row_object
            .get("msgId")
            .or_else(|| row_object.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if row_message_id != root_message_id {
            continue;
        }
        row_object.insert(
            "topic".to_string(),
            serde_json::json!({
                "parentChannelId": parent_channel_id,
                "topicChannelId": topic_channel_id,
                "rootMessageId": root_message_id,
                "title": title,
            }),
        );
    }
}

#[cfg(test)]
#[path = "local_first_tests.rs"]
mod tests;
