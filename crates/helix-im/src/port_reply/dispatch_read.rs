use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::older_context::LoadOlderState;
use crate::state::CorrelationContext;
use crate::{error::ImError, query::LocalStoreMode};
use helix_core::effect::{Effect, ScanOrder, ScanSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::Value;
use std::collections::HashSet;

const OLDER_NAVIGATION_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("create_at"),
    ScanOrder::desc("temporary_id"),
];
const NEWER_NAVIGATION_ORDER: &[ScanOrder] =
    &[ScanOrder::asc("create_at"), ScanOrder::asc("temporary_id")];
const LOCATE_NAVIGATION_ORDER: &[ScanOrder] =
    &[ScanOrder::asc("create_at"), ScanOrder::asc("temporary_id")];

impl ImModule {
    /// 发起 Timeline V3 authority HTTP，并注册唯一 correlation continuation。
    pub(crate) fn start_timeline_navigation_http(
        &mut self,
        state: crate::timeline_navigation::TimelineNavigationState,
        out: &mut EffectSink,
    ) {
        let corr = self.alloc_corr_internal();
        out.push(crate::timeline_navigation::navigation_http(
            &state,
            self.config.api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        ));
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TimelineNavigationHttp {
                state: Box::new(state),
            },
        );
    }

    /// 校验 V3 HTTP 页并把可见消息送入 matching Persist barrier。
    pub(super) fn handle_timeline_navigation_http_reply(
        &mut self,
        mut state: Box<crate::timeline_navigation::TimelineNavigationState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if locate_navigation_is_stale(&self.state.timeline_state, &state) {
            tracing::debug!(
                channel_id = state.channel_id().as_str(),
                "ignoring stale timeline locate HTTP correlation"
            );
            return Ok(());
        }
        let reply = match outcome {
            PortOutcome::Ok(reply) => reply,
            PortOutcome::Err(error) => {
                tracing::warn!(error = ?error, "timeline navigation HTTP failed");
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        let raw = match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(error = ?error, "timeline navigation envelope invalid");
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        if let Err(error) = state.ingest_http_body(&raw) {
            tracing::warn!(error = ?error, "timeline navigation body invalid");
            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            return Ok(());
        }
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. }
        ) {
            let rows = match crate::query::render_ready::locate::filter_visible_remote_rows(
                state.rows(),
                state.channel_id().as_str(),
                state.anchor_message_id(),
                self.config.auth_user_id.as_str(),
            ) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline locate target is not visible in HTTP authority"
                    );
                    self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    return Ok(());
                }
            };
            state.replace_rows(rows);
        }
        let (rows, ops) = match crate::query::local_first::visible_remote_rows_and_cache_ops(
            state.channel_id(),
            state.rows().to_vec(),
            &[],
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "timeline navigation rows could not become durable messages"
                );
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        state.replace_rows(rows);
        if self.local_store_mode == LocalStoreMode::Disabled {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                "timeline navigation requires durable storage before event publication"
            );
            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::TimelineNavigationCache { state });
        Ok(())
    }

    /// Publishes a bounded failed delta for the existing cursor-based Timeline window.
    pub(super) fn emit_timeline_navigation_failed_event(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let page_direction = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Older {
                anchor_post_id, ..
            } => Some(("older", anchor_post_id.as_str())),
            crate::timeline_navigation::TimelineNavigationKind::Newer {
                anchor_post_id, ..
            } => Some(("newer", anchor_post_id.as_str())),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => None,
        };
        if let Some((direction, anchor_post_id)) = page_direction {
            if let Some(request_id) = state.request_id() {
                let message = if direction == "older" {
                    "timeline older query failed before durable result"
                } else {
                    "timeline newer query failed before durable result"
                };
                out.push(crate::read_relay::emit_read_error(request_id, message));
                return Ok(());
            }
            let page = state.page();
            out.push(
                crate::event::timeline::page(
                    state.channel_id().as_str(),
                    state.window_token(),
                    direction,
                    "failed",
                    Vec::new(),
                    page.has_older,
                    page.has_newer,
                    Some(anchor_post_id),
                )?
                .into_effect(),
            );
            return Ok(());
        }
        let crate::timeline_navigation::TimelineNavigationKind::Locate {
            navigation_token, ..
        } = state.kind()
        else {
            return Ok(());
        };
        if !self.state.timeline_state.is_current_locate_navigation(
            state.channel_id().as_str(),
            state.window_token(),
            navigation_token,
        ) {
            return Ok(());
        }
        if let Some(request_id) = state.request_id() {
            out.push(crate::read_relay::emit_read_error(
                request_id,
                "timeline locate query failed before durable result",
            ));
        }
        Ok(())
    }

    /// PersistOk 后只读回页内权威行；该 readback 完成前不得发布成功事件。
    pub(super) fn schedule_timeline_navigation_readback(
        &mut self,
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let effect = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Older { .. } => {
                // Older 页面必须回读一个有界 message Scan；单行 Get 只能证明某条消息存在，
                // 无法证明返回页、去重、排序和 viewer 过滤都来自 durable storage。
                Effect::Persist {
                    corr,
                    ops: vec![StorageOp::Scan(older_navigation_scan_spec(&state))],
                }
            }
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => {
                // Newer 同样必须回读完整的尾部 bounded Scan，不能用单行 identity 旁路结算。
                Effect::Persist {
                    corr,
                    ops: vec![StorageOp::Scan(newer_navigation_scan_spec(&state))],
                }
            }
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => Effect::Persist {
                corr,
                ops: vec![StorageOp::Scan(locate_navigation_scan_spec(&state))],
            },
        };
        out.push(effect);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TimelineNavigationReadback { state },
        );
        Ok(())
    }

    /// 匹配的本地读回存在且身份一致时，释放唯一 production Timeline Delta。
    pub(super) fn handle_timeline_navigation_readback_reply(
        &mut self,
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if locate_navigation_is_stale(&self.state.timeline_state, &state) {
            tracing::debug!(
                channel_id = state.channel_id().as_str(),
                "ignoring stale timeline locate readback correlation"
            );
            return Ok(());
        }
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. }
        ) {
            let rows = match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                        Ok(rows) => {
                            newer_navigation_rows(&state, rows, self.config.auth_user_id.as_str())
                        }
                        Err(error) => {
                            tracing::warn!(
                                channel_id = state.channel_id().as_str(),
                                error = ?error,
                                "timeline newer durable read-back malformed"
                            );
                            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                            return Ok(());
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline newer durable read-back failed"
                    );
                    self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    return Ok(());
                }
            };
            if newer_http_has_rows(&state) && rows.is_empty() {
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            } else {
                match self.emit_timeline_navigation_newer_result(&state, &rows) {
                    Ok(effect) => out.push(effect),
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "timeline newer durable rows could not project"
                        );
                        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    }
                }
            }
            return Ok(());
        }
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Older { .. }
        ) {
            let rows = match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                        Ok(rows) => {
                            older_navigation_rows(&state, rows, self.config.auth_user_id.as_str())
                        }
                        Err(error) => {
                            tracing::warn!(
                                channel_id = state.channel_id().as_str(),
                                error = ?error,
                                "timeline older durable read-back malformed"
                            );
                            Vec::new()
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline older durable read-back failed"
                    );
                    Vec::new()
                }
            };
            // HTTP 只建立“期望有页”的事实；若该页确实有可见 rows，却没有任何
            // matching durable row，必须失败保旧，不能旁路用 HTTP rows 结算。
            if !state.rows().is_empty() && rows.is_empty() {
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            } else {
                out.push(self.emit_timeline_navigation_result(&state, &rows)?);
            }
            return Ok(());
        }
        let rows = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "timeline locate durable readback malformed"
                        );
                        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                        return Ok(());
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "timeline locate durable readback failed"
                );
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        let (target_message_id, _) = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Locate {
                target_message_id,
                navigation_token,
            } => (target_message_id.as_str(), navigation_token.as_str()),
            _ => return Ok(()),
        };
        let rows = match crate::query::render_ready::locate::normalize_durable_located_window(
            rows,
            state.channel_id().as_str(),
            target_message_id,
            self.config.auth_user_id.as_str(),
            state.page_size(),
        ) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "timeline locate durable rows failed closed"
                );
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        out.push(self.emit_timeline_locate_result(&state, &rows)?);
        Ok(())
    }

    /// 将 durable older rows 投影成同 reqId 的结构化 QueryMessagesResult。
    fn emit_timeline_navigation_result(
        &self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
    ) -> Result<Effect, ImError> {
        let request_id = state
            .request_id()
            .ok_or_else(|| ImError::Parse("timeline older result missing req_id".to_string()))?;
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &Value::Array(rows.to_vec()),
            self.config.auth_user_id.as_str(),
        );
        let page = state.page();
        Ok(crate::read_relay::emit_read_body(
            request_id,
            serde_json::json!({
                "reqId": request_id,
                "messages": shaped,
                "hasOlder": page.has_older,
                "hasNewer": page.has_newer,
                "olderCursor": Value::Null,
                "newerCursor": Value::Null,
            }),
        ))
    }

    /// 将 durable newer rows 合并到附着窗口，并以唯一 reqId 结果结算；不发布 Timeline 领域广播。
    fn emit_timeline_navigation_newer_result(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
    ) -> Result<Effect, ImError> {
        let request_id = state
            .request_id()
            .ok_or_else(|| ImError::Parse("timeline newer result missing req_id".to_string()))?;
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &Value::Array(rows.to_vec()),
            self.config.auth_user_id.as_str(),
        );
        let shaped_rows = shaped.as_array().ok_or_else(|| {
            ImError::Parse("timeline newer durable rows must shape to array".to_string())
        })?;
        let mut request = crate::timeline_state::TimelineWindowRequest::latest_with_window_token(
            state.channel_id().as_str(),
            state.window_token(),
        );
        request.page_size = state.page_size();
        request.page = state.page();
        self.state
            .timeline_state
            .patch_page_from_render_ready(
                request,
                shaped_rows,
                crate::timeline_state::TimelinePageMutation::Newer,
                state.request_id().map(str::to_string),
            )
            .map_err(|error| ImError::Parse(format!("timeline newer projection: {error}")))?;
        let page = state.page();
        Ok(crate::read_relay::emit_read_body(
            request_id,
            serde_json::json!({
                "reqId": request_id,
                "messages": shaped,
                "hasOlder": page.has_older,
                "hasNewer": page.has_newer,
                "olderCursor": Value::Null,
                "newerCursor": Value::Null,
            }),
        ))
    }

    /// 应用已读回的 locate 窗口并返回唯一 QueryMessagesResult，不发布领域广播。
    fn emit_timeline_locate_result(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
    ) -> Result<Effect, ImError> {
        let request_id = state
            .request_id()
            .ok_or_else(|| ImError::Parse("timeline locate result missing req_id".to_string()))?;
        let crate::timeline_navigation::TimelineNavigationKind::Locate {
            target_message_id,
            navigation_token,
        } = state.kind()
        else {
            return Err(ImError::Parse(
                "timeline locate result requires locate state".to_string(),
            ));
        };
        if !self.state.timeline_state.is_current_locate_navigation(
            state.channel_id().as_str(),
            state.window_token(),
            navigation_token,
        ) {
            return Err(ImError::Parse(
                "timeline locate result is stale".to_string(),
            ));
        }
        let mut request = crate::timeline_state::TimelineWindowRequest::latest_with_window_token(
            state.channel_id().as_str(),
            state.window_token(),
        );
        request.page_size = state.page_size();
        request.page = state.page();
        self.state
            .timeline_state
            .patch_page_from_render_ready(
                request,
                rows,
                crate::timeline_state::TimelinePageMutation::Locate {
                    target_message_id: target_message_id.to_string(),
                    navigation_token: navigation_token.to_string(),
                    activate: true,
                },
                Some(request_id.to_string()),
            )
            .map_err(|error| ImError::Parse(format!("timeline locate projection: {error}")))?;
        let page = state.page();
        Ok(crate::read_relay::emit_read_body(
            request_id,
            serde_json::json!({
                "reqId": request_id,
                "messages": rows,
                "hasOlder": page.has_older,
                "hasNewer": page.has_newer,
                "olderCursor": state.older_cursor().cloned().unwrap_or(Value::Null),
                "newerCursor": state.newer_cursor().cloned().unwrap_or(Value::Null),
            }),
        ))
    }

    /// 通用 HTTP 读回按命令身份投影，并保留原始 req_id 完成 Host 调用。
    pub(super) fn handle_outbound_read_reply(
        &self,
        req_id: &String,
        command: &str,
        channel_id: Option<&str>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    if command == "im_get_schedule" {
                        let body = serde_json::from_slice::<serde_json::Value>(&raw_body)
                            .unwrap_or(serde_json::Value::Null);
                        let schedules = crate::outbound::posts::read::schedule_projections(
                            channel_id.unwrap_or_default(),
                            &body,
                        );
                        out.push(crate::read_relay::emit_read_body(
                            req_id.as_str(),
                            serde_json::json!({
                                "ok": true,
                                "schedules": schedules,
                            }),
                        ));
                    } else {
                        out.push(crate::read_relay::emit_read_result(
                            req_id.as_str(),
                            raw_body.as_ref(),
                        ));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        req_id,
                        error = ?e,
                        "read reply envelope decode failed"
                    );
                    out.push(crate::read_relay::emit_read_error(
                        req_id,
                        "response envelope decode failed",
                    ));
                }
            },
            PortOutcome::Err(e) => {
                tracing::warn!(req_id = req_id.as_str(), error = ?e, "read outbound http failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "http request failed",
                ));
            }
        }
    }

    /// MV3-G02e 本地读回只按冻结的账号/频道命令投影，并通过非领域结果通道返回。
    pub(super) fn handle_draft_readback_reply(
        &self,
        command: Box<crate::draft::QueryDraftCommand>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let req_id = command.req_id.as_deref().unwrap_or_default();
        let PortOutcome::Ok(reply) = outcome else {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "draft read failed",
            ));
            return Ok(());
        };
        let rows = match helix_core::port_codec::rows_from_reply_bytes(&reply.0) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = ?error, "draft readback decode failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "draft readback decode failed",
                ));
                return Ok(());
            }
        };
        out.push(command.result_event(&rows)?.into_effect());
        Ok(())
    }

    pub(super) fn handle_todo_query_reply(&self, outcome: &PortOutcome, out: &mut EffectSink) {
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    out.push(crate::todo::emit_todo_updated(raw_body.as_ref()));
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "todo query reply envelope decode failed");
                }
            },
            PortOutcome::Err(e) => {
                tracing::warn!(error = ?e, "todo query outbound http failed");
            }
        }
    }

    pub(super) fn handle_load_older_context_reply(
        &mut self,
        mut state: Box<LoadOlderState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(reply) => {
                // 剥信封 → 取 wire Post 数组；信封/base64 畸形作为 invalid round fail-closed。
                let rows = unwrap_sync_envelope(reply.0.as_ref())
                    .map(|raw| crate::older_context::extract_post_rows(&raw))
                    .unwrap_or_default();
                match state.ingest_round(&rows) {
                    crate::older_context::RoundDecision::Continue => {
                        let next_corr = self.alloc_corr_internal();
                        let (_, body) = crate::older_context::build_post_context_body(&state);
                        out.push(crate::older_context::post_context_http_tracked(
                            &self.config.api_base_url,
                            &body,
                            next_corr,
                            self.state.connection_id.as_deref(),
                            state.request_id(),
                        ));
                        self.state
                            .corr_map
                            .insert(next_corr, CorrelationContext::LoadOlderContext { state });
                    }
                    crate::older_context::RoundDecision::Done => {
                        self.finish_load_older_http(state, now_ms, out)?;
                    }
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(error = ?e, "load_older_context postContext http failed");
                self.emit_load_older_failed_event(&state, now_ms, out)?;
            }
        }
        Ok(())
    }

    /// Converts a completed HTTP page into message writes and establishes the persistence barrier.
    fn finish_load_older_http(
        &mut self,
        state: Box<LoadOlderState>,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if state.failed() {
            return self.emit_load_older_failed_event(&state, now_ms, out);
        }
        let (_, ops) = match crate::query::local_first::visible_remote_rows_and_cache_ops(
            *state.channel_id(),
            state.older_rows(),
            &[],
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "load_older_context response could not become durable messages"
                );
                return self.emit_load_older_failed_event(&state, now_ms, out);
            }
        };
        if ops.is_empty() {
            return self.schedule_load_older_readback(state, out);
        }
        if self.local_store_mode == LocalStoreMode::Disabled {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                "load_older_context requires a writable store for durable timeline publication"
            );
            return self.emit_load_older_failed_event(&state, now_ms, out);
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::LoadOlderCache { state });
        Ok(())
    }

    /// 调度唯一可生成成功事件的有界存储读回。
    pub(super) fn schedule_load_older_readback(
        &mut self,
        state: Box<LoadOlderState>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let window_token = state.window_token().ok_or_else(|| {
            ImError::Parse("load older missing attached window token".to_string())
        })?;
        let request = crate::query::MessageQueryRequest {
            channel_id: *state.channel_id(),
            limit: state.readback_limit(),
            window_token: window_token.to_string(),
        };
        let corr = self.alloc_corr_internal();
        out.push(crate::query::build_message_query_from_request(
            &request, corr,
        ));
        self.state
            .corr_map
            .insert(corr, CorrelationContext::LoadOlderReadback { state });
        Ok(())
    }

    /// 从 durable readback 构造 G10a 终态事件并保留 Helix 分页事实。
    pub(super) fn handle_load_older_readback_reply(
        &mut self,
        state: Box<LoadOlderState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut rows_desc = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "load_older_context readback was malformed"
                        );
                        return self.emit_load_older_failed_event(&state, now_ms, out);
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "load_older_context readback failed"
                );
                return self.emit_load_older_failed_event(&state, now_ms, out);
            }
        };
        crate::query::local_first::sort_recent_rows_desc(&mut rows_desc);
        let window_token = state.window_token().ok_or_else(|| {
            ImError::Parse("load older missing attached window token".to_string())
        })?;
        let request = crate::query::MessageQueryRequest {
            channel_id: *state.channel_id(),
            limit: state.readback_limit(),
            window_token: window_token.to_string(),
        };
        let page = crate::timeline_state::WindowPage {
            window_token: window_token.to_string(),
            has_older: state.has_more(),
            has_newer: false,
            has_more: state.has_more(),
        };
        out.push(self.emit_timeline_snapshot_with_causation(
            &request,
            &rows_desc,
            now_ms,
            None,
            Some(page),
        )?);
        Ok(())
    }

    /// 发布上拉失败状态，但不擦除上一次成功窗口。
    pub(super) fn emit_load_older_failed_event(
        &mut self,
        state: &LoadOlderState,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        out.push(
            crate::event::timeline::page(
                state.channel_id().as_str(),
                state.window_token().unwrap_or("latest"),
                "older",
                "failed",
                Vec::new(),
                true,
                false,
                Some(state.anchor_post_id()),
            )?
            .into_effect(),
        );
        Ok(())
    }
}

