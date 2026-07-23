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
                    .commit_ok(channel_id, terminal_seq)
                {
                    self.emit_recovery_v2_frame(None, out);
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
            PortOutcome::Ok(reply) => {
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
                    if has_durable_message_changes && !hydration_already_pending {
                        // The legacy domain event is observability only. A Frame-only
                        // renderer must be refreshed from the committed local fact;
                        // otherwise a recipient can persist the WS post while its
                        // attached timeline remains stale indefinitely. This is one
                        // indexed attachment lookup plus one bounded local Scan.
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
                    .and_then(|pending| pending.timeline_readback.causation_id.clone());
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
                let planned = self
                    .state
                    .pending_media_after_optimistic
                    .remove(&temporary_id)
                    .unwrap_or_default();
                tracing::warn!(
                    tmp_id = ?temporary_id,
                    error = ?e,
                    "P1 optimistic persist failed"
                );

                // `im:post:sending` 已在 P1 同 step 发布；若首笔 message+journal 写失败后
                // 只停止外部 I/O，renderer 会永久停在 sending。这里立即形成失败终态。
                // 媒体补偿仍要把失败消息与原始 prepare journal 原子写回：进程若随后重启，
                // retry 必须先恢复 journal 并重新走 Java prepare，绝不能直发未校验 props。
                let terminal = self.state.pending_sends.get_mut(&temporary_id).and_then(|pending| {
                    pending.status = SendStatus::UnSend;
                    pending.persist_corr = None;
                    pending.upload_failed = !planned.is_empty();
                    let body = pending.body.as_mut()?;
                    if let Some(props) = body.get_mut("props") {
                        for media in &planned {
                            if let Err(error) = crate::send::upload_props::mark_media_stage(
                                props,
                                &media.target,
                                "failed",
                            ) {
                                tracing::warn!(
                                    tmp_id = temporary_id.0.as_str(),
                                    error = ?error,
                                    "failed to project media failure after optimistic persist error"
                                );
                            }
                        }
                    }
                    let channel_id = body
                        .get("channelId")
                        .and_then(serde_json::Value::as_str)
                        .and_then(ChannelId::from_str)?;
                    Some((
                        channel_id,
                        body.clone(),
                        pending.timeline_readback.clone(),
                        pending.timeout_timer,
                    ))
                });

                for media in &planned {
                    self.state.failed_media_ops.insert(
                        (temporary_id.clone(), media.target.clone()),
                        crate::send::upload_props::FailedMediaOp::Prepare(media.clone()),
                    );
                }
                let media_compensation = !planned.is_empty();
                if media_compensation {
                    // 复用 retry inflight gate：补偿事务成功前（以及失败后）均不允许用户
                    // retry 启动 Java/OSS I/O。成功回执由专用 correlation context 释放。
                    self.state.media_retry_inflight.insert(temporary_id.clone());
                }

                if let Some((channel_id, body, timeline_readback, timeout_timer)) = terminal {
                    let terminal_corr = self.alloc_corr_internal();
                    let mut terminal_ops = vec![
                        crate::pending_send::optimistic_message_persist_op(
                            temporary_id.0.as_str(),
                            channel_id.as_str(),
                            &body,
                        ),
                        crate::pending_send::send_status_persist_op(&temporary_id, "unsend"),
                    ];
                    if planned.is_empty() {
                        out.push(Effect::Persist {
                            corr: terminal_corr,
                            ops: terminal_ops,
                        });
                    } else {
                        let operations = planned
                            .into_iter()
                            .map(crate::send::upload_props::PendingMediaOp::Prepare)
                            .collect::<Vec<_>>();
                        terminal_ops
                            .push(crate::send::upload_props::durable_upsert_many(&operations)?);
                        out.push(Effect::PersistAtomic {
                            corr: terminal_corr,
                            ops: terminal_ops,
                        });
                    }
                    let context = if media_compensation {
                        crate::state::CorrelationContext::MediaFailureCompensation {
                            temporary_id: temporary_id.clone(),
                            channel_id,
                            window_token: timeline_readback.window_token,
                            causation_id: timeline_readback.causation_id,
                        }
                    } else {
                        crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                            channel_id,
                            window_token: timeline_readback.window_token,
                            causation_id: timeline_readback.causation_id,
                        }
                    };
                    self.state.corr_map.insert(terminal_corr, context);
                    out.push(Effect::CancelTimer { id: timeout_timer });
                    out.push(crate::acl::to_effect::emit_post_send_failed(
                        channel_id.as_str(),
                        temporary_id.0.as_str(),
                    ));
                }
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
        let timeline_readback = self
            .state
            .pending_sends
            .get(&temporary_id)
            .map(|pending| pending.timeline_readback.clone())
            .unwrap_or_default();
        match outcome {
            PortOutcome::Ok(reply) => {
                let reconcile_corr = self.alloc_corr_internal();
                let crate::pending_send::TimelineReadbackContext {
                    window_token,
                    causation_id,
                } = timeline_readback;
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
                            window_token,
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
                let crate::pending_send::TimelineReadbackContext {
                    window_token,
                    causation_id,
                } = timeline_readback;
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
                            window_token,
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
            PortOutcome::Err(e) => tracing::warn!(
                hop = "sync.request_failed",
                channel_id = channel_id.as_str(),
                corr = corr.raw(),
                track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                trigger = ?trigger,
                error = ?e,
                "sync/notify failed"
            ),
        }
        // 窗口已释放 → drain 队首续发下一 channel（B4，HX-C011 在途恒 ≤ K）。
        self.drain_sync_queue(out);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_retry_pending(
        module: &mut ImModule,
        temporary_id: &TemporaryId,
        window_token: Option<&str>,
        causation_id: Option<&str>,
    ) {
        let mut pending = crate::pending_send::PendingSend::new(
            temporary_id.clone(),
            helix_core::TimerId::from_raw(991),
            None,
        );
        pending.status = SendStatus::Sending;
        pending.timeline_readback = crate::pending_send::TimelineReadbackContext {
            window_token: window_token.map(str::to_string),
            causation_id: causation_id.map(str::to_string),
        };
        module
            .state
            .pending_sends
            .insert(temporary_id.clone(), pending);
    }

    fn successful_posts_create_reply() -> helix_core::tick::ReplyBytes {
        let raw = serde_json::to_vec(&serde_json::json!({
            "status": "SUCCESS",
            "data": {"id": crate::state::test_server_id(42).as_str()}
        }))
        .unwrap();
        let envelope = serde_json::json!({
            "status": 200,
            "headers": [],
            "body": helix_core::port_codec::base64_encode(&raw)
        });
        helix_core::tick::ReplyBytes(bytes::Bytes::from(serde_json::to_vec(&envelope).unwrap()))
    }

    fn terminal_refresh<'a>(
        module: &'a ImModule,
        out: &EffectSink,
    ) -> (
        helix_core::Correlation,
        &'a crate::state::CorrelationContext,
    ) {
        let corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("terminal send outcome persists a durable fact");
        (
            corr,
            module
                .state
                .corr_map
                .get(&corr)
                .expect("terminal persist retains its timeline refresh context"),
        )
    }

    #[test]
    // 回归锚: G03a（posts/create 成功终态沿用 retry window + causation）
    fn posts_create_success_preserves_retry_window_and_causation() {
        use helix_core::{Module, Tick};

        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "viewer".to_string();
        let mut module = ImModule::new(config);
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let scope = crate::interaction_frame::SurfaceScope::Timeline {
            channel_id: channel_id.as_str().to_string(),
            window_token: "retry-http-window".to_string(),
        };
        module.state.timeline_projector.register_attachment(&scope);
        let temporary_id = TemporaryId("tmp-retry-http-success".to_string());
        install_retry_pending(
            &mut module,
            &temporary_id,
            Some("retry-http-window"),
            Some("retry-http-request"),
        );

        let mut out = EffectSink::new();
        module.handle_outbound_send_http_reply(
            channel_id,
            temporary_id,
            &PortOutcome::Ok(successful_posts_create_reply()),
            &mut out,
        );

        let (terminal_corr, terminal_context) = terminal_refresh(&module, &out);
        assert_eq!(
            terminal_context,
            &crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token: Some("retry-http-window".to_string()),
                causation_id: Some("retry-http-request".to_string()),
            }
        );

        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: terminal_corr,
                    outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes::default()),
                },
                2,
                &mut out,
            )
            .expect("successful posts/create persist schedules the exact retry timeline");
        let scan_corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, ops }
                    if ops.iter().any(|op| {
                        matches!(
                            op,
                            helix_core::effect::StorageOp::Scan(spec)
                                if spec.table == "message"
                        )
                    }) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .expect("successful retry terminal re-enters local-first projection");

        let rows = serde_json::json!([{
            "id": crate::state::test_server_id(42).as_str(),
            "temporary_id": "tmp-retry-http-success",
            "channel_id": channel_id.as_str(),
            "user_id": "viewer",
            "type": "FILE",
            "message": "",
            "props": "{\"file\":{\"name\":\"report.pdf\",\"upload\":{\"status\":\"uploaded\"}}}",
            "send_status": "sent",
            "upload_progress_percent": 100,
            "create_at": 1
        }]);
        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: scan_corr,
                    outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(bytes::Bytes::from(
                        serde_json::to_vec(&rows).unwrap(),
                    ))),
                },
                3,
                &mut out,
            )
            .expect("successful retry readback emits the terminal frame");
        let frame = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => {
                    serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                        .ok()
                        .filter(|value| value["event"] == "im:interaction:frame")
                }
                _ => None,
            })
            .expect("successful retry readback emits a typed Frame");
        let decoded = crate::interaction_frame::InteractionFrame::decode(
            &serde_json::to_vec(&frame["data"]).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.mutations[0].scope, scope);
        assert_eq!(
            decoded.mutations[0].causation_id.as_deref(),
            Some("retry-http-request")
        );
        assert_eq!(
            frame["data"]["mutations"][0]["mutation"]["view"]["items"][0]["delivery"]["kind"],
            "sent"
        );
    }

    #[test]
    // 回归锚: G03a（posts/create 失败终态沿用 retry window + causation）
    fn posts_create_failure_preserves_retry_window_and_causation() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let temporary_id = TemporaryId("tmp-retry-http-failure".to_string());
        install_retry_pending(
            &mut module,
            &temporary_id,
            Some("retry-http-window"),
            Some("retry-http-request"),
        );

        let mut out = EffectSink::new();
        module.handle_outbound_send_http_reply(
            channel_id,
            temporary_id,
            &PortOutcome::Err(helix_core::tick::PortError::Network),
            &mut out,
        );

        assert_eq!(
            terminal_refresh(&module, &out).1,
            &crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token: Some("retry-http-window".to_string()),
                causation_id: Some("retry-http-request".to_string()),
            }
        );
    }

    #[test]
    fn ordinary_send_terminal_keeps_latest_fallback() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let temporary_id = TemporaryId("tmp-ordinary-http-failure".to_string());
        install_retry_pending(&mut module, &temporary_id, None, Some("ordinary-request"));

        let mut out = EffectSink::new();
        module.handle_outbound_send_http_reply(
            channel_id,
            temporary_id,
            &PortOutcome::Err(helix_core::tick::PortError::Network),
            &mut out,
        );

        assert_eq!(
            terminal_refresh(&module, &out).1,
            &crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token: None,
                causation_id: Some("ordinary-request".to_string()),
            }
        );
    }

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
