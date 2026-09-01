use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext, Seq, SyncTrigger};
use helix_core::{Effect, EffectSink};

fn append_persona_columns(
    row: &mut helix_core::effect::Row,
    persona: &crate::sync_session::SyncPersona,
) {
    use helix_core::effect::SqlValue;
    let seq = |value: Option<Seq>| {
        SqlValue::Integer(
            value
                .map(|seq| seq.0.min(i64::MAX as u64) as i64)
                .unwrap_or(0),
        )
    };
    row.push((
        "membership_state".to_string(),
        SqlValue::Text(persona.membership_state.clone()),
    ));
    row.push(("epoch_start_seq".to_string(), seq(persona.epoch_start_seq)));
    row.push(("epoch_end_seq".to_string(), seq(persona.epoch_end_seq)));
}

type PreparedSyncProjection = (
    helix_core::effect::Row,
    crate::channel_update::MemberChannelUpdate,
    u64,
    String,
);

fn prepare_sync_projection(
    persona: &crate::sync_session::SyncPersona,
    channel_id: ChannelId,
    auth_user_id: &str,
) -> Result<Option<PreparedSyncProjection>, ImError> {
    let Some(data) = persona.member_projection.as_ref() else {
        return Ok(None);
    };
    let Some((mut row, projection)) = crate::channel_update::member_channel_from_update_channel(
        data,
        channel_id,
        auth_user_id,
        0,
    ) else {
        return Err(ImError::Parse("invalid sync memberProjection".to_string()));
    };
    let revision = projection
        .projection_revision
        .filter(|value| *value > 0)
        .ok_or_else(|| ImError::Parse("sync memberProjection missing revision".to_string()))?;
    let effect_id = projection
        .effect_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("sync memberProjection missing effectId".to_string()))?
        .to_string();
    append_persona_columns(&mut row, persona);
    Ok(Some((row, projection, revision, effect_id)))
}

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
        let trace_enabled = self.config.offline_sync_diagnostics.trace;
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
            crate::offline_sync_warn!(
                trace_enabled,
                channel_id = channel_id.as_str(),
                "maybe_continue_sync: inflight_sync already set, skipping continuation"
            );
            return;
        }

        // 终止保证：新 fromSeq（= cursor after commit）必须严格 > prev_from_seq
        // 若相等，说明服务端返回的事件均低于上次 fromSeq（全部幂等丢弃，cursor 未前进）
        if let Some(prev_from) = prev_from_seq {
            if cursor_seq.0 <= prev_from.0 {
                crate::offline_sync_warn!(trace_enabled,
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
        crate::offline_sync_info!(trace_enabled,
            hop = "sync.dispatch",
            corr = sync_corr.raw(),
            track_id = crate::acl::sync_http_effects::sync_track_id(sync_corr),
            channel_id = channel_id.as_str(),
            from_seq = cursor_seq.0,
            trigger = ?trigger,
            continuation = true,
            scheduler_inflight = self.state.sync_scheduler.inflight(),
            scheduler_pending = self.state.sync_scheduler.pending_len(),
            "sync/notify continuation dispatched"
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
        let trace_enabled = self.config.offline_sync_diagnostics.trace;
        use crate::parser::parse_sync_response;
        use crate::sync_session::{
            CommittedRecoveryHead, EventKind, RecoveryComparison, SyncBatchFacts, SyncResponse,
        };

        crate::offline_sync_info!(trace_enabled,
            hop = "sync.reply_received",
            corr = corr.raw(),
            track_id = crate::acl::sync_http_effects::sync_track_id(corr),
            channel_id = channel_id.as_str(),
            trigger = ?trigger,
            reply_bytes = reply.0.len(),
            "sync/notify port reply received"
        );

        if self
            .state
            .channels
            .get(&channel_id)
            .is_some_and(|channel| channel.is_terminal())
        {
            crate::offline_sync_info!(
                trace_enabled,
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
            SyncResponse::NoChange { next_seq, persona } => {
                let local_cursor = self
                    .state
                    .channels
                    .get(&channel_id)
                    .map(|channel| channel.cursor.value().0)
                    .unwrap_or(0);
                crate::offline_sync_info!(trace_enabled,
                    hop = "sync.response",
                    corr = corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                    channel_id = channel_id.as_str(),
                    trigger = ?trigger,
                    kind = "no_change",
                    local_cursor,
                    "sync/notify response decoded"
                );
                // `no_change` is the authority's explicit answer for the submitted
                // cursor. It creates no storage write, but it can complete an
                // already attached recovery view through the same V2-only path.
                if self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str())
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
                        crate::sync_session::AuthorityHead {
                            event_seq: next_seq,
                        },
                    );
                    crate::offline_sync_info!(trace_enabled,
                        hop = "recovery.compare",
                        corr = corr.raw(),
                        channel_id = channel_id.as_str(),
                        local_cursor = cursor.0,
                        authority_head = cursor.0,
                        comparison = ?comparison,
                        "recovery no-change comparison completed without VM content"
                    );
                }
                if let Some((row, expected, revision, effect_id)) = prepare_sync_projection(
                    &persona,
                    channel_id,
                    self.config.auth_user_id.as_str(),
                )? {
                    let projection_corr = self.alloc_corr_internal();
                    let auth_user_id = self.config.auth_user_id.as_str();
                    self.state.corr_map.insert(
                        projection_corr,
                        crate::state::CorrelationContext::MemberProjectionPersist {
                            channel_id,
                            expected_revision: revision,
                            expected_effect_id: effect_id,
                            expected_projection: Box::new(expected),
                        },
                    );
                    out.push(Effect::PersistAtomic {
                        corr: projection_corr,
                        ops: crate::acl::to_effect::canonical_member_projection_ops(
                            channel_id,
                            auth_user_id,
                            revision,
                            row,
                        ),
                    });
                }
                self.finish_increment_hydration(channel_id, out)?;
            }
            SyncResponse::Events {
                mut events,
                messages,
                next_seq,
                needs_continuation,
                persona,
            } => {
                let track_id = crate::acl::sync_http_effects::sync_track_id(corr);
                let type_counts = crate::sync::observability::event_type_counts(&events);
                crate::offline_sync_info!(trace_enabled,
                    hop = "sync.batch.input",
                    corr = corr.raw(),
                    track_id = track_id.as_str(),
                    channel_id = channel_id.as_str(),
                    event_seq = next_seq.0,
                    event_type = "batch",
                    msg_id = "",
                    source = "sync_notify",
                    operation_id = format!("sync-batch:{}", corr.raw()),
                    from_seq = self
                        .state
                        .channels
                        .get(&channel_id)
                        .map(|channel| channel.cursor.value().0)
                        .unwrap_or(0),
                    next_seq = next_seq.0,
                    event_count = events.len(),
                    message_count = messages.len(),
                    type_counts = %type_counts,
                    "同步批次已解析"
                );
                for event in &events {
                    let fields = event.msg_id.as_deref().and_then(|id| messages.get(id));
                    if let Some(fields) = fields {
                        let metadata = crate::sync::observability::post_fields_metadata(fields);
                        crate::offline_sync_info!(
                            trace_enabled
                                && self.config.offline_sync_diagnostics.target_matches(
                                    event.msg_id.as_deref(),
                                    Some(fields),
                                ),
                            hop = "sync.event.parsed",
                            corr = corr.raw(),
                            track_id = track_id.as_str(),
                            channel_id = event.channel_id.as_str(),
                            event_seq = event.seq.0,
                            event_type = event.kind.type_num(),
                            msg_id = event.msg_id.as_deref().unwrap_or_default(),
                            source = "sync_notify",
                            operation_id = format!("sync:{}:{}:{}", corr.raw(), event.seq.0, event.kind.type_num()),
                            message_map_hit = true,
                            message_map_key = event.msg_id.as_deref().unwrap_or_default(),
                            field_presence = %metadata["field_presence"],
                            field_lengths = %metadata["field_lengths"],
                            field_hashes = %metadata["field_hashes"],
                            "同步事件已解析"
                        );
                    } else {
                        crate::offline_sync_info!(
                            trace_enabled
                                && self
                                    .config
                                    .offline_sync_diagnostics
                                    .target_matches(event.msg_id.as_deref(), None,),
                            hop = "sync.event.parsed",
                            corr = corr.raw(),
                            track_id = track_id.as_str(),
                            channel_id = event.channel_id.as_str(),
                            event_seq = event.seq.0,
                            event_type = event.kind.type_num(),
                            msg_id = event.msg_id.as_deref().unwrap_or_default(),
                            source = "sync_notify",
                            operation_id = format!(
                                "sync:{}:{}:{}",
                                corr.raw(),
                                event.seq.0,
                                event.kind.type_num()
                            ),
                            message_map_hit = false,
                            message_map_key = event.msg_id.as_deref().unwrap_or_default(),
                            field_presence = "{}",
                            field_lengths = "{}",
                            field_hashes = "{}",
                            "同步事件已解析"
                        );
                    }
                }
                crate::offline_sync_info!(trace_enabled,
                    hop = "sync.response",
                    corr = corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                    channel_id = channel_id.as_str(),
                    trigger = ?trigger,
                    kind = "events",
                    event_count = events.len(),
                    message_count = messages.len(),
                    next_seq = next_seq.0,
                    needs_continuation,
                    "sync/notify response decoded"
                );
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
                    let terminal_seq = terminal.seq;
                    return self.handle_terminal_sync_event(
                        corr,
                        channel_id,
                        trigger,
                        from_exclusive,
                        terminal_seq,
                        events,
                        messages,
                        next_seq,
                        needs_continuation,
                        persona,
                        out,
                    );
                }
                if events.is_empty() {
                    if needs_continuation {
                        crate::offline_sync_warn!(trace_enabled,
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
                    .is_collecting_for(self.config.auth_user_id.as_str())
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
                        crate::offline_sync_warn!(trace_enabled,
                            hop = "recovery.compare_rejected",
                            corr = corr.raw(),
                            channel_id = channel_id.as_str(),
                            comparison = ?comparison,
                            "recovery batch does not require a contiguous pull; retaining local state"
                        );
                        if comparison != RecoveryComparison::Equal {
                            self.emit_proactive_resync_for(&[channel_id], out);
                        }
                        return Ok(());
                    }
                }
                let commit_target = facts.authority_head.event_seq;
                let events = facts.events;
                crate::offline_sync_info!(
                    trace_enabled,
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
                let member_projection =
                    prepare_sync_projection(&persona, channel_id, auth_user_id)?;
                let pending_send_reconciliations =
                    crate::acl::to_effect::pending_send_reconciliations(
                        &events,
                        &messages,
                        auth_user_id,
                    );
                let observation = crate::sync::observability::SyncObservation::new(
                    corr.raw(),
                    track_id.clone(),
                    "sync_notify",
                    self.config.offline_sync_diagnostics.clone(),
                );
                let mut persist_ops =
                    crate::acl::to_effect::batch_upsert_events_with_messages_and_auth_observed(
                        &events,
                        &messages,
                        auth_user_id,
                        if trigger == SyncTrigger::Hydration {
                            crate::acl::sync_effects::SyncApplyMode::HydrationHistory
                        } else {
                            crate::acl::sync_effects::SyncApplyMode::LiveRecovery
                        },
                        Some(&observation),
                    );
                if let Some((row, _, revision, _)) = member_projection.as_ref() {
                    persist_ops.extend(crate::acl::to_effect::canonical_member_projection_ops(
                        channel_id,
                        auth_user_id,
                        *revision,
                        row.clone(),
                    ));
                }
                // Chain facts arrive inside the same channel-event payload as ordinary message
                // facts. Extend this write set before the cursor op so a replay can never commit
                // the channel sequence without its canonical chain projection.
                let mut chain_domain_events = Vec::new();
                let mut pending_chain_event_ids = Vec::new();
                for event in &events {
                    if let Some(projection) = crate::chain::synced_projection(event)? {
                        let duplicate = projection.event_id.as_ref().is_some_and(|event_id| {
                            self.state.seen_chain_event_ids.contains(event_id)
                                || self.state.pending_chain_event_ids.contains(event_id)
                        });
                        if duplicate {
                            continue;
                        }
                        persist_ops.extend(projection.ops);
                        if let Some(event_id) = projection.event_id.clone() {
                            self.state.pending_chain_event_ids.insert(event_id.clone());
                            pending_chain_event_ids.push(event_id);
                        }
                        // 接龙声明不是普通 message 行：新租户 hydration 可能只有 type=2
                        // 公告更新，但 canonical chain_* 已能完整还原。PersistAtomic 成功后
                        // 仍需把 chainProjection 发给客户端，否则初次打开永远没有声明卡。
                        chain_domain_events.push(projection.event);
                    }
                }
                // type=1 编译器已在同一 PersistAtomic 中写入 viewer-scoped 未读，禁止再写共享 channel 未读。
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
                let persist_type_counts =
                    crate::sync::observability::storage_op_type_counts(&persist_ops);
                crate::offline_sync_info!(trace_enabled,
                    hop = "sync.persist.summary",
                    phase = "before_commit",
                    upstream_corr = corr.raw(),
                    corr = persist_corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(persist_corr),
                    channel_id = channel_id.as_str(),
                    event_seq = commit_target.0,
                    event_type = "batch",
                    msg_id = "",
                    source = "sync_notify",
                    operation_id = format!("sync-persist:{}", persist_corr.raw()),
                    operation_count = persist_ops.len(),
                    type_counts = %persist_type_counts,
                    cursor_target = commit_target.0,
                    "同步批次事务准备提交"
                );
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.insert(persist_corr, commit_target);
                }
                if self
                    .state
                    .recovery_session
                    .is_collecting_for(self.config.auth_user_id.as_str())
                {
                    self.state
                        .recovery_session
                        .await_commit(channel_id, commit_target);
                }
                if trigger == SyncTrigger::PongGap {
                    self.state.pong_gap_batch.register_persist(persist_corr);
                }
                out.push(Effect::PersistAtomic {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                crate::offline_sync_info!(
                    trace_enabled,
                    hop = "recovery.persist_dispatched",
                    upstream_corr = corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                    corr = persist_corr.raw(),
                    channel_id = channel_id.as_str(),
                    from_seq = from_exclusive.0,
                    to_seq = commit_target.0,
                    event_count = events.len(),
                    message_count = messages.len(),
                    needs_continuation,
                    "recovery atomic write dispatched"
                );
                let mut pending_domain_events = if trigger == SyncTrigger::Hydration {
                    Vec::new()
                } else {
                    crate::acl::to_effect::sync_mutation_emits_with_auth_and_path(
                        &events,
                        &messages,
                        auth_user_id,
                        if trigger == SyncTrigger::PongGap {
                            "offline_recovery"
                        } else {
                            "sync_replay"
                        },
                    )
                    .into_iter()
                    .filter_map(|effect| match effect {
                        Effect::Emit { event } => Some(event.0.as_ref().to_vec()),
                        _ => None,
                    })
                    .collect()
                };
                pending_domain_events.extend(chain_domain_events);
                self.state.corr_map.insert(
                    persist_corr,
                    CorrelationContext::ChannelPersist {
                        channel_id,
                        trigger,
                        wants_continuation: needs_continuation,
                        channel_updates: Vec::new(),
                        pending_domain_events,
                        pending_chain_event_ids,
                        pending_send_reconciliations: if trigger == SyncTrigger::Hydration {
                            Vec::new()
                        } else {
                            pending_send_reconciliations
                        },
                        member_projection: member_projection
                            .map(|(_, projection, _, _)| Box::new(projection)),
                    },
                );
            }
            SyncResponse::TooLong { reset_to } => {
                crate::offline_sync_warn!(trace_enabled,
                    hop = "sync.response",
                    corr = corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                    channel_id = channel_id.as_str(),
                    trigger = ?trigger,
                    kind = "too_long",
                    reset_to = reset_to.0,
                    "sync/notify response requires snapshot recovery"
                );
                if trigger == SyncTrigger::Hydration {
                    self.fail_hydration_for_channel(
                        channel_id,
                        "hydration sync requires TooLong recovery",
                        out,
                    );
                    return Ok(());
                }
                // 覆盖式恢复不先修改内存 cursor、清 buffer 或清 dialog：若窗口拉取/持久化
                // 失败，旧历史与旧 cursor 必须仍可读；成功回包由 TooLongReloadPersist
                // 在同一 PersistAtomic 回执后一次提交。
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
                crate::offline_sync_info!(trace_enabled,
                    hop = "sync.response",
                    corr = corr.raw(),
                    track_id = crate::acl::sync_http_effects::sync_track_id(corr),
                    channel_id = channel_id.as_str(),
                    trigger = ?trigger,
                    kind = "snapshot",
                    reset_to = snap.reset_to.0,
                    message_count = snap.messages.len(),
                    "sync/notify response decoded"
                );
                // Snapshot 与 too_long 共用“覆盖而非先删”不变量：只产 BatchUpsert，
                // 持久化失败时既有 message 行保持不变。
                let persist_corr = self.alloc_corr_internal();
                let reset_to = snap.reset_to;
                let mut persist_ops = crate::acl::to_effect::batch_upsert_events(&snap.messages);
                persist_ops.push(crate::acl::to_effect::advance_cursor_op(
                    channel_id, reset_to,
                ));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.insert(persist_corr, reset_to);
                }
                if trigger == SyncTrigger::PongGap {
                    self.state.pong_gap_batch.register_persist(persist_corr);
                }
                out.push(Effect::PersistAtomic {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                self.state.corr_map.insert(
                    persist_corr,
                    CorrelationContext::ChannelPersist {
                        channel_id,
                        trigger,
                        wants_continuation: false,
                        channel_updates: Vec::new(),
                        pending_domain_events: Vec::new(),
                        pending_chain_event_ids: Vec::new(),
                        pending_send_reconciliations: Vec::new(),
                        member_projection: None,
                    },
                );
            }
        }

        Ok(())
    }

    /// Consume a sealed type7 terminal only through an all-write atomic storage boundary.
    ///
    /// Go returns every viewer-visible fact after `fromSeq`, so a newly discovered closed channel
    /// can legitimately contain visible message events followed by type7 in one response. The
    /// visible prefix and terminal cursor/tombstone therefore commit as one transaction. An invalid
    /// deterministic response is retained for a later reconnect/manual retry rather than immediately
    /// requesting the identical response forever.
    #[allow(clippy::too_many_arguments)]
    fn handle_terminal_sync_event(
        &mut self,
        upstream_corr: helix_core::Correlation,
        channel_id: ChannelId,
        trigger: SyncTrigger,
        from_exclusive: Seq,
        terminal_seq: Seq,
        events: Vec<crate::sync_session::EventEnvelope>,
        messages: std::collections::HashMap<String, crate::sync_session::PostFields>,
        next_seq: Seq,
        needs_continuation: bool,
        persona: crate::sync_session::SyncPersona,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let trace_enabled = self.config.offline_sync_diagnostics.trace;
        let terminal_count = events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    crate::sync_session::EventKind::ChannelTerminalClosed
                )
            })
            .count();
        let terminal_is_last = events.last().is_some_and(|event| {
            matches!(
                event.kind,
                crate::sync_session::EventKind::ChannelTerminalClosed
            )
        });
        if terminal_count != 1
            || !terminal_is_last
            || next_seq != terminal_seq
            || needs_continuation
        {
            crate::offline_sync_warn!(
                trace_enabled,
                channel_id = channel_id.as_str(),
                event_count = events.len(),
                terminal_count,
                terminal_is_last,
                next_seq = next_seq.0,
                terminal_seq = terminal_seq.0,
                needs_continuation,
                "rejecting malformed terminal sync batch; retaining cursor without immediate retry"
            );
            return Ok(());
        }
        let events = match crate::sync_session::SyncBatchFacts::from_events(
            channel_id,
            from_exclusive,
            next_seq,
            events,
        ) {
            Ok(facts) => facts.events,
            Err(reason) => {
                crate::offline_sync_warn!(trace_enabled,
                    channel_id = channel_id.as_str(),
                    terminal_seq = terminal_seq.0,
                    reason,
                    "rejecting incoherent terminal sync facts; retaining cursor without immediate retry"
                );
                return Ok(());
            }
        };

        let Some(channel) = self.state.channels.get(&channel_id) else {
            crate::offline_sync_warn!(
                trace_enabled,
                channel_id = channel_id.as_str(),
                "terminal sync for unknown channel ignored"
            );
            return Ok(());
        };
        if channel.is_terminal() || terminal_seq <= channel.cursor.value() {
            // Same seq replay is an idempotent no-op. A restored tombstone is already terminal,
            // and no second terminal event/cursor write must be emitted.
            return Ok(());
        }
        let persist_corr = self.alloc_corr_internal();
        let prefix = &events[..events.len().saturating_sub(1)];
        let auth_user_id = self.config.auth_user_id.as_str();
        let observation = crate::sync::observability::SyncObservation::new(
            upstream_corr.raw(),
            crate::acl::sync_http_effects::sync_track_id(upstream_corr),
            "sync_notify",
            self.config.offline_sync_diagnostics.clone(),
        );
        let mut ops = crate::acl::to_effect::batch_upsert_events_with_messages_and_auth_observed(
            prefix,
            &messages,
            auth_user_id,
            if trigger == SyncTrigger::Hydration {
                crate::acl::sync_effects::SyncApplyMode::HydrationHistory
            } else {
                crate::acl::sync_effects::SyncApplyMode::LiveRecovery
            },
            Some(&observation),
        );
        let member_projection = prepare_sync_projection(&persona, channel_id, auth_user_id)?;
        if let Some((row, _, revision, _)) = member_projection.as_ref() {
            ops.extend(crate::acl::to_effect::canonical_member_projection_ops(
                channel_id,
                auth_user_id,
                *revision,
                row.clone(),
            ));
        }
        // terminal prefix 与普通 Sync 共用绝对 memberProjection，不再追加共享 channel 未读。
        // Terminal batches can contain ordinary chain events before type7. Keep their durable
        // projection in the same transaction as the prefix messages and terminal tombstone.
        let mut chain_domain_events = Vec::new();
        let mut pending_chain_event_ids = Vec::new();
        for event in prefix {
            if let Some(projection) = crate::chain::synced_projection(event)? {
                let duplicate = projection.event_id.as_ref().is_some_and(|event_id| {
                    self.state.seen_chain_event_ids.contains(event_id)
                        || self.state.pending_chain_event_ids.contains(event_id)
                });
                if duplicate {
                    continue;
                }
                ops.extend(projection.ops);
                if let Some(event_id) = projection.event_id.clone() {
                    self.state.pending_chain_event_ids.insert(event_id.clone());
                    pending_chain_event_ids.push(event_id);
                }
                chain_domain_events.push(projection.event);
            }
        }
        ops.push(crate::acl::to_effect::terminal_tombstone_and_cursor_op(
            channel_id,
            terminal_seq,
        ));
        let persist_type_counts = crate::sync::observability::storage_op_type_counts(&ops);
        crate::offline_sync_info!(trace_enabled,
            hop = "sync.persist.summary",
            phase = "before_commit",
            upstream_corr = upstream_corr.raw(),
            corr = persist_corr.raw(),
            track_id = crate::acl::sync_http_effects::sync_track_id(persist_corr),
            channel_id = channel_id.as_str(),
            event_seq = terminal_seq.0,
            event_type = "batch",
            msg_id = "",
            source = "sync_notify",
            operation_id = format!("sync-persist:{}", persist_corr.raw()),
            operation_count = ops.len(),
            type_counts = %persist_type_counts,
            cursor_target = terminal_seq.0,
            "终态同步批次事务准备提交"
        );
        let pending_domain_events = crate::acl::to_effect::sync_mutation_emits_with_auth_and_path(
            prefix,
            &messages,
            auth_user_id,
            if trigger == SyncTrigger::PongGap {
                "offline_recovery"
            } else {
                "sync_replay"
            },
        )
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(event.0.as_ref().to_vec()),
            _ => None,
        })
        .chain(chain_domain_events)
        .collect();
        out.push(Effect::PersistAtomic {
            corr: persist_corr,
            ops,
        });
        crate::offline_sync_info!(trace_enabled,
            hop = "sync.terminal_persist_dispatched",
            upstream_corr = upstream_corr.raw(),
            track_id = crate::acl::sync_http_effects::sync_track_id(upstream_corr),
            corr = persist_corr.raw(),
            channel_id = channel_id.as_str(),
            from_seq = from_exclusive.0,
            terminal_seq = terminal_seq.0,
            event_count = events.len(),
            message_count = messages.len(),
            trigger = ?trigger,
            "terminal sync atomic write dispatched"
        );
        if self
            .state
            .recovery_session
            .is_collecting_for(self.config.auth_user_id.as_str())
        {
            self.state
                .recovery_session
                .await_commit(channel_id, terminal_seq);
        }
        if trigger == SyncTrigger::PongGap {
            self.state.pong_gap_batch.register_persist(persist_corr);
        }
        self.state.corr_map.insert(
            persist_corr,
            CorrelationContext::ChannelTerminalPersist {
                channel_id,
                terminal_seq,
                trigger,
                pending_domain_events,
                pending_chain_event_ids,
                member_projection: member_projection
                    .map(|(_, projection, _, _)| Box::new(projection)),
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
