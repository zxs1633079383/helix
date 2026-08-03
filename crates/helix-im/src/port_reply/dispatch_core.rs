use crate::channel_update::PendingChannelUpdate;
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
        if trigger == SyncTrigger::Hydration {
            match outcome {
                PortOutcome::Ok(_) => {
                    let committed = self
                        .state
                        .channels
                        .get_mut(&channel_id)
                        .is_some_and(|channel| channel.commit_terminal_after_atomic(terminal_seq));
                    if committed {
                        self.finish_increment_hydration(channel_id, out)?;
                    } else {
                        self.fail_hydration_for_channel(
                            channel_id,
                            "hydration terminal cursor mismatch",
                            out,
                        );
                    }
                }
                PortOutcome::Err(_) => {
                    self.fail_hydration_for_channel(
                        channel_id,
                        "hydration terminal persist failed",
                        out,
                    );
                }
            }
            return Ok(());
        }
        match outcome {
            PortOutcome::Ok(_) => {
                let has_durable_message_changes = !pending_domain_events.is_empty();
                let hydration_already_pending = self.state.hydration_pending.contains(&channel_id);
                let committed = self
                    .state
                    .channels
                    .get_mut(&channel_id)
                    .is_some_and(|channel| channel.commit_terminal_after_atomic(terminal_seq));
                self.state.pong_gap_batch.finish_persist(corr, committed);
                if !committed {
                    tracing::error!(
                        channel_id = channel_id.as_str(),
                        terminal_seq = terminal_seq.0,
                        "terminal atomic receipt no longer matches a live channel cursor; suppressing event"
                    );
                    self.try_finalize_pong_gap(Some(corr), out)?;
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
                if trigger == SyncTrigger::Routine {
                    out.push(
                        crate::event::sync::recovered(serde_json::json!({
                            "channelId": channel_id.as_str(),
                            "committedSeq": terminal_seq.0,
                            "state": "recovered",
                        }))?
                        .into_effect(),
                    );
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
                    self.try_finalize_recovery(Some(corr), out)?;
                }
                self.finish_increment_hydration(channel_id, out)?;
                if has_durable_message_changes && !hydration_already_pending {
                    self.state.invalidate_recent_message_coverage(channel_id);
                    self.refresh_attached_latest_timeline(channel_id, None, out)?;
                }
                self.try_finalize_pong_gap(Some(corr), out)?;
            }
            PortOutcome::Err(error) => {
                self.state.pong_gap_batch.finish_persist(corr, false);
                tracing::warn!(
                    hop = "sync.persist_failed",
                    corr = corr.raw(),
                    channel_id = channel_id.as_str(),
                    terminal_seq = terminal_seq.0,
                    error = ?error,
                    "terminal atomic persist failed; cursor/tombstone/frame remain unchanged"
                );
                self.try_finalize_pong_gap(Some(corr), out)?;
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
        channel_updates: Vec<PendingChannelUpdate>,
        pending_domain_events: Vec<Vec<u8>>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let pong_batch_active =
            trigger == SyncTrigger::PongGap && self.state.pong_gap_batch.is_active();
        match outcome {
            PortOutcome::Ok(_) => {
                let has_durable_message_changes = !pending_domain_events.is_empty();
                let hydration_already_pending = self.state.hydration_pending.contains(&channel_id);
                let next_buffered = if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.on_persist_ok(corr)?
                } else {
                    None
                };
                if let Some(event) = next_buffered {
                    self.queue_next_message_v3_event(event, out)?;
                }
                self.state.pong_gap_batch.finish_persist(corr, true);
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
                let recovery_channel_attached = recovery_active
                    && self.state.timeline_state.is_attached(
                        &crate::timeline_state::TimelineScope {
                            channel_id: channel_id.as_str().to_string(),
                            window_token: "latest".to_string(),
                        },
                    );
                for bytes in pending_domain_events {
                    out.push(Effect::Emit {
                        event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(bytes)),
                    });
                }
                // PersistAtomic 是写屏障；仅读回受影响频道行，避免恢复批次退化为全表扫描。
                if (!recovery_active || recovery_channel_attached) && !pong_batch_active {
                    if let Some(pending) = channel_updates.into_iter().last() {
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
                            crate::state::CorrelationContext::ChannelUpdateByPost { pending },
                        );
                    }
                }
                let committed_seq = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value())
                    .unwrap_or(crate::state::Seq(0));
                if trigger != SyncTrigger::Hydration {
                    out.push(crate::acl::to_effect::emit_sync_state_with_trigger(
                        channel_id,
                        committed_seq,
                        trigger,
                    ));
                }
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
                    self.try_finalize_recovery(Some(corr), out)?;
                }
                // E3：续拉检查（cursor 已推进后才发，确保 fromSeq 严格递增）
                if wants_continuation {
                    self.maybe_continue_sync(channel_id, trigger, out);
                } else {
                    self.finish_increment_hydration(channel_id, out)?;
                    if has_durable_message_changes && !hydration_already_pending {
                        // 已提交消息使最近窗口缓存失效，随后从本地权威事实刷新已 attach 时间线。
                        self.state.invalidate_recent_message_coverage(channel_id);
                        self.refresh_attached_latest_timeline(channel_id, None, out)?;
                    }
                }
                self.try_finalize_pong_gap(Some(corr), out)?;
            }
            PortOutcome::Err(e) => {
                self.state.pong_gap_batch.finish_persist(corr, false);
                tracing::warn!(
                    hop = "sync.persist_failed",
                    channel_id = channel_id.as_str(),
                    corr = corr.raw(),
                    error = ?e,
                    "persist failed, cursor not advanced, will re-sync"
                );
                if trigger == SyncTrigger::Hydration {
                    self.fail_hydration_for_channel(
                        channel_id,
                        "hydration history persist failed",
                        out,
                    );
                }
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
                self.try_finalize_pong_gap(Some(corr), out)?;
            }
        }
        Ok(())
    }

    /// 在 hello increment 全批持久成功后建立分页 session，仅发布轻量 ready 元数据。
    pub(super) fn handle_increment_batch_persist_reply(
        &mut self,
        _projections: Vec<(ChannelId, Vec<u8>)>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                if !self.state.channel_sync_batch_pending {
                    tracing::debug!("channel sync ready already emitted for this increment batch");
                    return Ok(());
                }
                self.state.channel_sync_batch_pending = false;
                // PersistOk 是唯一 ready 屏障；事件只携带 session 元数据，不重放频道数组。
                let generation = self.state.channel_sync_generation.wrapping_add(1);
                self.state.channel_sync_generation = generation;
                let session_id = format!("channel-sync-{generation}");
                let session = crate::channel_sync::ChannelSyncSession::new(
                    session_id,
                    generation,
                    self.config.auth_user_id.as_str(),
                    self.config.company_id.as_str(),
                );
                out.push(
                    crate::event::channel_sync::ready(
                        session.channel_sync_session_id.as_str(),
                        session.generation,
                    )?
                    .into_effect(),
                );
                self.state.channel_sync_session = Some(session);
            }
            PortOutcome::Err(error) => {
                self.state.channel_sync_batch_pending = false;
                tracing::warn!(error = ?error, "increment batch persist failed; final projections suppressed");
            }
        }
        Ok(())
    }

    /// 将本地 channel Scan 回包投影成不超过 20 行的 typed page；失败不发 page 事件。
    pub(super) fn handle_channel_sync_page_reply(
        &mut self,
        session_id: String,
        generation: u64,
        offset: usize,
        req_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(session) = self.state.channel_sync_session.as_ref() else {
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync session is not ready",
                ));
            }
            return Ok(());
        };
        if session.channel_sync_session_id != session_id
            || session.generation != generation
            || session.completed
            || session.account_id != self.config.auth_user_id
            || session.company_id != self.config.company_id
        {
            tracing::warn!("channel sync page reply scope mismatch");
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync page reply scope mismatch",
                ));
            }
            return Ok(());
        }
        let PortOutcome::Ok(reply) = outcome else {
            tracing::warn!("channel sync page scan failed; preserving previous page");
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync page scan failed",
                ));
            }
            return Ok(());
        };
        if !matches!(
            serde_json::from_slice::<serde_json::Value>(reply.0.as_ref()),
            Ok(serde_json::Value::Array(_))
        ) {
            tracing::warn!(
                "channel sync page scan returned invalid rows; preserving previous page"
            );
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync page rows are invalid",
                ));
            }
            return Ok(());
        }
        let scope = crate::query::DialogListScope::new(
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        );
        let (items, has_more) = crate::channel_sync::project_page(reply.0.as_ref(), &scope, offset);
        let next_cursor = has_more.then(|| {
            crate::channel_sync::encode_cursor(
                session,
                offset + crate::channel_sync::PAGE_SIZE as usize,
            )
        });
        if let Some(req_id) = req_id.as_deref() {
            // query bridge 统一等待 `im:read:result`; body 仍是 Helix 生成的 typed page，
            // Host 不需要恢复旧 raw channel/increment 事件。
            out.push(crate::read_relay::emit_read_body(
                req_id,
                serde_json::json!({
                    "channelSyncSessionId": session.channel_sync_session_id,
                    "generation": session.generation,
                    "items": items,
                    "hasMore": has_more,
                    "nextCursor": next_cursor,
                }),
            ));
        } else {
            // 无 transport waiter 的内部调用仍保留 typed page 事件。
            out.push(
                crate::event::channel_sync::page(
                    session.channel_sync_session_id.as_str(),
                    session.generation,
                    items,
                    has_more,
                    next_cursor.as_deref(),
                )?
                .into_effect(),
            );
        }
        Ok(())
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
                let optimistic_body =
                    if let Some(ps) = self.state.pending_sends.get_mut(&temporary_id) {
                        if ps.status == SendStatus::Local {
                            ps.status = SendStatus::Sending;
                            ps.body.clone()
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                if let Some(body) = optimistic_body {
                    let channel_id = body
                        .get("channelId")
                        .and_then(serde_json::Value::as_str)
                        .and_then(ChannelId::from_str)
                        .ok_or_else(|| {
                            ImError::Parse(
                                "P1 optimistic send missing cached channelId".to_string(),
                            )
                        })?;
                    out.push(
                        crate::event::post::sending_from_local_body(
                            channel_id.as_str(),
                            temporary_id.0.as_str(),
                            self.config.auth_user_id.as_str(),
                            &body,
                        )?
                        .into_effect(),
                    );
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

                // P1 未成功前不会发布 `im:post:sending`；失败终态只保留 durable retry
                // 恢复所需事实，不允许外部 HTTP/timer 抢跑。
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
                    out.push(
                        crate::event::post::send_failed_for_identity(
                            channel_id.as_str(),
                            temporary_id.0.as_str(),
                        )?
                        .into_effect(),
                    );
                }
            }
        }
        Ok(())
    }

    /// 将 posts/create 终态收敛到本地发送状态，并只发布 MessageV3 失败事实。
    pub(super) fn handle_outbound_send_http_reply(
        &mut self,
        channel_id: ChannelId,
        temporary_id: TemporaryId,
        authoritative_readback: bool,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let timeline_readback = self
            .state
            .pending_sends
            .get(&temporary_id)
            .map(|pending| pending.timeline_readback.clone())
            .unwrap_or_default();
        match outcome {
            // posts/create HTTP 只代表 transport admission。服务端不会在该回包提供
            // success/post.id，终态必须继续等待 WS post 或权威 read-back。
            PortOutcome::Ok(_) if authoritative_readback => {
                self.start_authoritative_send_readback(
                    channel_id,
                    timeline_readback.window_token.as_deref(),
                    timeline_readback.causation_id,
                    temporary_id,
                    out,
                )?;
            }
            PortOutcome::Ok(_) => {}
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
                out.push(
                    crate::event::post::send_failed_for_identity(
                        channel_id.as_str(),
                        temporary_id.0.as_str(),
                    )?
                    .into_effect(),
                );
            }
        }
        Ok(())
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
                if let Err(error) = self.handle_sync_reply(corr, channel_id, trigger, reply, out) {
                    if trigger == SyncTrigger::Hydration {
                        self.fail_hydration_for_channel(
                            channel_id,
                            "hydration sync response invalid",
                            out,
                        );
                        return Ok(());
                    }
                    return Err(error);
                }
            }
            PortOutcome::Err(e) => {
                if trigger == SyncTrigger::Hydration {
                    self.fail_hydration_for_channel(channel_id, "hydration sync failed", out);
                }
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
        self.try_finalize_recovery(None, out)?;
        self.try_finalize_pong_gap(None, out)?;
        Ok(())
    }
}

