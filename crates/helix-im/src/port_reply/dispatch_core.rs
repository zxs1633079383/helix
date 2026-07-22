use crate::dialog_patch::PendingDialogPatch;
use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, SendStatus, SyncTrigger, TemporaryId};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    pub(super) fn handle_channel_terminal_persist_reply(
        &mut self,
        channel_id: ChannelId,
        terminal_seq: crate::state::Seq,
        trigger: SyncTrigger,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
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
                self.finish_increment_hydration(channel_id, out)?;
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
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
            PortOutcome::Ok(reply) => {
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.on_persist_ok(corr, out)?;
                }
                let recovery_active = self
                    .state
                    .recovery_session
                    .is_active_for(self.config.auth_user_id.as_str());
                for bytes in pending_domain_events {
                    out.push(Effect::Emit {
                        event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(bytes)),
                    });
                }
                for pending in dialog_patches {
                    if let Some(data) =
                        pending.frame_patch_data_from_channel_reply(reply.0.as_ref())
                    {
                        // 写后 channel reply 已给出累计 unread/mention/urgent 终值。若列表已有
                        // snapshot，只发布同 revision 的属性 patch；客户端不再消费原始 DialogPatch。
                        let frame = crate::dialog_frame::patch_from_dialog_projection(
                            &mut self.state.dialog_list_frame,
                            &data,
                        );
                        // During a V2 authority recovery, mutate the committed
                        // dialog slots but do not leak a parallel V1 frame.  The
                        // composed V2 snapshot below is the sole renderer batch.
                        if !recovery_active {
                            if let Some(frame) = frame {
                                out.push(frame);
                            }
                        }
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
                        "recovery commit acknowledged; composing V2 frame"
                    );
                    self.emit_recovery_v2_frame(Some(corr), out);
                }
                // E3：续拉检查（cursor 已推进后才发，确保 fromSeq 严格递增）
                if wants_continuation {
                    self.maybe_continue_sync(channel_id, trigger, out);
                } else {
                    self.finish_increment_hydration(channel_id, out)?;
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(
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
                    .is_active_for(self.config.auth_user_id.as_str())
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
                crate::send_reconcile::reconcile_send_http(
                    &mut self.state,
                    &temporary_id,
                    reply.0.as_ref(),
                    reconcile_corr,
                    out,
                );
            }
            PortOutcome::Err(e) => {
                tracing::warn!(
                    tmp_id = ?temporary_id, error = ?e,
                    "posts/create http failed, marking pending send failed immediately"
                );
                if let Some(ps) = self.state.pending_sends.get_mut(&temporary_id) {
                    ps.mark_failed_immediately(out);
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
            PortOutcome::Err(e) => tracing::warn!(
                channel_id = channel_id.as_str(), corr = corr.raw(), error = ?e,
                "sync/notify failed"
            ),
        }
        // 窗口已释放 → drain 队首续发下一 channel（B4，HX-C011 在途恒 ≤ K）。
        self.drain_sync_queue(out);
        Ok(())
    }
}