/// 构造 older durable read-back 的 bounded message Scan。
fn older_navigation_scan_spec(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(state.page_size().saturating_add(1)),
        filter: Some((
            "channel_id",
            SqlValue::Text(state.channel_id().as_str().to_string()),
        )),
        order_by: OLDER_NAVIGATION_ORDER,
    }
}

/// 构造 newer durable read-back 的 bounded message Scan，尾部按复合键升序读取一条 lookahead。
fn newer_navigation_scan_spec(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(state.page_size().saturating_add(1)),
        filter: Some((
            "channel_id",
            SqlValue::Text(state.channel_id().as_str().to_string()),
        )),
        order_by: NEWER_NAVIGATION_ORDER,
    }
}

/// 构造 locate durable read-back 的 bounded channel Scan，目标和 viewer 由投影层复核。
fn locate_navigation_scan_spec(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(state.page_size().saturating_add(1)),
        filter: Some((
            "channel_id",
            SqlValue::Text(state.channel_id().as_str().to_string()),
        )),
        order_by: LOCATE_NAVIGATION_ORDER,
    }
}

/// 判断 locate correlation 是否已被同窗口更新的 navigation token 淘汰。
fn locate_navigation_is_stale(
    timeline_state: &crate::timeline_state::TimelineState,
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> bool {
    match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Locate {
            navigation_token, ..
        } => !timeline_state.is_current_locate_navigation(
            state.channel_id().as_str(),
            state.window_token(),
            navigation_token,
        ),
        _ => false,
    }
}