#[cfg(test)]
mod channel_sync_contract_tests {
    use super::*;
    use bytes::Bytes;
    use helix_core::tick::{PortOutcome, ReplyBytes};
    use helix_core::EffectSink;

    fn module() -> ImModule {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "user-a".to_string();
        config.company_id = "company-a".to_string();
        ImModule::new(config)
    }

    fn event(effect: &helix_core::Effect) -> serde_json::Value {
        let helix_core::Effect::Emit { event } = effect else {
            panic!("expected typed event");
        };
        serde_json::from_slice(event.0.as_ref()).expect("event JSON")
    }

    #[test]
    fn ready_requires_persist_ok_and_is_emitted_once_without_channel_array() {
        let mut module = module();
        let mut out = EffectSink::new();
        module.state.channel_sync_session = Some(crate::channel_sync::ChannelSyncSession::new(
            "old-session".to_string(),
            1,
            "user-a",
            "company-a",
        ));
        module.state.reset_increment_batch();
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                &PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
                &mut out,
            )
            .expect("failed persist is handled");
        assert!(out.as_slice().is_empty());
        assert_eq!(
            module
                .state
                .channel_sync_session
                .as_ref()
                .map(|session| session.channel_sync_session_id.as_str()),
            Some("old-session")
        );

        module.state.reset_increment_batch();
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("successful persist emits ready");
        assert_eq!(out.as_slice().len(), 1);
        let ready = event(&out.as_slice()[0]);
        assert_eq!(ready["event"], "im:channel-sync-ready");
        assert!(ready["data"].get("items").is_none());
        assert!(ready["data"].get("channels").is_none());
        assert!(ready["data"].get("accountId").is_none());
        assert!(ready["data"].get("companyId").is_none());
        assert_eq!(ready["data"]["defaultPageSize"], 20);

