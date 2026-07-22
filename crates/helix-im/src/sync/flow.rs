use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext, Seq, SyncTrigger};
use helix_core::{Effect, EffectSink};

impl ImModule {
    /// E3 续拉：Persist 落库成功后，若上批 sync 标记了 needs_continuation，
    /// 以推进后的 cursor（新 fromSeq）发出下一批 sync Http。
    ///
    /// 终止保证（双重）：
    /// 1. fromSeq（= cursor after commit）必须严格 > last_sync_from_seq（上次发出续拉时的 fromSeq）
    ///    这保证每次续拉都在推进位置；若 cursor 未前进，说明所有事件均是幂等 dup → 停止
    /// 2. B1 gate_inflight 守卫：同 channel 同时只允许一个在途 sync
    pub(crate) fn maybe_continue_sync(
        &mut self,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        out: &mut EffectSink,
    ) {
        // A3b：connectionId 身份头（clone 一次，避免后续 &mut self.state 借用冲突）。
        let conn_id = self.state.connection_id.clone();
        // 读取 cursor 和 last_sync_from_seq（不可变借用）
        let (cursor_seq, prev_from_seq, has_inflight, is_terminal) =
            match self.state.channels.get(&channel_id) {
                Some(ch) => (
                    ch.cursor.value(),
                    ch.last_sync_from_seq,
                    ch.inflight_sync.is_some(),
                    ch.is_terminal(),
                ),
                None => return,
            };

        if is_terminal {
            return;
        }

        // B1：若已有在途 sync（意外情况），跳过
        if has_inflight {
            tracing::warn!(
                channel_id = channel_id.as_str(),
                "maybe_continue_sync: inflight_sync already set, skipping continuation"
            );
            return;
        }

        // 终止保证：新 fromSeq（= cursor after commit）必须严格 > prev_from_seq
        // 若相等，说明服务端返回的事件均低于上次 fromSeq（全部幂等丢弃，cursor 未前进）
        if let Some(prev_from) = prev_from_seq {
            if cursor_seq.0 <= prev_from.0 {
                tracing::warn!(
                    channel_id = channel_id.as_str(),
                    cursor = cursor_seq.0,
                    prev_from_seq = prev_from.0,
                    "continuation aborted: cursor did not advance past prev sync fromSeq (all events were dup-drops)"
                );
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.last_sync_from_seq = None;
                }
                return;
            }
        }

