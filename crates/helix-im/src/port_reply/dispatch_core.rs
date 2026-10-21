use crate::channel_update::PendingChannelUpdate;
use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, PendingSendReconciliation, SendStatus, SyncTrigger, TemporaryId};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    /// canonical原子回执仍是唯一提交边界，诊断只旁路记录该结果。
    pub(super) fn handle_canonical_stream_persist_reply(
        &mut self,
        event: crate::sync_session::EventEnvelope,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 普通消息只借用 eventId；仅旧接龙缺 eventId 的边界才分配复合去重键。
        let hydration_key = if event.event_id.is_empty() && event.fields.msg_type == "TEXT_CHAIN" {
            std::borrow::Cow::Owned(
                crate::ws::handlers::channel_stream_event::chain_post_event_key(&event),
            )
        } else {
            std::borrow::Cow::Borrowed(event.event_id.as_str())
        };
        let hydrated = self
            .state
            .pending_chain_post_event_ids
            .remove(hydration_key.as_ref());
        self.diagnose(crate::diagnostics::Observation {
            event: "delivery_stage",
            stage: "persist_terminal",
            domain: "stream_seq",
            business_event_id: event.event_id.as_str(),
            path: "live_ws",
            result: if matches!(outcome, PortOutcome::Ok(_)) {
                "success"
            } else {
                "failed"
            },
            channel: event.channel_id.as_str(),
            seq: Some(event.seq.0),
            count: 1,
            ..Default::default()
        });
        if !matches!(outcome, PortOutcome::Ok(_)) {
            if let Some(channel) = self.state.channels.get_mut(&event.channel_id) {
                channel.restore_message_v3_post(event, out);
            }
            return Ok(());
        }

        if hydrated {
            self.state
                .seen_chain_post_event_ids
                .insert(hydration_key.into_owned());
        }
        let terminal = matches!(
            &event.kind,
            crate::sync_session::EventKind::ChannelTerminalClosed
        );
        let committed = if terminal {
            self.state
                .channels
                .get_mut(&event.channel_id)
                .is_some_and(|channel| channel.commit_terminal_after_atomic(event.seq))
        } else {
            let (committed, next) = self
                .state
                .channels
                .get_mut(&event.channel_id)
                .map(|channel| {
                    // Only the canonical stream persist that owned the pending contiguous slot may
                    // emit. If posts_update won the race, its cursor advance must suppress this
                    // duplicate canonical projection.
                    let owns_slot = channel.pending_stream_seq == Some(event.seq)
                        && event.seq
                            == crate::state::Seq(channel.cursor.value().0.saturating_add(1));
                    let next = channel.commit_message_v3_post(event.seq);
                    (owns_slot && channel.cursor.value() == event.seq, next)
                })
                .unwrap_or((false, None));
            if let Some(next) = next {
                let corr = self.alloc_corr_internal();
                crate::ws::handlers::channel_stream_event::queue_or_hydrate_stream_commit(
                    &mut self.state,
                    corr,
                    next,
                    self.config.api_base_url.as_str(),
                    out,
                )?;
            }
            committed
        };
        if !committed && !hydrated {
            return Ok(());
        }
        self.diagnose_checkpoint(event.channel_id, "persist_committed");
        if terminal {
            out.push(crate::acl::to_effect_s1::emit_channel_closed(
                event.channel_id,
                0,
            ));
        } else if !event.redacted
            && !matches!(&event.kind, crate::sync_session::EventKind::Other(_))
        {
            out.push(crate::channel::emit_for_canonical_kind(&event));
        }
        Ok(())
    }

    /// 验证 posts/get 的业务状态、消息身份、频道与可见性，成功后提交同一事件的完整消息。
    pub(super) fn handle_chain_post_hydration_reply(
        &mut self,
        mut event: crate::sync_session::EventEnvelope,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let result = (|| {
            let PortOutcome::Ok(reply) = outcome else {
                return Err(ImError::Parse(
                    "chain post hydration transport failed".to_string(),
                ));
            };
            let body =
                crate::http_envelope::unwrap_success_envelope(reply.0.as_ref(), "posts/get")?;
            let posts = super::dispatch_read::parse_exact_posts_body(&body)?;
            let post_id = event.msg_id.as_deref().unwrap_or_default();
            let fields = posts
                .iter()
                .map(crate::ws::parser::extract_post_fields)
                .find(|fields| fields.id == post_id)
                .ok_or_else(|| {
                    ImError::Parse("chain post hydration missing requested post".to_string())
                })?;
            if self.config.auth_user_id.is_empty()
                || fields.channel_id != event.channel_id.as_str()
                || fields.msg_type != "TEXT_CHAIN"
                || fields.user_id.is_empty()
                || fields.create_at <= 0
                || !crate::channel_write::post_updates_from_fields(
                    event.channel_id,
                    &fields,
                    self.config.auth_user_id.as_str(),
                )
                .visible
            {
                return Err(ImError::Parse(
                    "chain post hydration invalid scope or body".to_string(),
                ));
            }
            Ok(fields)
        })();
        match result {
            Ok(fields) => {
                event.fields = fields;
                let corr = self.alloc_corr_internal();
                crate::ws::handlers::channel_stream_event::queue_stream_commit(
                    &mut self.state,
                    corr,
                    event,
                    out,
                );
            }
            Err(error) => {
                self.state.pending_chain_post_event_ids.remove(
                    &crate::ws::handlers::channel_stream_event::chain_post_event_key(&event),
                );
                tracing::warn!(error = ?error, "chain post hydration rejected; recover through channel sync");
                if let Some(channel) = self.state.channels.get_mut(&event.channel_id) {
                    if event.seq > channel.cursor.value() {
                        channel.restore_message_v3_post(event, out);
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn queue_member_projection_readback(
        &mut self,
        channel_id: ChannelId,
        expected: Box<crate::channel_update::MemberChannelUpdate>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let expected_revision = expected.projection_revision.ok_or_else(|| {
            ImError::Parse("member projection readback missing expected revision".to_string())
        })?;
        let expected_effect_id = expected
            .effect_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ImError::Parse("member projection readback missing expected effectId".to_string())
            })?
            .to_string();
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::MemberProjectionReadback {
                channel_id,
                expected_revision,
                expected_effect_id,
                expected_projection: expected,
            },
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![crate::channel_write::message_v3_member_read_op(
                channel_id,
                self.config.auth_user_id.as_str(),
            )],
        });
        Ok(())
    }

    pub(super) fn handle_channel_terminal_persist_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        terminal_seq: crate::state::Seq,
        trigger: SyncTrigger,
        pending_domain_events: Vec<Vec<u8>>,
        has_category_posts: bool,
        pending_chain_event_ids: Vec<String>,
        member_projection: Option<Box<crate::channel_update::MemberChannelUpdate>>,
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
                        out.push(crate::acl::to_effect_s1::emit_channel_closed(channel_id, 0));
                        if let Some(expected) = member_projection {
                            self.queue_member_projection_readback(channel_id, expected, out)?;
                        }
                        for event_id in pending_chain_event_ids {
                            self.state.pending_chain_event_ids.remove(&event_id);
                            self.state.seen_chain_event_ids.insert(event_id);
                        }
                        self.finish_increment_hydration(channel_id, out)?;
                    } else {
                        for event_id in pending_chain_event_ids {
                            self.state.pending_chain_event_ids.remove(&event_id);
                        }
                        self.fail_hydration_for_channel(
                            channel_id,
                            "hydration terminal cursor mismatch",
                            out,
                        );
                    }
                }
                PortOutcome::Err(_) => {
                    for event_id in pending_chain_event_ids {
                        self.state.pending_chain_event_ids.remove(&event_id);
                    }
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
                if committed {
                    out.push(crate::acl::to_effect_s1::emit_channel_closed(channel_id, 0));
                    for event_id in pending_chain_event_ids {
                        self.state.pending_chain_event_ids.remove(&event_id);
                        self.state.seen_chain_event_ids.insert(event_id);
                    }
                } else {
                    for event_id in pending_chain_event_ids {
                        self.state.pending_chain_event_ids.remove(&event_id);
                    }
                }
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

                if let Some(expected) = member_projection {
                    self.queue_member_projection_readback(channel_id, expected, out)?;
                }

                self.release_post_events(pending_domain_events, has_category_posts, out)?;
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
                self.fail_hydration_for_channel(
                    channel_id,
                    "hydration terminal persist failed",
                    out,
                );
                self.try_finalize_pong_gap(Some(corr), out)?;
            }
        }
        Ok(())
    }

    /// 同步批次只在matching回执后报告提交结果；cursor仍由原状态机推进。
    pub(super) fn handle_channel_persist_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        wants_continuation: bool,
        channel_updates: Vec<PendingChannelUpdate>,
        pending_domain_events: Vec<Vec<u8>>,
        has_category_posts: bool,
        pending_chain_event_ids: Vec<String>,
        pending_send_reconciliations: Vec<PendingSendReconciliation>,
        member_projection: Option<Box<crate::channel_update::MemberChannelUpdate>>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.diagnose(crate::diagnostics::Observation {
            event: "sync_persist_terminal",
            stage: "persist",
            path: "sync_replay",
            result: if matches!(outcome, PortOutcome::Ok(_)) {
                "success"
            } else {
                "failed"
            },
            channel: channel_id.as_str(),
            corr: Some(corr.raw()),
            count: pending_domain_events.len(),
            has_more: wants_continuation,
            ..Default::default()
        });
        match outcome {
            PortOutcome::Ok(_) => {
                let has_durable_message_changes = !pending_domain_events.is_empty();
                let hydration_already_pending = self.state.hydration_pending.contains(&channel_id);
                let (committed, next_buffered) =
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.on_persist_ok(corr)?
                    } else {
                        (false, None)
                    };
                if let Some(event) = next_buffered {
                    self.queue_next_message_v3_event(event, out)?;
                }
                self.state.pong_gap_batch.finish_persist(corr, true);
                let pong_batch_active =
                    trigger == SyncTrigger::PongGap && self.state.pong_gap_batch.is_active();
                if committed && pong_batch_active && !channel_updates.is_empty() {
                    self.state.pong_gap_batch.record_dialog_channel(channel_id);
                }
                if committed {
                    for event_id in pending_chain_event_ids {
                        self.state.pending_chain_event_ids.remove(&event_id);
                        self.state.seen_chain_event_ids.insert(event_id);
                    }
                } else {
                    for event_id in pending_chain_event_ids {
                        self.state.pending_chain_event_ids.remove(&event_id);
                    }
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
                if committed {
                    self.diagnose_checkpoint(channel_id, "sync_committed");
                }
                let recovery_active = self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str());
                if committed {
                    if let Some(expected) = member_projection {
                        self.queue_member_projection_readback(channel_id, expected, out)?;
                    }
                    for reconciliation in pending_send_reconciliations {
                        self.reconcile_pending_send_after_sync(reconciliation, out)?;
                    }
                    self.release_post_events(pending_domain_events, has_category_posts, out)?;
                }
                // PersistAtomic 是写屏障；viewer dialog 读回不依赖时间线 attach，且只按复合键读取一行。
                if committed && !pong_batch_active {
                    if let Some(pending) = channel_updates.into_iter().last() {
                        let readback_corr = self.alloc_corr_internal();
                        out.push(Effect::Persist {
                            corr: readback_corr,
                            ops: vec![crate::channel_write::message_v3_member_read_op(
                                pending.channel_id,
                                self.config.auth_user_id.as_str(),
                            )],
                        });
                        self.state.corr_map.insert(
                            readback_corr,
                            crate::state::CorrelationContext::MessageV3SyncDialogReadback,
                        );
                    }
                }
                let committed_seq = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value())
                    .unwrap_or(crate::state::Seq(0));
                if committed && trigger != SyncTrigger::Hydration {
                    out.push(crate::acl::to_effect::emit_sync_state_with_trigger(
                        channel_id,
                        committed_seq,
                        trigger,
                    ));
                }
                if committed
                    && recovery_active
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
                    if committed && has_durable_message_changes && !hydration_already_pending {
                        // 已提交消息使最近窗口缓存失效，随后从本地权威事实刷新已 attach 时间线。
                        self.state.invalidate_recent_message_coverage(channel_id);
                        self.refresh_attached_latest_timeline(channel_id, None, out)?;
                    }
                }
                // matching 回执解除该写屏障后，兑现期间合并的新意图，使用已提交 cursor。
                self.drain_sync_queue(out);
                self.try_finalize_pong_gap(Some(corr), out)?;
            }
            PortOutcome::Err(e) => {
                self.state.pong_gap_batch.finish_persist(corr, false);
                for event_id in pending_chain_event_ids {
                    self.state.pending_chain_event_ids.remove(&event_id);
                }
                tracing::warn!(
                    hop = "sync.persist_failed",
                    channel_id = channel_id.as_str(),
                    corr = corr.raw(),
                    error = ?e,
                    "persist failed, cursor not advanced, will re-sync"
                );
                // 普通sync也可能被显式hydration合并等待，失败按频道等待者结算。
                self.fail_hydration_for_channel(
                    channel_id,
                    "hydration history persist failed",
                    out,
                );
                // 仅释放 matching 写屏障；失败不推进 cursor，后续同步仍可从旧水位重试。
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.remove(&corr);
                    ch.last_sync_from_seq = None;
                }
                // matching 失败仅解除本 corr；留队重试仍从未推进的旧 cursor 开始。
                self.drain_sync_queue(out);
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

    /// 建立频道分页 session，并只发布轻量 ready 元数据。
    pub(crate) fn open_channel_sync_session(
        &mut self,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let generation = self.state.channel_sync_generation.wrapping_add(1);
        self.state.channel_sync_generation = generation;
        let channel_sync_session_id = format!("channel-sync-{generation}");
        let session = crate::channel_sync::ChannelSyncSession::new(
            channel_sync_session_id,
            generation,
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        );
        tracing::info!(
            session_id = %session.channel_sync_session_id,
            generation,
            "channel-sync-ready 已触发：increment 批次 PersistOk"
        );
        out.push(
            crate::event::channel_sync::ready(
                session.channel_sync_session_id.as_str(),
                session.generation,
            )?
            .into_effect(),
        );
        self.state.channel_sync_session = Some(session);
        Ok(())
    }

    /// 在 global increment 持久成功后建立分页 session；活动 session 会合并刷新。
    pub(super) fn handle_increment_batch_persist_reply(
        &mut self,
        _projections: Vec<(ChannelId, Vec<u8>)>,
        batch_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self.state.channel_sync_persist_inflight == 0 {
            tracing::info!(
                "channel-sync-ready 未触发：当前 increment Persist 回执已处理或无在途批次"
            );
            return Ok(());
        }
        self.state.channel_sync_persist_inflight -= 1;
        self.diagnose(crate::diagnostics::Observation {
            event: "channel_inventory_persisted",
            batch_id: batch_id.as_deref().unwrap_or(""),
            stage: "persist",
            result: if matches!(outcome, PortOutcome::Ok(_)) {
                "success"
            } else {
                "failed"
            },
            count: _projections.len(),
            ..Default::default()
        });
        if matches!(outcome, PortOutcome::Ok(_)) {
            for (channel, _) in &_projections {
                self.diagnose(crate::diagnostics::Observation {
                    event: "channel_inventory_item_committed",
                    stage: "persist",
                    result: "success",
                    channel: channel.as_str(),
                    batch_id: batch_id.as_deref().unwrap_or(""),
                    count: 1,
                    ..Default::default()
                });
                self.diagnose_checkpoint(*channel, "inventory_committed");
            }
        }
        match outcome {
            PortOutcome::Ok(_) => {
                // PersistOk 是唯一 ready 屏障；活动 session 未 complete 前不能被新批次覆盖。
                if self
                    .state
                    .channel_sync_session
                    .as_ref()
                    .is_some_and(|session| !session.completed)
                {
                    self.state.channel_sync_refresh_pending = true;
                    tracing::info!(
                        pending_inflight = self.state.channel_sync_persist_inflight,
                        "channel-sync-ready 延迟：已有频道分页 session 在途"
                    );
                } else {
                    self.open_channel_sync_session(out)?;
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "channel-sync-ready 未触发：increment 批次持久化失败，保留旧会话"
                );
            }
        }
        Ok(())
    }

    /// 处理话题增量原子落库回执；成功只通知该父群继续读取本地话题。
    pub(super) fn handle_subtopic_increment_batch_persist_reply(
        &mut self,
        snapshot_read: bool,
        snapshot_count: Option<usize>,
        request_id: Option<String>,
        parent_channel_id: crate::state::ChannelId,
        batch_key: String,
        batch_id: Option<String>,
        _projections: Vec<(crate::state::ChannelId, Vec<u8>)>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if snapshot_read {
            let Some(mut ops) = self.state.pending_topic_snapshot_ops.remove(&batch_key) else {
                self.state.subtopic_sync_active.remove(&batch_key);
                return Ok(());
            };
            use helix_core::effect::{SqlValue, StorageOp, UpsertGuard, UpsertSpec};
            let current = self
                .state
                .subtopic_requests
                .get(&parent_channel_id)
                .is_none_or(|expected| request_id.as_deref() == Some(expected.as_str()));
            let rows = match outcome {
                PortOutcome::Ok(reply) => {
                    serde_json::from_slice::<Vec<serde_json::Value>>(reply.0.as_ref()).ok()
                }
                _ => None,
            };
            let parent = rows.as_ref().and_then(|rows| {
                rows.iter()
                    .find(|row| row["id"] == parent_channel_id.as_str())
            });
            let expected = parent
                .filter(|row| row["topic_inventory_version"].as_i64().unwrap_or(0) == 0)
                .and_then(|row| row["subtopics_loaded_at"].as_i64());
            let Some(expected) = expected.filter(|_| current) else {
                self.state.subtopic_sync_active.remove(&batch_key);
                return Ok(());
            };
            let Some(StorageOp::BatchUpdate(marker)) = ops.pop() else {
                self.state.subtopic_sync_active.remove(&batch_key);
                return Err(ImError::Parse("topic snapshot marker missing".into()));
            };
            let mut row = vec![(
                "id".to_string(),
                SqlValue::Text(parent_channel_id.as_str().to_string()),
            )];
            for (column, value) in marker.patch {
                let value = if column == "subtopics_loaded_at" {
                    let prior = match value {
                        SqlValue::Integer(value) => value,
                        _ => 0,
                    };
                    SqlValue::Integer(prior.max(expected.saturating_add(1)))
                } else {
                    value
                };
                row.push((column, value));
            }
            // CAS失败使整个清单事务回滚，跨table-set队列也不能迟到覆盖新快照。
            ops.insert(
                0,
                StorageOp::BatchUpsert(UpsertSpec {
                    table: "channel",
                    rows: vec![row],
                    conflict_key: Some("id"),
                    exclude_from_update: vec![],
                    update_guard: Some(UpsertGuard {
                        column: "subtopics_loaded_at",
                        expected: SqlValue::Integer(expected),
                    }),
                    version_column: None,
                }),
            );
            let corr = self.alloc_corr_internal();
            self.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::SubtopicIncrementBatchPersist {
                    snapshot_read: false,
                    snapshot_count,
                    request_id,
                    parent_channel_id,
                    batch_key,
                    batch_id,
                    projections: _projections,
                },
            );
            out.push(Effect::PersistAtomic { corr, ops });
            return Ok(());
        }
        self.diagnose(crate::diagnostics::Observation {
            event: "channel_inventory_persisted",
            stage: "persist",
            channel: parent_channel_id.as_str(),
            batch_id: batch_id.as_deref().unwrap_or(""),
            result: if matches!(outcome, PortOutcome::Ok(_)) {
                "success"
            } else {
                "failed"
            },
            count: _projections.len(),
            ..Default::default()
        });
        if matches!(outcome, PortOutcome::Ok(_)) {
            for (channel, _) in &_projections {
                self.diagnose(crate::diagnostics::Observation {
                    event: "channel_inventory_item_committed",
                    stage: "persist",
                    result: "success",
                    channel: channel.as_str(),
                    batch_id: batch_id.as_deref().unwrap_or(""),
                    count: 1,
                    ..Default::default()
                });
                self.diagnose_checkpoint(*channel, "inventory_committed");
            }
        }
        self.state.subtopic_sync_active.remove(&batch_key);
        // 旧事务回执只释放关联，不能覆盖新请求的UI数量或完成等待。
        if let Some(expected) = self.state.subtopic_requests.get(&parent_channel_id) {
            if request_id.as_deref() != Some(expected.as_str()) {
                return Ok(());
            }
        }

        match outcome {
            PortOutcome::Ok(_) => {
                if !self.state.subtopic_sync_completed.insert(batch_key) {
                    tracing::info!(
                        channel_id = %parent_channel_id.as_str(),
                        "subtopics-sync-ready 未触发：批次已完成"
                    );
                    return Ok(());
                }
                tracing::info!(
                    channel_id = %parent_channel_id.as_str(),
                    "subtopics-sync-ready 已触发：话题 increment 批次 PersistOk"
                );
                if let Some(count) = snapshot_count {
                    out.push(
                        crate::event::channel::update(serde_json::json!({
                            "id": parent_channel_id.as_str(), "threadCount": count
                        }))?
                        .into_effect(),
                    );
                }
                out.push(
                    crate::event::channel_sync::subtopics_ready(
                        parent_channel_id.as_str(),
                        request_id.as_deref(),
                        snapshot_count.is_some(),
                    )?
                    .into_effect(),
                );
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = %parent_channel_id.as_str(),
                    error = ?error,
                    "subtopics-sync-ready 未触发：话题 increment 持久化失败"
                );
            }
        }
        Ok(())
    }

    /// 将本地 channel Scan 回包投影成不超过 20 行的 typed page；失败不发 page 事件。
    pub(super) fn handle_channel_sync_page_reply(
        &mut self,
        channel_sync_session_id: String,
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
        if session.channel_sync_session_id != channel_sync_session_id
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
        tracing::info!(
            channel_rows = serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
                .ok()
                .and_then(|value| value.as_array().map(Vec::len))
                .unwrap_or(0),
            offset,
            has_req_id = req_id.is_some(),
            "channel sync page channel scan accepted; scheduling member snapshot"
        );
        let channel_rows = serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
            .ok()
            .and_then(|value| value.as_array().cloned())
            .ok_or_else(|| ImError::Parse("channel sync page rows are invalid".to_string()))?;
        let member_corr = self.alloc_corr_internal();
        let page_rows = channel_rows
            .iter()
            .skip(offset)
            .take(crate::channel_sync::PAGE_SIZE as usize)
            .cloned()
            .collect::<Vec<_>>();
        let member_effect = crate::channel_sync::member_scan_effect(
            member_corr,
            &page_rows,
            self.config.company_id.as_str(),
            self.config.auth_user_id.as_str(),
        )?;
        self.state.corr_map.insert(
            member_corr,
            crate::state::CorrelationContext::ChannelSyncPageMemberSnapshot {
                channel_sync_session_id,
                generation,
                offset,
                req_id,
                channel_rows: Box::new(channel_rows),
            },
        );
        out.push(member_effect);
        Ok(())
    }

    /// 成员快照持久读回后组装并发布单一绝对频道页；任一 scope/回包失败都不发空投影。
    pub(super) fn handle_channel_sync_page_member_reply(
        &mut self,
        channel_sync_session_id: String,
        generation: u64,
        offset: usize,
        req_id: Option<String>,
        channel_rows: Box<Vec<serde_json::Value>>,
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
        if session.channel_sync_session_id != channel_sync_session_id
            || session.generation != generation
            || session.completed
            || session.account_id != self.config.auth_user_id
            || session.company_id != self.config.company_id
        {
            tracing::warn!("channel sync member snapshot scope mismatch");
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync member snapshot scope mismatch",
                ));
            }
            return Ok(());
        }
        let PortOutcome::Ok(member_reply) = outcome else {
            tracing::warn!("channel sync member snapshot failed; preserving previous page");
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync member snapshot failed",
                ));
            }
            return Ok(());
        };
        let scope = crate::query::DialogListScope::new(
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        );
        let Some((items, has_more)) = crate::channel_sync::project_page_with_members(
            &serde_json::to_vec(channel_rows.as_ref()).unwrap_or_default(),
            member_reply.0.as_ref(),
            &scope,
            offset,
        ) else {
            tracing::warn!("channel sync member snapshot returned invalid rows; preserving page");
            if let Some(req_id) = req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync member snapshot rows are invalid",
                ));
            }
            return Ok(());
        };
        let next_cursor = has_more.then(|| {
            crate::channel_sync::encode_cursor(
                session,
                offset + crate::channel_sync::PAGE_SIZE as usize,
            )
        });
        tracing::info!(
            offset,
            items = items.len(),
            has_more,
            has_req_id = req_id.is_some(),
            "channel sync page member snapshot projected"
        );
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

    /// 成员增删提交后先读回频道行，再扫描 durable roster，避免 sparse WS 回声成为 UI authority。
    pub(super) fn handle_channel_member_update_channel_readback_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let channel = match outcome {
            PortOutcome::Ok(reply) => serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
                .ok()
                .and_then(|value| value.as_array().and_then(|rows| rows.first()).cloned())
                .filter(|value| value.is_object()),
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = %channel_id.as_str(),
                    error = ?error,
                    "channel member update channel readback failed; continuing with roster-only projection"
                );
                None
            }
        };
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelMemberUpdateReadback {
                channel_id,
                channel: channel.map(Box::new),
                causation_id,
            },
        );
        out.push(helix_core::Effect::Persist {
            corr,
            ops: vec![helix_core::effect::StorageOp::Scan(
                helix_core::effect::ScanSpec {
                    table: "channel_member",
                    limit: None,
                    filter: Some((
                        "channel_id",
                        helix_core::effect::SqlValue::Text(channel_id.as_str().to_string()),
                    )),
                    order_by: &[],
                },
            )],
        });
        Ok(())
    }

    /// 成员增删最终读回完整 roster，并只发布由 SQLite 事实组装的绝对成员投影。
    pub(super) fn handle_channel_member_update_readback_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        channel: Option<serde_json::Value>,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member update roster readback failed; suppressing projection");
            return Ok(());
        };
        let Some(rows) = serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
            .ok()
            .and_then(|value| value.as_array().cloned())
        else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member update roster readback invalid; suppressing projection");
            return Ok(());
        };
        let mut members = Vec::new();
        let mut admins = Vec::new();
        let mut bosses = Vec::new();
        let mut owner = serde_json::Value::Null;
        let mut viewer_role = None;
        for row in rows {
            let Some(object) = row.as_object() else {
                continue;
            };
            let Some(user_id) = object
                .get("user_id")
                .or_else(|| object.get("userId"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(team_id) = object
                .get("team_id")
                .or_else(|| object.get("teamId"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let role = normalize_member_projection_role(
                object
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("MEMBER"),
            );
            if !self.config.auth_user_id.is_empty() && user_id == self.config.auth_user_id {
                viewer_role = Some(role);
            }
            let nick_name = object
                .get("nick_name")
                .or_else(|| object.get("nickName"))
                .or_else(|| object.get("nickname"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let member = serde_json::json!({
                "id": user_id,
                "userId": user_id,
                "teamId": team_id,
                "nickName": nick_name,
                "nickname": nick_name,
                "role": role,
            });
            match role {
                "OWNER" if owner.is_null() => owner = member,
                "MANAGER" => admins.push(member),
                "BOSS" => bosses.push(member),
                _ => members.push(member),
            }
        }
        let member_count =
            members.len() + admins.len() + bosses.len() + usize::from(!owner.is_null());
        let mut channel = channel.unwrap_or_else(|| {
            serde_json::json!({
                "id": channel_id.as_str(),
                "channelId": channel_id.as_str(),
            })
        });
        let Some(channel_object) = channel.as_object_mut() else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member update projection is not an object");
            return Ok(());
        };
        channel_object.insert(
            "id".to_string(),
            serde_json::Value::String(channel_id.as_str().to_string()),
        );
        channel_object.insert(
            "channelId".to_string(),
            serde_json::Value::String(channel_id.as_str().to_string()),
        );
        channel_object.insert("members".to_string(), serde_json::Value::Array(members));
        channel_object.insert("adminUsers".to_string(), serde_json::Value::Array(admins));
        channel_object.insert("boss".to_string(), serde_json::Value::Array(bosses));
        channel_object.insert("owner".to_string(), owner);
        channel_object.insert(
            "memberCount".to_string(),
            serde_json::Value::from(member_count),
        );
        channel_object.insert("isMemberChange".to_string(), serde_json::Value::Bool(true));
        // 本轮绝对 roster 必须覆盖扫描到的旧 snake_case 列，否则 render-ready 会优先读回旧名单。
        remove_stale_member_projection_aliases(channel_object);
        if let Some(role) = viewer_role {
            channel_object.insert(
                "role".to_string(),
                serde_json::Value::String(role.to_string()),
            );
        } else if !self.config.auth_user_id.is_empty() {
            channel_object.remove("role");
        }
        let channel = crate::query::render_ready::channel::shape_channel_row(&channel);
        out.push(
            crate::event::channel_member::updated(serde_json::json!({
                "channelId": channel_id.as_str(),
                "channel": channel,
                "isMemberChange": true,
                "projectionSource": "channel_member_update",
                "causationId": causation_id,
            }))?
            .into_effect(),
        );
        Ok(())
    }

    /// 将角色变更首段成员快照编译为只更新既有成员行的原子 upsert。
    pub(super) fn handle_channel_member_role_scan_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        user_ids: Vec<String>,
        role: String,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            self.state.inflight_member_role_updates.remove(&channel_id);
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role snapshot failed; preserving roster");
            return Ok(());
        };
        let rows = serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
            .ok()
            .and_then(|value| value.as_array().cloned());
        let Some(rows) = rows else {
            self.state.inflight_member_role_updates.remove(&channel_id);
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role snapshot invalid; preserving roster");
            return Ok(());
        };
        let mut upsert_rows = Vec::with_capacity(user_ids.len());
        for user_id in &user_ids {
            let Some(row) = rows.iter().find(|row| {
                row.get("channel_id")
                    .or_else(|| row.get("channelId"))
                    .and_then(serde_json::Value::as_str)
                    == Some(channel_id.as_str())
                    && row
                        .get("user_id")
                        .or_else(|| row.get("userId"))
                        .and_then(serde_json::Value::as_str)
                        == Some(user_id.as_str())
            }) else {
                self.state.inflight_member_role_updates.remove(&channel_id);
                tracing::warn!(channel_id = %channel_id.as_str(), user_id, "channel member role update rejected: member row missing");
                return Ok(());
            };
            let Some(team_id) = row
                .get("team_id")
                .or_else(|| row.get("teamId"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                self.state.inflight_member_role_updates.remove(&channel_id);
                tracing::warn!(channel_id = %channel_id.as_str(), user_id, "channel member role update rejected: member tenant missing");
                return Ok(());
            };
            let nick_name = row
                .get("nick_name")
                .or_else(|| row.get("nickName"))
                .or_else(|| row.get("nickname"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            upsert_rows.push(vec![
                (
                    "channel_id".to_string(),
                    helix_core::effect::SqlValue::Text(channel_id.as_str().to_string()),
                ),
                (
                    "user_id".to_string(),
                    helix_core::effect::SqlValue::Text(user_id.clone()),
                ),
                (
                    "team_id".to_string(),
                    helix_core::effect::SqlValue::Text(team_id.to_string()),
                ),
                (
                    "role".to_string(),
                    helix_core::effect::SqlValue::Text(role.clone()),
                ),
                (
                    "nick_name".to_string(),
                    helix_core::effect::SqlValue::Text(nick_name.to_string()),
                ),
            ]);
        }
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelMemberRolePersist { channel_id },
        );
        out.push(helix_core::Effect::PersistAtomic {
            corr,
            ops: vec![helix_core::effect::StorageOp::BatchUpsert(
                helix_core::effect::UpsertSpec {
                    version_column: None,
                    update_guard: None,
                    table: "channel_member",
                    rows: upsert_rows,
                    conflict_key: Some("channel_id,user_id"),
                    exclude_from_update: vec!["team_id", "nick_name"],
                },
            )],
        });
        Ok(())
    }

    /// 角色成员表写成功后发起同频道 roster 读回，确保 UI 只收到持久化终态。
    pub(super) fn handle_channel_member_role_persist_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            self.state.inflight_member_role_updates.remove(&channel_id);
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role persist failed; suppressing roster");
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelMemberRoleChannelReadback { channel_id },
        );
        out.push(helix_core::Effect::Persist {
            corr,
            ops: vec![helix_core::effect::StorageOp::Get(
                helix_core::effect::GetSpec {
                    table: "channel",
                    key_col: "id",
                    key_val: helix_core::effect::SqlValue::Text(channel_id.as_str().to_string()),
                },
            )],
        });
        Ok(())
    }

    /// 角色写后先读回本地频道行，再进入成员绝对 roster 读回，保留权限门槛。
    pub(super) fn handle_channel_member_role_channel_readback_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let channel = match outcome {
            PortOutcome::Ok(reply) => serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
                .ok()
                .and_then(|value| value.as_array().and_then(|rows| rows.first()).cloned())
                .filter(|value| value.is_object()),
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = %channel_id.as_str(),
                    error = ?error,
                    "channel member role channel readback failed; continuing with roster-only projection"
                );
                None
            }
        };
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelMemberRoleReadback {
                channel_id,
                channel: channel.map(Box::new),
            },
        );
        out.push(helix_core::Effect::Persist {
            corr,
            ops: vec![helix_core::effect::StorageOp::Scan(
                helix_core::effect::ScanSpec {
                    table: "channel_member",
                    limit: None,
                    filter: Some((
                        "channel_id",
                        helix_core::effect::SqlValue::Text(channel_id.as_str().to_string()),
                    )),
                    order_by: &[],
                },
            )],
        });
        Ok(())
    }

    /// 角色写后读回成员绝对 roster，并通过既有 channelMemberUpdated consumer 刷新 Angular。
    pub(super) fn handle_channel_member_role_readback_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        channel: Option<serde_json::Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.inflight_member_role_updates.remove(&channel_id);
        let PortOutcome::Ok(reply) = outcome else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role readback failed; preserving roster");
            return Ok(());
        };
        let Some(rows) = serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
            .ok()
            .and_then(|value| value.as_array().cloned())
        else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role readback invalid; preserving roster");
            return Ok(());
        };
        let mut members = Vec::new();
        let mut admins = Vec::new();
        let mut bosses = Vec::new();
        let mut owner = serde_json::Value::Null;
        let mut viewer_role = None;
        for row in rows {
            let Some(object) = row.as_object() else {
                continue;
            };
            let Some(user_id) = object
                .get("user_id")
                .or_else(|| object.get("userId"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(team_id) = object
                .get("team_id")
                .or_else(|| object.get("teamId"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let role = normalize_member_projection_role(
                object
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("MEMBER"),
            );
            if !self.config.auth_user_id.is_empty() && user_id == self.config.auth_user_id {
                viewer_role = Some(role);
            }
            let nick_name = object
                .get("nick_name")
                .or_else(|| object.get("nickName"))
                .or_else(|| object.get("nickname"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let member = serde_json::json!({
                "id": user_id,
                "userId": user_id,
                "teamId": team_id,
                "nickName": nick_name,
                "nickname": nick_name,
                "role": role,
            });
            match role {
                "OWNER" if owner.is_null() => owner = member,
                "MANAGER" => admins.push(member),
                "BOSS" => bosses.push(member),
                _ => members.push(member),
            }
        }
        let member_count =
            members.len() + admins.len() + bosses.len() + usize::from(!owner.is_null());
        let mut channel = channel.unwrap_or_else(|| {
            serde_json::json!({
                "id": channel_id.as_str(),
                "channelId": channel_id.as_str(),
            })
        });
        let Some(channel_object) = channel.as_object_mut() else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role channel projection is not an object");
            return Ok(());
        };
        channel_object.insert(
            "id".to_string(),
            serde_json::Value::String(channel_id.as_str().to_string()),
        );
        channel_object.insert(
            "channelId".to_string(),
            serde_json::Value::String(channel_id.as_str().to_string()),
        );
        channel_object.insert("members".to_string(), serde_json::Value::Array(members));
        channel_object.insert("adminUsers".to_string(), serde_json::Value::Array(admins));
        channel_object.insert("boss".to_string(), serde_json::Value::Array(bosses));
        channel_object.insert("owner".to_string(), owner);
        channel_object.insert(
            "memberCount".to_string(),
            serde_json::Value::from(member_count),
        );
        channel_object.insert("isMemberChange".to_string(), serde_json::Value::Bool(true));
        // 角色 readback 是当前事实；整形前移除旧持久化别名，避免 admin_users 把 adminUsers 覆盖为空。
        remove_stale_member_projection_aliases(channel_object);
        if let Some(role) = viewer_role {
            channel_object.insert(
                "role".to_string(),
                serde_json::Value::String(role.to_string()),
            );
        } else if !self.config.auth_user_id.is_empty() {
            // Never reuse a stale channel role when the authenticated viewer is absent from the
            // durable roster; the next full channel projection must re-establish authority.
            channel_object.remove("role");
        }
        let channel = crate::query::render_ready::channel::shape_channel_row(&channel);
        out.push(
            crate::event::channel_member::updated(serde_json::json!({
                "channelId": channel_id.as_str(),
                "channel": channel,
                "isMemberChange": true,
                "projectionSource": "channel_member_role_updated",
            }))?
            .into_effect(),
        );
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
                    let failed = if media.input.uses_multipart() {
                        crate::send::upload_props::FailedMediaOp::Multipart(media.clone())
                    } else {
                        crate::send::upload_props::FailedMediaOp::Prepare(media.clone())
                    };
                    self.state
                        .failed_media_ops
                        .insert((temporary_id.clone(), media.target.clone()), failed);
                }
                let media_compensation = !planned.is_empty();
                if media_compensation {
                    // 复用 retry inflight gate：补偿事务成功前（以及失败后）均不允许用户
                    // retry 启动 Java/OSS I/O。成功回执由专用 correlation context 释放。
                    self.state.media_retry_inflight.insert(temporary_id.clone());
                }

                if let Some((channel_id, body, timeline_readback, timeout_timer)) = terminal {
                    let crate::pending_send::TimelineReadbackContext {
                        window_token,
                        causation_id,
                    } = timeline_readback;
                    let failed_event =
                        crate::event::post::send_failed_for_identity_with_request_id(
                            channel_id.as_str(),
                            temporary_id.0.as_str(),
                            causation_id.as_deref(),
                        )?;
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
                            window_token,
                            causation_id,
                        }
                    } else {
                        crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                            channel_id,
                            window_token,
                            causation_id,
                        }
                    };
                    self.state.corr_map.insert(terminal_corr, context);
                    out.push(Effect::CancelTimer { id: timeout_timer });
                    out.push(failed_event.into_effect());
                }
            }
        }
        Ok(())
    }

    /// 消费 posts/create transport admission：成功不产生发送终态，失败立即发布失败事实。
    pub(super) fn handle_outbound_send_http_reply(
        &mut self,
        channel_id: ChannelId,
        temporary_id: TemporaryId,
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
            // success/post.id，发送终态只由权威 WS post echo 推进；echo 缺失时保留 15s
            // 终态，并由下一个 seq gap 触发的 sync notify 补偿。
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
                    let failed_event =
                        crate::event::post::send_failed_for_identity_with_request_id(
                            channel_id.as_str(),
                            temporary_id.0.as_str(),
                            causation_id.as_deref(),
                        )?;
                    self.state.corr_map.insert(
                        reconcile_corr,
                        crate::state::CorrelationContext::TimelineRefreshAfterSendPersist {
                            channel_id,
                            window_token,
                            causation_id,
                        },
                    );
                    out.push(failed_event.into_effect());
                }
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
        // buffer 溢出可能已把旧 sync 升级为覆盖恢复；旧响应只释放自己的全局窗口，不能拆掉新守卫。
        let superseded = self.state.channels.get(&channel_id).is_some_and(|channel| {
            channel
                .inflight_sync
                .is_some_and(|inflight| inflight.0 != corr)
        });
        if superseded {
            self.state.sync_scheduler.release_window();
            self.drain_sync_queue(out);
            return Ok(());
        }
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            ch.inflight_sync = None;
        }
        // B4：释放 1 个全局 sync 窗口（Ok/Err 都释放——否则失败 sync 永久占窗 → 队列饿死）。
        self.state.sync_scheduler.release_window();
        let mut response_error = None;
        match outcome {
            PortOutcome::Ok(reply) => {
                if let Err(error) = self.handle_sync_reply(corr, channel_id, trigger, reply, out) {
                    self.diagnose(crate::diagnostics::Observation {
                        event: "sync_terminal",
                        result: "failed",
                        reason: "response_invalid",
                        path: "sync_replay",
                        channel: channel_id.as_str(),
                        corr: Some(corr.raw()),
                        ..Default::default()
                    });
                    if self.state.hydration_req_ids.contains_key(&channel_id) {
                        self.fail_hydration_for_channel(
                            channel_id,
                            "hydration sync response invalid",
                            out,
                        );
                    } else {
                        response_error = Some(error);
                    }
                }
            }
            PortOutcome::Err(e) => {
                self.diagnose(crate::diagnostics::Observation {
                    event: "sync_terminal",
                    result: "failed",
                    reason: "request_failed",
                    path: "sync_replay",
                    channel: channel_id.as_str(),
                    corr: Some(corr.raw()),
                    ..Default::default()
                });
                self.fail_hydration_for_channel(channel_id, "hydration sync failed", out);
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
        // 无hydration等待者仍保留原Err；engine会丢弃失败step的Effect，不能在这里占新窗口。
        if let Some(error) = response_error {
            return Err(error);
        }
        // 已处理的hydration失败也兑现释放窗口，防止其它等待频道饿死。
        self.drain_sync_queue(out);
        self.try_finalize_recovery(None, out)?;
        self.try_finalize_pong_gap(None, out)?;
        Ok(())
    }
}

