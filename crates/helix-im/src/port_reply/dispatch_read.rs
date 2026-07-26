use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::older_context::LoadOlderState;
use crate::state::CorrelationContext;
use crate::{error::ImError, query::LocalStoreMode};
use helix_core::effect::Effect;
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;

impl ImModule {
    pub(super) fn handle_outbound_read_reply(
        &self,
        req_id: &String,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    out.push(crate::read_relay::emit_read_result(
                        req_id.as_str(),
                        raw_body.as_ref(),
                    ));
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
                self.emit_load_older_failed_frame(&state, now_ms, out)?;
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
            return self.emit_load_older_failed_frame(&state, now_ms, out);
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
                return self.emit_load_older_failed_frame(&state, now_ms, out);
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
            return self.emit_load_older_failed_frame(&state, now_ms, out);
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::LoadOlderCache { state });
        Ok(())
    }

    /// Schedules the exact bounded storage readback that is the only success Frame input.
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

    /// Builds the terminal G10a Frame from the durable readback while preserving Helix paging facts.
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
                        return self.emit_load_older_failed_frame(&state, now_ms, out);
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "load_older_context readback failed"
                );
                return self.emit_load_older_failed_frame(&state, now_ms, out);
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
        let page = crate::interaction_frame::WindowPageVm {
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

    /// Publishes a failed replaceView for the attached window without erasing its last good items.
    pub(super) fn emit_load_older_failed_frame(
        &mut self,
        state: &LoadOlderState,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mutation = self
            .state
            .timeline_projector
            .patch_failed_for_anchor(state.channel_id().as_str(), state.anchor_post_id())
            .map_err(|error| ImError::Parse(format!("load older failure projection: {error}")))?;
        let Some(mutation) = mutation else {
            return Ok(());
        };
        let frame_id = format!(
            "timeline-{}-{}",
            mutation.scope.canonical_key(),
            mutation.revision
        );
        out.push(self.build_registered_timeline_frame(frame_id, vec![mutation], now_ms)?);
        Ok(())
    }
}