        // 记录本次续拉的 fromSeq（下一批检查时用）
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            ch.last_sync_from_seq = Some(cursor_seq);
        }

        let sync_corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            sync_corr,
            CorrelationContext::SyncPull {
                channel_id,
                trigger,
            },
        );
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            ch.inflight_sync = Some(crate::state::InflightSync(sync_corr));
        }
        // B4：续拉重新占 1 个全局窗口（SyncPull 回报已 release），保持 inflight = 真在途数。
        self.state.sync_scheduler.acquire_window();
        out.push(crate::acl::to_effect::sync_notify(
            &self.config.api_base_url,
            channel_id,
            cursor_seq,
            sync_corr,
            conn_id.as_deref(),
        ));
        tracing::debug!(
            channel_id = channel_id.as_str(),
            from_seq = cursor_seq.0,
            "E3: continuation sync Http emitted"
        );
    }

    /// 处理 sync/notify HTTP 响应
    ///
    /// ## E3 修复
    ///
    /// `needs_continuation=true` 时，Persist 落库后（on_persist_ok 路径），
    /// 此函数还需记录「续拉意图」——通过在 channel 上设置 `last_sync_max_seq`。
    /// 实际续拉在 `on_persist_ok` → `maybe_continue_sync` 中触发，确保 cursor 已推进。
    /// 终止条件：events 为空但 needs_continuation=true → 停止 + log warn（防服务端 bug）。
    pub(crate) fn handle_sync_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use crate::parser::parse_sync_response;
        use crate::sync_session::{
            CommittedRecoveryHead, EventKind, RecoveryComparison, SyncBatchFacts, SyncResponse,
        };

        if self
            .state
            .channels
            .get(&channel_id)
            .is_some_and(|channel| channel.is_terminal())
        {
            tracing::info!(
                channel_id = channel_id.as_str(),
                "ignoring sync reply for terminal channel"
            );
            return Ok(());
        }

        // #6 信封解层（仅 SyncPull 路径）：剥 ADR-007 `{status,headers,body:base64(raw_go_body)}` 信封
        // + base64 decode 出裸 Go body 再解析（直喂信封会找不到 data.entries → 静默 NoChange 丢消息）。
        // 边界零信任（HX-C 不变量 4）：信封 / base64 畸形走 Result::Err，不 unwrap / panic / 静默吞。
        let raw_body = unwrap_sync_envelope(reply.0.as_ref())?;
        let response = parse_sync_response(raw_body.as_ref(), channel_id)
            .map_err(|e| ImError::Parse(format!("sync response parse error: {}", e)))?;

        match response {
            SyncResponse::NoChange => {
                // `no_change` is the authority's explicit answer for the submitted
                // cursor. It creates no storage write, but it can complete an
                // already attached recovery view through the same V2-only path.
                if self
                    .state
                    .recovery_session
                    .is_active_for(self.config.auth_user_id.as_str())
                {
                    let cursor = self
                        .state
                        .channels
                        .get(&channel_id)
                        .map(|channel| channel.cursor.value())
                        .unwrap_or(Seq(0));
                    let comparison = self.state.recovery_session.compare(
                        CommittedRecoveryHead {
                            cursor,
                            ledger_to_seq: cursor,
                            coverage_to_seq: cursor,
                        },
                        crate::sync_session::AuthorityHead { event_seq: cursor },
                    );
                    tracing::info!(
                        hop = "recovery.compare",
                        corr = corr.raw(),
                        channel_id = channel_id.as_str(),
                        local_cursor = cursor.0,
                        authority_head = cursor.0,
                        comparison = ?comparison,
                        "recovery no-change comparison completed without VM content"
                    );
                    if comparison == RecoveryComparison::Equal {
                        self.emit_recovery_v2_frame(None, out);
                    }
                }
                self.finish_increment_hydration(channel_id, out)?;
            }
            SyncResponse::Events {
                mut events,
                messages,
                next_seq,
                needs_continuation,
            } => {
                let from_exclusive = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value())
                    .ok_or_else(|| {
                        ImError::Parse("sync reply has no registered channel".to_string())
                    })?;
                // The Go endpoint may replay a stale prefix while a concurrent
                // recovery advances the local cursor. Stale facts are already
                // committed evidence, not an authority violation.
                events.retain(|event| event.seq > from_exclusive);
                // The Go response is assembled from a viewer-filtered set and
                // does not promise array order. Canonicalize before validating
                // uniqueness/monotonicity so replay is deterministic.
                events.sort_by_key(|event| event.seq);
                if let Some(terminal) = events
                    .iter()
                    .find(|event| matches!(event.kind, EventKind::ChannelTerminalClosed))
                {
                    return self.handle_terminal_sync_event(
                        channel_id,
                        trigger,
                        terminal.seq,
                        events.len(),
                        !messages.is_empty(),
                        next_seq,
                        needs_continuation,
                        out,
                    );
                }
                if events.is_empty() {
                    if needs_continuation {
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            "needs_continuation=true but events is empty, stopping continuation to prevent infinite loop"
                        );
                    }
                    self.finish_increment_hydration(channel_id, out)?;
                    return Ok(());
                }

                let facts =
                    SyncBatchFacts::from_events(channel_id, from_exclusive, next_seq, events)
                        .map_err(|reason| {
                            ImError::Parse(format!("invalid sync batch facts: {reason}"))
                        })?;
                if self
                    .state
                    .recovery_session
                    .is_active_for(self.config.auth_user_id.as_str())
                {
                    let cursor = self
                        .state
                        .channels
                        .get(&channel_id)
                        .map(|channel| channel.cursor.value())
                        .unwrap_or(Seq(0));
                    let comparison = self.state.recovery_session.compare(
                        CommittedRecoveryHead {
                            cursor,
                            ledger_to_seq: cursor,
                            coverage_to_seq: cursor,
                        },
                        facts.authority_head,
                    );
                    if !matches!(comparison, RecoveryComparison::Pull { .. }) {
                        tracing::warn!(
                            hop = "recovery.compare_rejected",
                            corr = corr.raw(),
                            channel_id = channel_id.as_str(),
                            comparison = ?comparison,
                            "recovery batch does not require a contiguous pull; retaining local state"
                        );
                        if comparison == RecoveryComparison::Equal {
                            self.emit_recovery_v2_frame(None, out);
                        } else {
                            self.emit_recovery_v2_frame(None, out);
                            self.emit_proactive_resync_for(&[channel_id], out);
                        }
                        return Ok(());
                    }
                }
                let commit_target = facts.authority_head.event_seq;
                let events = facts.events;
                tracing::info!(
                    hop = "recovery.compare",
                    corr = corr.raw(),
                    channel_id = channel_id.as_str(),
                    from_seq = from_exclusive.0,
                    authority_head = commit_target.0,
                    event_count = events.len(),
                    "recovery batch accepted without rendering VM content"
                );
                let persist_corr = self.alloc_corr_internal();
                let auth_user_id = self.config.auth_user_id.as_str();
                let tenant_id = crate::acl::to_effect::recovery_tenant_actor_scope(
                    self.config.company_id.as_str(),
                    self.config.auth_user_id.as_str(),
                )
                .ok_or_else(|| {
                    ImError::Parse(
                        "recovery persistence requires a tenant and actor identity".to_string(),
                    )
                })?;
                let from_seq = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value().0.saturating_add(1))
                    .unwrap_or(1);
                let coverage_id = format!(
                    "sync:{}:{}:{}:{}",
                    tenant_id,
                    channel_id.as_str(),
                    from_seq,
                    persist_corr.raw()
                );
                let dialog_patches = crate::acl::to_effect::sync_dialog_patch_plans(
                    &events,
                    &messages,
                    auth_user_id,
                );
                let mut persist_ops =
                    crate::acl::to_effect::batch_upsert_events_with_messages_and_auth(
                        &events,
                        &messages,
                        auth_user_id,
                    );
                for pending in &dialog_patches {
                    persist_ops.push(crate::acl::to_effect::bump_channel_unread_op(
                        &pending.update,
                    ));
                }
                // A recovery batch is write-only and atomic: facts, per-event evidence, the
                // range receipt, then cursor. A driver must never report a read result as the
                // durable commit receipt.
                let mut canonical_facts = String::new();
                for event in &events {
                    canonical_facts.push_str(&event.seq.0.to_string());
                    canonical_facts.push(':');
                    canonical_facts.push_str(&event.kind.type_num().to_string());
                    canonical_facts.push(':');
                    canonical_facts.push_str(event.msg_id.as_deref().unwrap_or(""));
                    canonical_facts.push('|');
                    persist_ops.push(crate::acl::to_effect::recovery_ledger_op(
                        crate::acl::to_effect::RecoveryLedgerEntry {
                            tenant_id: tenant_id.clone(),
                            channel_id: channel_id.as_str().to_string(),
                            event_seq: event.seq.0,
                            event_kind: event.kind.type_num().to_string(),
                            message_id: event.msg_id.clone(),
                            event_hash: format!("{}:{}", event.event_id, event.event_payload),
                            coverage_id: coverage_id.clone(),
                            applied_at_ms: event.occurred_at,
                        },
                    ));
                }
                persist_ops.push(crate::acl::to_effect::recovery_coverage_op(
                    crate::acl::to_effect::RecoveryCoverage {
                        coverage_id,
                        tenant_id,
                        channel_id: channel_id.as_str().to_string(),
                        from_seq,
                        to_seq: commit_target.0,
                        event_count: events.len() as u64,
                        facts_hash: canonical_facts,
                        correlation_id: persist_corr.raw().to_string(),
                        committed_at_ms: 0,
                    },
                ));
                persist_ops.push(crate::acl::to_effect::advance_cursor_op(
                    channel_id,
                    commit_target,
                ));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.insert(persist_corr, commit_target);
                }
                if self
                    .state
                    .recovery_session
                    .is_active_for(self.config.auth_user_id.as_str())
                {
                    self.state
                        .recovery_session
                        .await_commit(channel_id, commit_target);
                }
                out.push(Effect::PersistAtomic {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                tracing::info!(
                    hop = "recovery.persist_dispatched",
                    upstream_corr = corr.raw(),
                    corr = persist_corr.raw(),
                    channel_id = channel_id.as_str(),
                    to_seq = commit_target.0,
                    "recovery atomic write dispatched"
                );
                self.state.corr_map.insert(
                    persist_corr,
                    CorrelationContext::ChannelPersist {
                        channel_id,
                        trigger,
                        wants_continuation: needs_continuation,
                        dialog_patches,
                        pending_domain_events:
                            crate::acl::to_effect::sync_mutation_emits_with_auth(
                                &events,
                                &messages,
                                auth_user_id,
                            )
                            .into_iter()
                            .filter_map(|effect| match effect {
                                Effect::Emit { event } => Some(event.0.as_ref().to_vec()),
                                _ => None,
                            })
                            .collect(),
                    },
                );
            }
            SyncResponse::TooLong { reset_to } => {
                // 覆盖式恢复不先删除本地 message：若 getLatestPost 失败，旧历史仍可读；
                // 成功回包沿既有 BatchUpsert 链按主键覆盖消息最终态。
                let reset_cursor = Seq(reset_to.0.saturating_sub(1));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.buffer.clear();
                    ch.gate = None;
                    ch.cursor = crate::state::Cursor::new(reset_cursor);
                    ch.last_sync_from_seq = None;
                }
                out.push(Effect::PersistFire {
                    ops: vec![crate::acl::to_effect::reset_channel_dialog_op(channel_id)],
                });
                out.push(crate::acl::to_effect::emit_sync_too_long(
                    channel_id, reset_to,
                ));
                let reload_corr = self.alloc_corr_internal();
                let payload = serde_json::json!({
                    "channel_id": channel_id.as_str(),
                    "timestamp": 0,
                });
                let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
                if let Ok(effects) = crate::commands::handle_outbound(
                    "im_get_latest_post",
                    payload_bytes.as_ref(),
                    self.config.api_base_url.as_str(),
                    self.config.default_api_base_url.as_str(),
                    self.state.connection_id.as_deref(),
                    reload_corr,
                ) {
                    self.state.corr_map.insert(
                        reload_corr,
                        CorrelationContext::TooLongReload {
                            channel_id,
                            reset_to,
                        },
                    );
                    for effect in effects {
                        out.push(effect);
                    }
                }
            }
            SyncResponse::Snapshot(snap) => {
                // Snapshot 与 too_long 共用“覆盖而非先删”不变量：只产 BatchUpsert，
                // 持久化失败时既有 message 行保持不变。
                let persist_corr = self.alloc_corr_internal();
                let reset_to = snap.reset_to;
                let mut persist_ops = crate::acl::to_effect::batch_upsert_events(&snap.messages);
                persist_ops.push(crate::acl::to_effect::advance_cursor_op(
                    channel_id, reset_to,
                ));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.buffer.clear();
                    ch.gate = None;
                    ch.pending_commits.insert(persist_corr, reset_to);
                    ch.last_sync_from_seq = None;
                }
                out.push(Effect::Persist {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                self.state.corr_map.insert(
                    persist_corr,
                    CorrelationContext::ChannelPersist {
                        channel_id,
                        trigger,
                        wants_continuation: false,
                        dialog_patches: Vec::new(),
                        pending_domain_events: Vec::new(),
                    },
                );
            }
        }

        let _ = corr;
        Ok(())
    }

    /// Consume a strict type7 terminal only through an all-write atomic storage boundary.
    ///
    /// The current Go contract is a one-event closed response. Any mixed/misaligned batch is not
    /// partially applied: no cursor, tombstone or Frame is emitted, and the normal scheduler asks
    /// for a fresh response from the unchanged cursor.
    #[allow(clippy::too_many_arguments)]
    fn handle_terminal_sync_event(
        &mut self,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        terminal_seq: Seq,
        event_count: usize,
        has_messages: bool,
        next_seq: Seq,
        needs_continuation: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if event_count != 1 || has_messages || next_seq != terminal_seq || needs_continuation {
            tracing::warn!(
                channel_id = channel_id.as_str(),
                event_count,
                has_messages,
                next_seq = next_seq.0,
                terminal_seq = terminal_seq.0,
                needs_continuation,
                "rejecting mixed or misaligned terminal sync batch; retaining cursor for recovery"
            );
            self.emit_proactive_resync_for(&[channel_id], out);
            return Ok(());
        }

        let Some(channel) = self.state.channels.get(&channel_id) else {
            tracing::warn!(
                channel_id = channel_id.as_str(),
                "terminal sync for unknown channel ignored"
            );
            return Ok(());
        };
        if channel.is_terminal() || terminal_seq <= channel.cursor.value() {
            // Same seq replay is an idempotent no-op. A restored tombstone is already terminal,
            // and no second Frame/cursor write must be emitted.
            return Ok(());
        }
        if terminal_seq != Seq(channel.cursor.value().0.saturating_add(1)) {
            tracing::warn!(
                channel_id = channel_id.as_str(),
                cursor = channel.cursor.value().0,
                terminal_seq = terminal_seq.0,
                "terminal sync gap rejected; retaining cursor and scheduling recovery"
            );
            self.emit_proactive_resync_for(&[channel_id], out);
            return Ok(());
        }

        let persist_corr = self.alloc_corr_internal();
        out.push(Effect::PersistAtomic {
            corr: persist_corr,
            ops: vec![crate::acl::to_effect::terminal_tombstone_and_cursor_op(
                channel_id,
                terminal_seq,
            )],
        });
        self.state.corr_map.insert(
            persist_corr,
            CorrelationContext::ChannelTerminalPersist {
                channel_id,
                terminal_seq,
                trigger,
            },
        );
        Ok(())
    }

    /// B4：drain SyncScheduler 队列至满窗（拆借 state + next_corr 分配器，避免 &mut self 双借）。
    pub(crate) fn drain_sync_queue(&mut self, out: &mut EffectSink) {
        let api_base_url = self.config.api_base_url.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            crate::sync_scheduler::drain(state, &api_base_url, alloc, out);
        });
    }
}