/// 将持久层 ADMIN/wire 别名收敛为 Angular roster 使用的四种业务角色。
fn normalize_member_projection_role(role: &str) -> &'static str {
    match role.trim().to_ascii_uppercase().as_str() {
        "OWNER" | "CREATOR" => "OWNER",
        "BOSS" => "BOSS",
        "ADMIN" | "MANAGER" | "MANGER" => "MANAGER",
        _ => "MEMBER",
    }
}

/// 移除已由本轮成员 readback 重建的持久层别名，保证 render-ready 选择最新 camelCase 字段。
fn remove_stale_member_projection_aliases(
    channel: &mut serde_json::Map<String, serde_json::Value>,
) {
    for key in ["admin_users", "member_count"] {
        channel.remove(key);
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

    /// 管理员 wire/storage 别名不得越过 render-ready 边界暴露为 ADMIN。
    #[test]
    fn member_projection_role_uses_manager_business_vocabulary() {
        assert_eq!(normalize_member_projection_role("ADMIN"), "MANAGER");
        assert_eq!(normalize_member_projection_role("MANGER"), "MANAGER");
        assert_eq!(normalize_member_projection_role("MANAGER"), "MANAGER");
        assert_eq!(normalize_member_projection_role("CREATOR"), "OWNER");
        assert_eq!(normalize_member_projection_role("MEMBER"), "MEMBER");
    }

    /// 新完整 roster 写入后不得被同一频道快照中的旧 snake_case 别名反向覆盖。
    #[test]
    fn member_projection_removes_stale_storage_aliases_before_rendering() {
        let mut channel = serde_json::json!({
            "admin_users": [],
            "member_count": 2,
            "members": [{ "userId": "member-a" }, { "userId": "member-b" }],
            "adminUsers": [{ "userId": "manager-a", "role": "MANAGER" }],
            "boss": [],
            "owner": null,
            "memberCount": 3,
        });
        remove_stale_member_projection_aliases(channel.as_object_mut().expect("channel object"));
        let projected = crate::query::render_ready::channel::shape_channel_row(&channel);

        assert_eq!(projected["adminUsers"][0]["userId"], "manager-a");
        assert_eq!(projected["memberCount"], 3);
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
        module
            .state
            .channel_sync_session
            .as_mut()
            .expect("old session")
            .completed = true;
        module.state.reset_increment_batch();
        module.state.channel_sync_persist_inflight = 1;
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                None,
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
        module.state.channel_sync_persist_inflight = 1;
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                None,
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

        module
            .state
            .channel_sync_session
            .as_mut()
            .expect("ready session")
            .completed = true;
        module.state.reset_increment_batch();
        module.state.channel_sync_persist_inflight = 1;
        out.clear();
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                None,
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("next batch success emits ready");
        assert_eq!(out.as_slice().len(), 1);
        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                None,
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("duplicate success is idempotent");
        assert_eq!(out.as_slice().len(), 1);
    }

    /// 后续批次持久化完成仍等待当前分页会话关闭，不替换活动会话。
    #[test]
    fn active_channel_sync_session_is_not_replaced_until_complete() {
        let mut module = module();
        let mut out = EffectSink::new();
        module.state.channel_sync_persist_inflight = 2;

        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                None,
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("first persist opens a session");
        let first_session = module
            .state
            .channel_sync_session
            .as_ref()
            .expect("first session")
            .channel_sync_session_id
            .clone();
        assert_eq!(out.as_slice().len(), 1);

        module
            .handle_increment_batch_persist_reply(
                Vec::new(),
                None,
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .expect("second persist is coalesced");
        assert_eq!(out.as_slice().len(), 1);
        assert_eq!(
            module
                .state
                .channel_sync_session
                .as_ref()
                .expect("active session")
                .channel_sync_session_id,
            first_session
        );
        assert!(module.state.channel_sync_refresh_pending);

        out.clear();
        module
            .handle_query_command(
                "im_complete_channel_sync",
                serde_json::json!({
                    "channel_sync_session_id": first_session,
                    "req_id": "complete-refresh"
                })
                .to_string()
                .as_bytes(),
                &mut out,
            )
            .expect("complete opens the coalesced refresh");
        assert_eq!(out.as_slice().len(), 3);
        assert!(!module.state.channel_sync_refresh_pending);
        assert_ne!(
            module
                .state
                .channel_sync_session
                .as_ref()
                .expect("refreshed session")
                .channel_sync_session_id,
            first_session
        );
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
        let member_rows = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "channel_id": row["id"],
                    "user_id": "user-a",
                    "team_id": "company-a",
                    "role": "MEMBER",
                    "nick_name": "viewer"
                })
            })
            .collect::<Vec<_>>();
        let member_outcome = PortOutcome::Ok(ReplyBytes(Bytes::from(
            serde_json::to_vec(&member_rows).unwrap(),
        )));
        let finish_page = |module: &mut ImModule,
                           channel_rows: Vec<serde_json::Value>,
                           offset: usize,
                           req_id: Option<String>| {
            let mut first_out = EffectSink::new();
            module
                .handle_channel_sync_page_reply(
                    session.channel_sync_session_id.clone(),
                    session.generation,
                    offset,
                    req_id.clone(),
                    &outcome,
                    &mut first_out,
                )
                .expect("channel scan schedules member snapshot");
            let member_corr = first_out.as_slice().iter().find_map(|effect| match effect {
                helix_core::Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            });
            assert!(member_corr.is_some());
            let mut page_out = EffectSink::new();
            module
                .handle_channel_sync_page_member_reply(
                    session.channel_sync_session_id.clone(),
                    session.generation,
                    offset,
                    req_id,
                    Box::new(channel_rows),
                    &member_outcome,
                    &mut page_out,
                )
                .expect("member snapshot emits page");
            page_out
        };
        let out = finish_page(&mut module, rows.clone(), 0, None);
        let first = event(&out.as_slice()[0]);
        assert_eq!(first["data"]["items"].as_array().unwrap().len(), 20);
        assert_eq!(first["data"]["hasMore"], true);

        let query_out = finish_page(&mut module, rows.clone(), 0, Some("page-1".to_string()));
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
        let second_out = finish_page(&mut module, rows.clone(), 20, None);
        let second = event(&second_out.as_slice()[0]);
        assert_eq!(second["data"]["items"].as_array().unwrap().len(), 20);
        assert_eq!(second["data"]["hasMore"], true);

        let third_out = finish_page(&mut module, rows.clone(), 40, None);
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

    /// Channel sync pages retain absolute viewer capabilities and reject another company.
    #[test]
    fn channel_sync_page_projects_capabilities_with_tenant_scope() {
        let mut module = module();
        let session = crate::channel_sync::ChannelSyncSession::new(
            "session-capabilities".to_string(),
            7,
            "user-a",
            "company-a",
        );
        module.state.channel_sync_session = Some(session.clone());
        let rows = vec![
            serde_json::json!({
                "id": "creator-channel",
                "team_id": "company-a",
                "user_id": "user-a",
                "role": "CREATOR",
                "create_by": "user-a",
                "owner": "{\"id\":\"user-a\"}",
                "notice_permission": "MANAGER",
            }),
            serde_json::json!({
                "id": "member-channel",
                "team_id": "company-a",
                "user_id": "user-a",
                "role": "MEMBER",
                "create_by": "owner-b",
                "notice_permission": "MANAGER",
            }),
            serde_json::json!({
                "id": "other-company",
                "team_id": "company-b",
                "user_id": "user-a",
                "role": "CREATOR",
            }),
        ];
        let outcome = PortOutcome::Ok(ReplyBytes(Bytes::from(
            serde_json::to_vec(&rows).expect("rows JSON"),
        )));
        let member_rows = serde_json::json!([
            {
                "channel_id": "creator-channel",
                "user_id": "user-a",
                "team_id": "company-a",
                "role": "CREATOR"
            },
            {
                "channel_id": "member-channel",
                "user_id": "user-a",
                "team_id": "company-a",
                "role": "MEMBER"
            }
        ]);
        let member_outcome = PortOutcome::Ok(ReplyBytes(Bytes::from(
            serde_json::to_vec(&member_rows).expect("member rows JSON"),
        )));
        let mut scan_out = EffectSink::new();

        module
            .handle_channel_sync_page_reply(
                session.channel_sync_session_id.clone(),
                session.generation,
                0,
                None,
                &outcome,
                &mut scan_out,
            )
            .expect("channel page schedules member snapshot");
        assert!(scan_out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, helix_core::Effect::Persist { .. })));
        let member_scan = scan_out.as_slice().iter().find_map(|effect| match effect {
            helix_core::Effect::Persist { ops, .. } => ops.first(),
            _ => None,
        });
        let Some(helix_core::effect::StorageOp::ScopedScan(spec)) = member_scan else {
            panic!("channel page must schedule one scoped member scan");
        };
        assert_eq!(spec.limit, 15_001usize);
        let scoped_ids = spec
            .scope_values
            .iter()
            .map(|value| match value {
                helix_core::effect::SqlValue::Text(value) => value.as_str(),
                _ => panic!("channel scope must use text IDs"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            scoped_ids,
            vec!["creator-channel", "member-channel", "other-company"]
        );
        assert!(scoped_ids.contains(&"other-company"));

        let mut out = EffectSink::new();
        module
            .handle_channel_sync_page_member_reply(
                session.channel_sync_session_id,
                session.generation,
                0,
                None,
                Box::new(rows),
                &member_outcome,
                &mut out,
            )
            .expect("member snapshot emits channel page");
        let page = event(&out.as_slice()[0]);
        let items = page["data"]["items"].as_array().expect("items array");
        assert_eq!(items.len(), 2, "cross-company rows must be fail-closed");
        let creator = items
            .iter()
            .find(|item| item["id"] == "creator-channel")
            .expect("creator item");
        assert_eq!(creator["role"], "CREATOR");
        assert_eq!(creator["createBy"], "user-a");
        assert_eq!(creator["owner"]["id"], "user-a");
        assert_eq!(creator["noticePermission"], "MANAGER");
        assert_eq!(creator["canManageSettings"], true);
        assert_eq!(creator["canManageMembers"], true);

        let member = items
            .iter()
            .find(|item| item["id"] == "member-channel")
            .expect("member item");
        assert_eq!(member["canManageSettings"], false);
        assert_eq!(member["canManageMembers"], false);
    }
}