        module.state.reset_increment_batch();
        out.clear();
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("next batch success emits ready");
        assert_eq!(out.as_slice().len(), 1);
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("duplicate success is idempotent");
        assert_eq!(out.as_slice().len(), 1);
    }

    #[test]
    fn page_results_are_continuous_and_bounded() {
        let mut module = module();
        let session = crate::channel_sync::ChannelSyncSession::new(
            "session-a".to_string(),
            3,
            "user-a",
            "company-a",
        );
        module.state.channel_sync_session = Some(session.clone());
        let rows = (0..41)
            .map(|index| {
                serde_json::json!({
                    "id": format!("channel-{index}"),
                    "team_id": "company-a",
                    "user_id": "user-a",
                    "type": "D",
                })
            })
            .collect::<Vec<_>>();
        let outcome = PortOutcome::Ok(ReplyBytes(Bytes::from(serde_json::to_vec(&rows).unwrap())));
        let mut out = EffectSink::new();
        module
            .handle_channel_sync_page_reply(
                session.channel_sync_session_id.clone(),
                session.generation,
                0,
                None,
                &outcome,
                &mut out,
            )
            .expect("first page emits");
        let first = event(&out.as_slice()[0]);
        assert_eq!(first["data"]["items"].as_array().unwrap().len(), 20);
        assert_eq!(first["data"]["hasMore"], true);

        let mut query_out = EffectSink::new();
        module
            .handle_channel_sync_page_reply(
                "session-a".to_string(),
                3,
                0,
                Some("page-1".to_string()),
                &outcome,
                &mut query_out,
            )
            .expect("query waiter page emits");
        let query_result = event(&query_out.as_slice()[0]);
        assert_eq!(query_result["event"], "im:read:result");
        assert_eq!(query_result["data"]["req_id"], "page-1");
        assert_eq!(
            query_result["data"]["body"]["items"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        let mut second_out = EffectSink::new();
        module
            .handle_channel_sync_page_reply(
                session.channel_sync_session_id,
                session.generation,
                20,
                None,
                &outcome,
                &mut second_out,
            )
            .expect("second page emits");
        let second = event(&second_out.as_slice()[0]);
        assert_eq!(second["data"]["items"].as_array().unwrap().len(), 20);
        assert_eq!(second["data"]["hasMore"], true);

        let mut third_out = EffectSink::new();
        module
            .handle_channel_sync_page_reply(
                "session-a".to_string(),
                3,
                40,
                None,
                &outcome,
                &mut third_out,
            )
            .expect("third page emits");
        let third = event(&third_out.as_slice()[0]);
        assert_eq!(third["data"]["items"].as_array().unwrap().len(), 1);
        assert_eq!(third["data"]["hasMore"], false);

        let mut failed_out = EffectSink::new();
        module
            .handle_channel_sync_page_reply(
                "session-a".to_string(),
                3,
                0,
                None,
                &PortOutcome::Ok(ReplyBytes(Bytes::from_static(b"not-json"))),
                &mut failed_out,
            )
            .expect("invalid page rows are handled");
        assert!(failed_out.as_slice().is_empty());
    }
}
