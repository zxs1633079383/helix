//! UC-4.5 陌生 channel 单频道 hydration 编排。
//!
//! HTTP `incrementByChannelId` 仍保留通用 read relay，但其 `data=*IncrementChannel` 还会复用
//! WS increment 解析/应用规则。channel/member 确认落库后才启动既有 sync；terminal sync commit
//! 后从本地 channel/message 表读回并复用 render-ready query 投影，前端不参与业务计算。

use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, EffectSink};

impl ImModule {
    /// B-rest 测试 / driver 入口：直接喂一个解析好的 `increment_channel` 帧。
    pub fn ingest_increment(
        &mut self,
        inc: &crate::sync_session::IncrementChannel,
        out: &mut EffectSink,
    ) {
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, 0, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::increment_channel::apply_increment(&mut ctx, inc, out);
        });
    }

    fn ingest_increment_hydration(
        &mut self,
        inc: &crate::sync_session::IncrementChannel,
        persist_corr: Correlation,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> bool {
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, now_ms, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::increment_channel::apply_increment_hydration(
                &mut ctx,
                inc,
                persist_corr,
                out,
            )
        })
    }

    pub(crate) fn handle_increment_hydration_reply(
        &mut self,
        req_id: &str,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    out.push(crate::read_relay::emit_read_result(req_id, &raw_body));
                    let body: serde_json::Value = match serde_json::from_slice(&raw_body) {
                        Ok(body) => body,
                        Err(e) => {
                            tracing::warn!(req_id, error = ?e, "increment hydration body is not json");
                            return Ok(());
                        }
                    };
                    let Some(data) = body.get("data") else {
                        tracing::warn!(req_id, "increment hydration reply missing data");
                        return Ok(());
                    };
                    let Some(increment) = crate::ws::parser::parse_increment_channel(data) else {
                        tracing::warn!(
                            req_id,
                            "increment hydration reply has invalid IncrementChannel"
                        );
                        return Ok(());
                    };
                    let persist_corr = self.alloc_corr_internal();
                    let has_persist =
                        self.ingest_increment_hydration(&increment, persist_corr, now_ms, out);
                    // needSync 是服务端按其会话水位给的提示，不能覆盖本端持久化事实。独立窗口
                    // 首次见到频道时本地 cursor=0；只要仍落后 lastEventSeq，就必须从本地 cursor
                    // 拉 sync，把 events/messages 与 cursor 在同一 commit 链落稳后再做终态投影。
                    let local_cursor = self
                        .state
                        .channels
                        .get(&increment.channel_id)
                        .map(|channel| channel.cursor.value())
                        .unwrap_or(crate::state::Seq(0));
                    let requires_sync =
                        increment.need_sync || local_cursor < increment.last_event_seq;
                    if requires_sync {
                        self.state.need_sync_skip.remove(&increment.channel_id);
                    }
                    if has_persist {
                        self.state.corr_map.insert(
                            persist_corr,
                            CorrelationContext::IncrementHydrationPersist {
                                channel_id: increment.channel_id,
                                need_sync: requires_sync,
                                raw_increment: increment.raw.as_ref().to_vec(),
                            },
                        );
                    } else {
                        tracing::warn!(
                            channel_id = increment.channel_id.as_str(),
                            "increment hydration produced no durable writes; final projection suppressed"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(req_id, error = ?e, "increment hydration envelope decode failed");
                    out.push(crate::read_relay::emit_read_error(
                        req_id,
                        "response envelope decode failed",
                    ));
                }
            },
            PortOutcome::Err(e) => {
                tracing::warn!(req_id, error = ?e, "increment hydration http failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "http request failed",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn handle_increment_hydration_persist(
        &mut self,
        channel_id: ChannelId,
        need_sync: bool,
        raw_increment: Vec<u8>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                out.push(crate::acl::to_effect::emit_channel_increment(
                    channel_id,
                    raw_increment.as_ref(),
                ));
                if let Some(effect) = crate::query::emit_hydrated_dialog_result(&raw_increment) {
                    out.push(effect);
                }
                self.after_increment_persist(channel_id, need_sync, out)?;
            }
            PortOutcome::Err(e) => tracing::warn!(
                channel_id = channel_id.as_str(),
                error = ?e,
                "increment hydration channel/member persist failed"
            ),
        }
        Ok(())
    }

    fn after_increment_persist(
        &mut self,
        channel_id: ChannelId,
        need_sync: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.hydration_pending.insert(channel_id);
        if !need_sync {
            return self.finish_increment_hydration(channel_id, out);
        }
        let api_base_url = self.config.api_base_url.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            crate::sync_scheduler::enqueue_and_drain(
                state,
                &api_base_url,
                &[channel_id],
                alloc,
                out,
            );
        });
        Ok(())
    }

    /// terminal sync 后从本地表读回完整 dialogList 与该 channel messages。
    pub(crate) fn finish_increment_hydration(
        &mut self,
        channel_id: ChannelId,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !self.state.hydration_pending.remove(&channel_id) {
            return Ok(());
        }
        self.handle_query_command("im_query_dialog_list", b"{}", out)?;
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": channel_id.as_str(),
        }))
        .map_err(|e| ImError::Parse(format!("hydration message query payload: {e}")))?;
        self.handle_query_command("im_query_messages_by_channel", &payload, out)
    }
}
