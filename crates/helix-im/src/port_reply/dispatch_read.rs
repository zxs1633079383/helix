use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::older_context::LoadOlderState;
use crate::state::CorrelationContext;
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
        out: &mut EffectSink,
    ) {
        match outcome {
            PortOutcome::Ok(reply) => {
                // 剥信封 → 取 wire Post 数组；信封/base64 畸形 → 空批 → stop（零信任不 panic）。
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
                        out.push(crate::older_context::emit_older_loaded_for_viewer(
                            &state,
                            self.config.auth_user_id.as_str(),
                        ));
                    }
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(error = ?e, "load_older_context postContext http failed");
                // 收尾：把当前已收集的更早消息（可能为空）emit，前端结束 loading。
                // 用空批喂一轮触发 stop(has_more=false) → 已收集态被 sort+truncate+定 has_more。
                state.ingest_round(&[]);
                out.push(crate::older_context::emit_older_loaded_for_viewer(
                    &state,
                    self.config.auth_user_id.as_str(),
                ));
            }
        }
    }
}