/// 过滤、去重、排序 durable older rows，避免 HTTP rows 成为成功结果旁路。
fn older_navigation_rows(
    state: &crate::timeline_navigation::TimelineNavigationState,
    rows: Vec<Value>,
    auth_user_id: &str,
) -> Vec<Value> {
    // Go 的空页/hasMore=false 是该次 older authority 的边界；不能用旧 cache
    // 中恰好位于 anchor 前的行重新推导一页，避免 stale storage 旁路污染结果。
    if state.rows().is_empty() {
        return Vec::new();
    }
    let Some(cursor) = (match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Older { cursor, .. } => Some(cursor),
        _ => None,
    }) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut filtered = rows
        .into_iter()
        .filter_map(|row| {
            let fields = crate::ws::parser::extract_post_fields(&row);
            if fields.channel_id != state.channel_id().as_str()
                || !crate::channel_write::post_updates_from_fields(
                    state.channel_id(),
                    &fields,
                    auth_user_id,
                )
                .visible
            {
                return None;
            }
            let key = durable_navigation_key(&row)?;
            if key.0 > cursor.create_at
                || (key.0 == cursor.create_at && key.1.as_str() >= cursor.temporary_id.as_str())
            {
                return None;
            }
            let identity = if !fields.id.is_empty() {
                format!("id:{}", fields.id)
            } else if !fields.temporary_id.is_empty() {
                format!("tmp:{}", fields.temporary_id)
            } else {
                return None;
            };
            seen.insert(identity).then_some((key, row))
        })
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| left.0.cmp(&right.0));
    filtered
        .into_iter()
        .take(state.page_size() as usize)
        .map(|(_, row)| row)
        .collect()
}

