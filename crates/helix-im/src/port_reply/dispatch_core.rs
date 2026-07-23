use crate::dialog_patch::PendingDialogPatch;
use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, SendStatus, SyncTrigger, TemporaryId};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    pub(super) fn handle_channel_terminal_persist_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        terminal_seq: crate::state::Seq,
        trigger: SyncTrigger,
        pending_domain_events: Vec<Vec<u8>>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                let has_durable_message_changes = !pending_domain_events.is_empty();
                let hydration_already_pending = self.state.hydration_pending.contains(&channel_id);
                let committed = self
                    .state
                    .channels
                    .get_mut(&channel_id)
                    .is_some_and(|channel| channel.commit_terminal_after_atomic(terminal_seq));
                if !committed {
                    tracing::error!(
                        channel_id = channel_id.as_str(),
                        terminal_seq = terminal_seq.0,
                        "terminal atomic receipt no longer matches a live channel cursor; suppressing Frame"
                    );
                    return Ok(());
                }
                tracing::info!(
                    hop = "sync.persist_ok",
                    corr = corr.raw(),
                    channel_id = channel_id.as_str(),
                    committed_seq = terminal_seq.0,
                    terminal = true,
                    trigger = ?trigger,
                    "terminal sync atomic write committed"
                );

                for bytes in pending_domain_events {
                    out.push(Effect::Emit {
                        event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(bytes)),
                    });
                }

                // This is the only renderer-facing type7 projection. `patch_closed` produces an
                // `im:interaction:frame` and only after the local tombstone/cursor transaction
                // has committed; no deleteAt or other absent Go fields are invented.
                if let Some(scope) = self.state.dialog_list_frame.scope().cloned() {
                    if let Some(frame) = self
                        .state
                        .dialog_list_frame
                        .patch_closed(scope, channel_id.as_str())
                    {
                        out.push(frame);
                    }
                }
                out.push(crate::acl::to_effect::emit_sync_state_with_trigger(
                    channel_id,
                    terminal_seq,
                    trigger,
                ));
                if self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str())
                    && self
                        .state
                        .recovery_session
                        .commit_ok(channel_id, terminal_seq)
                {
                    self.try_finalize_recovery_surface(Some(corr), out)?;
                }
                self.finish_increment_hydration(channel_id, out)?;
                if has_durable_message_changes && !hydration_already_pending {
                    self.state.invalidate_recent_message_coverage(channel_id);
                    self.refresh_attached_latest_timeline(channel_id, None, out)?;
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    hop = "sync.persist_failed",
                    corr = corr.raw(),
                    channel_id = channel_id.as_str(),
                    terminal_seq = terminal_seq.0,
                    error = ?error,
                    "terminal atomic persist failed; cursor/tombstone/frame remain unchanged"
                );
            }
        }
        Ok(())
    }

    pub(super) fn handle_channel_persist_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        wants_continuation: bool,
        dialog_patches: Vec<PendingDialogPatch>,
        pending_domain_events: Vec<Vec<u8>>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                let has_durable_message_changes = !pending_domain_events.is_empty();
                let hydration_already_pending = self.state.hydration_pending.contains(&channel_id);
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.on_persist_ok(corr, out)?;
                }
                let committed_seq = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value().0)
                    .unwrap_or(0);
                tracing::info!(
                    hop = "sync.persist_ok",
                    corr = corr.raw(),
                    channel_id = channel_id.as_str(),
                    committed_seq,
                    terminal = false,
                    trigger = ?trigger,
                    wants_continuation,
                    "sync atomic write committed"
                );
                let recovery_active = self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str());
                for bytes in pending_domain_events {
                    out.push(Effect::Emit {
                        event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(bytes)),
                    });
                }
                // PersistAtomic is a write-only barrier. Read back only the affected
                // channel row so the attached dialog preview observes committed data
                // without turning each recovery batch into a full-list scan.
                if !recovery_active && self.state.dialog_list_frame.scope().is_some() {
                    if let Some(pending) = dialog_patches.into_iter().last() {
                        let readback_corr = self.alloc_corr_internal();
                        let pending =
                            pending.with_causation_id(Some(format!("sync-persist-{}", corr.raw())));
                        out.push(Effect::Persist {
                            corr: readback_corr,
                            ops: vec![crate::acl::to_effect::get_channel_row_op(
                                pending.channel_id,
                            )],
                        });
                        self.state.corr_map.insert(
                            readback_corr,
                            crate::state::CorrelationContext::DialogPatchByPost { pending },
                        );
                    }
                }
                let committed_seq = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value())
                    .unwrap_or(crate::state::Seq(0));
                out.push(crate::acl::to_effect::emit_sync_state_with_trigger(
                    channel_id,
                    committed_seq,
                    trigger,
                ));
                if recovery_active
                    && self
                        .state
                        .recovery_session
                        .commit_ok(channel_id, committed_seq)
                {
                    tracing::info!(
                        hop = "recovery.persist_ok",
                        corr = corr.raw(),
                        channel_id = channel_id.as_str(),
                        committed_seq = committed_seq.0,
                        "recovery commit acknowledged; checking bounded completion"
                    );
                    self.try_finalize_recovery_surface(Some(corr), out)?;
                }
                // E3：续拉检查（cursor 已推进后才发，确保 fromSeq 严格递增）
                if wants_continuation {
                    self.maybe_continue_sync(channel_id, trigger, out);
                } else {
                    self.finish_increment_hydration(channel_id, out)?;
                    if has_durable_message_changes && !hydration_already_pending {
                        // The legacy domain event is observability only. A Frame-only
                        // renderer refreshes attached message surfaces from committed
                        // local facts; the dialog preview readback is scheduled above.
                        self.state.invalidate_recent_message_coverage(channel_id);
                        self.refresh_attached_latest_timeline(channel_id, None, out)?;
                    }
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(
                    hop = "sync.persist_failed",
                    channel_id = channel_id.as_str(),
                    corr = corr.raw(),
                    error = ?e,
                    "persist failed, cursor not advanced, will re-sync"
                );
                // persist 失败：清除 last_sync_from_seq 避免遗留脏状态
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.last_sync_from_seq = None;
                }
                if self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str())
                {
                    self.state.recovery_session.commit_failed(channel_id);
                    tracing::warn!(
                        hop = "recovery.persist_failed",
                        corr = corr.raw(),
                        channel_id = channel_id.as_str(),
                        "recovery persistence failed; no V2 frame was emitted"
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_increment_batch_persist_reply(
        &mut self,
        projections: Vec<(ChannelId, Vec<u8>)>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        match outcome {
            PortOutcome::Ok(_) => {
                crate::ws::handlers::increment_channel_end::after_increment_batch_persist(
                    projections,
                    out,
                );
            }
            PortOutcome::Err(error) => {
                tracing::warn!(error = ?error, "increment batch persist failed; final projections suppressed");
            }
        }
    }

    pub(super) fn handle_optimistic_send_reply(
        &mut self,
        temporary_id: TemporaryId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                // P1 可能晚于 HTTP 失败回报；只允许初始 Local 推进，不能把
                // 已终结的 UnSend/Sent 倒退回 Sending。
                if let Some(ps) = self.state.pending_sends.get_mut(&temporary_id) {
                    if ps.status == SendStatus::Local {
                        ps.status = SendStatus::Sending;
                    }
                }
                if let Some(planned) = self
                    .state
                    .pending_media_after_optimistic
                    .remove(&temporary_id)
                {
                    for media in planned {
                        self.emit_media_prepare(
                            media.channel_id,
                            media.temporary_id,
                            media.target,
                            media.input,
                            out,
                        )?;
                    }
                }
                let channel_id = self
                    .state
                    .pending_sends
                    .get(&temporary_id)
                    .and_then(|pending| pending.body.as_ref())
                    .and_then(|body| body.get("channelId"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(ChannelId::from_str);
                let causation_id = self
                    .state
                    .pending_sends
                    .get(&temporary_id)
                    .and_then(|pending| pending.request_id.clone());
                if let Some(channel_id) = channel_id {
                    let defer_http =
                        self.state
                            .pending_sends
                            .get(&temporary_id)
                            .is_some_and(|pending| {
                                pending.remaining_uploads == 0 && !pending.upload_failed
                            });
                    let refresh_scheduled = self
                        .refresh_attached_latest_timeline_with_deferred_send(
                            channel_id,
                            causation_id,
                            defer_http.then_some(temporary_id.clone()),
                            out,
                        )?;
                    if defer_http && !refresh_scheduled {
                        let body = self
                            .state
                            .pending_sends
                            .get(&temporary_id)
                            .and_then(|pending| pending.body.clone())
                            .ok_or_else(|| {
                                ImError::Parse("P1 ordinary send missing cached body".to_string())
                            })?;
                        self.emit_posts_create_http(channel_id, temporary_id, &body, out)?;
                    }
                }
            }
            PortOutcome::Err(e) => {
                self.state
                    .pending_media_after_optimistic
                    .remove(&temporary_id);
                tracing::warn!(
                    tmp_id = ?temporary_id,
                    error = ?e,
                    "P1 optimistic persist failed"
                );
            }
        }
        Ok(())
    }

    pub(super) fn handle_outbound_send_http_reply(
        &mut self,
        channel_id: ChannelId,
        temporary_id: TemporaryId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        match outcome {
            PortOutcome::Ok(reply) => {
                let reconcile_corr = self.alloc_corr_internal();
                let causation_id = self
                    .state
                    .pending_sends
                    .get(&temporary_id)
                    .and_then(|pending| pending.request_id.clone());
                if crate::send_reconcile::reconcile_send_http(
                    &mut self.state,
                    &temporary_id,
                    reply.0.as_ref(),
                    reconcile_corr,
                    out,
                ) {
                    self.state.corr_map.insert(
                        reconcile_corr,
                        crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                            channel_id,
                            causation_id,
                        },
                    );
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(
                    tmp_id = ?temporary_id, error = ?e,
                    "posts/create http failed, marking pending send failed immediately"
                );
                let reconcile_corr = self.alloc_corr_internal();
                let causation_id = self
                    .state
                    .pending_sends
                    .get(&temporary_id)
                    .and_then(|pending| pending.request_id.clone());
                if self
                    .state
                    .pending_sends
                    .get_mut(&temporary_id)
                    .is_some_and(|pending| pending.mark_failed_immediately(reconcile_corr, out))
                {
                    self.state.corr_map.insert(
                        reconcile_corr,
                        crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                            channel_id,
                            causation_id,
                        },
                    );
                }
                out.push(crate::acl::to_effect::emit_post_send_failed(
                    channel_id.as_str(),
                    temporary_id.0.as_str(),
                ));
            }
        }
    }

    pub(super) fn handle_sync_pull_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            ch.inflight_sync = None;
        }
        // B4：释放 1 个全局 sync 窗口（Ok/Err 都释放——否则失败 sync 永久占窗 → 队列饿死）。
        self.state.sync_scheduler.release_window();
        match outcome {
            PortOutcome::Ok(reply) => {
                self.handle_sync_reply(corr, channel_id, trigger, reply, out)?
            }
            PortOutcome::Err(e) => {
                if self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str())
                {
                    self.state.recovery_session.phase = crate::sync_session::RecoveryPhase::Failed;
                }
                tracing::warn!(
                    hop = "sync.request_failed",
                    channel_id = channel_id.as_str(),
                    corr = corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                    trigger = ?trigger,
                    error = ?e,
                    "sync/notify failed"
                );
            }
        }
        // 窗口已释放 → drain 队首续发下一 channel（B4，HX-C011 在途恒 ≤ K）。
        self.drain_sync_queue(out);
        self.try_finalize_recovery_surface(None, out)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_recipient_batch_refreshes_an_attached_timeline_from_storage() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: "latest".to_string(),
        };
        module.state.timeline_projector.register_attachment(&scope);
        let mut out = EffectSink::new();

        module
            .handle_channel_persist_reply(
                helix_core::Correlation::from_raw(7),
                channel_id,
                SyncTrigger::Routine,
                false,
                Vec::new(),
                vec![br#"{"event":"im:post:received","data":{}}"#.to_vec()],
                &PortOutcome::Ok(helix_core::tick::ReplyBytes(bytes::Bytes::new())),
                &mut out,
            )
            .expect("committed recipient batch schedules its render readback");

        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })));
    }
}