/// 判断 HTTP newer 页是否包含锚点之后的可见行；anchor-only 页仍是合法 empty terminal。
fn newer_http_has_rows(state: &crate::timeline_navigation::TimelineNavigationState) -> bool {
    let Some(cursor) = (match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Newer { cursor, .. } => Some(cursor),
        _ => None,
    }) else {
        return false;
    };
    state
        .rows()
        .iter()
        .filter_map(durable_navigation_key)
        .any(|key| {
            key.0 > cursor.create_at
                || (key.0 == cursor.create_at && key.1.as_str() > cursor.temporary_id.as_str())
        })
}

/// 过滤、去重、排序 durable newer rows，只保留锚点之后的页内可见消息。
fn newer_navigation_rows(
    state: &crate::timeline_navigation::TimelineNavigationState,
    rows: Vec<Value>,
    auth_user_id: &str,
) -> Vec<Value> {
    // HTTP empty/anchor-only 页是 Go 的明确终态，不能被旧 cache 中恰好存在的
    // 新侧行重新推导成一页，避免 stale storage 旁路污染 empty result。
    if !newer_http_has_rows(state) {
        return Vec::new();
    }
    let Some(cursor) = (match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Newer { cursor, .. } => Some(cursor),
        _ => None,
    }) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut filtered = rows
        .into_iter()
        .filter_map(|row| {
            let fields = crate::ws::parser::extract_post_fields(&row);
            if fields.channel_id != state.channel_id().as_str()
                || !crate::channel_write::post_updates_from_fields(
                    state.channel_id(),
                    &fields,
                    auth_user_id,
                )
                .visible
            {
                return None;
            }
            let key = durable_navigation_key(&row)?;
            if key.0 < cursor.create_at
                || (key.0 == cursor.create_at && key.1.as_str() <= cursor.temporary_id.as_str())
            {
                return None;
            }
            let identity = if !fields.id.is_empty() {
                format!("id:{}", fields.id)
            } else if !fields.temporary_id.is_empty() {
                format!("tmp:{}", fields.temporary_id)
            } else {
                return None;
            };
            seen.insert(identity).then_some((key, row))
        })
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| left.0.cmp(&right.0));
    filtered
        .into_iter()
        .take(state.page_size() as usize)
        .map(|(_, row)| row)
        .collect()
}

/// 从 snake/camel durable row 提取稳定分页键。
fn durable_navigation_key(row: &Value) -> Option<(i64, String)> {
    let create_at = row
        .get("create_at")
        .or_else(|| row.get("createAt"))
        .and_then(Value::as_i64)?;
    let temporary_id = row
        .get("temporary_id")
        .or_else(|| row.get("temporaryId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    Some((create_at, temporary_id.to_string()))
}
