use crate::error::ImError;
use crate::module::ImModule;
use crate::state::CorrelationContext;
use helix_core::EffectSink;

/// 以明确失败状态结束 dialog 两段式回读，避免无身份或畸形回包静默成功。
fn emit_dialog_list_failed(
    out: &mut EffectSink,
    causation_id: Option<String>,
) -> Result<(), ImError> {
    out.push(
        crate::event::channel::list(serde_json::json!({
            "channels": [],
            "requestId": causation_id,
            "state": "failed",
        }))?
        .into_effect(),
    );
    Ok(())
}

/// Decode the channel row returned after a notify write; sparse WS input is never used as the
/// final event payload, so absent fields retain the durable values already in SQLite.
fn notify_channel_update_from_reply(
    channel_id: crate::state::ChannelId,
    reply: &[u8],
) -> Result<Option<serde_json::Value>, ImError> {
    let rows: serde_json::Value = serde_json::from_slice(reply)
        .map_err(|error| ImError::Parse(format!("notify channel readback: {error}")))?;
    let Some(row) = rows.as_array().and_then(|items| {
        items.iter().find(|item| {
            item.get("id")
                .or_else(|| item.get("channel_id"))
                .or_else(|| item.get("channelId"))
                .and_then(serde_json::Value::as_str)
                == Some(channel_id.as_str())
        })
    }) else {
        return Ok(None);
    };
    let shaped = crate::query::render_ready::channel::shape_channel_row(row);
    let notify = shaped
        .get("notifyProps")
        .and_then(serde_json::Value::as_str);
    if !matches!(notify, Some("NORMAL" | "STRONG" | "IGNORE")) {
        return Err(ImError::Parse(
            "notify channel readback missing valid scalar notifyProps".to_string(),
        ));
    }
    Ok(Some(shaped))
}

impl ImModule {
    /// 处理 PortReply（Persist/Http 的异步结果回报）。
    ///
    /// 统一 `corr_map` 单表 + `match CorrelationContext` 分发（类别互斥靠 corr 全局唯一，见 state.rs
    /// `CorrelationContext` 不变量）：`remove(corr)` 取上下文 → `ChannelPersist` 委托
    /// `channel.on_persist_ok` 推 cursor / `OptimisticSend` 找 PendingSend / `ScanCursors`（E4）解析
    /// on_start cursor rows（channel_id TEXT + last_event_seq INTEGER）→ register_channel；随后由
    /// `ScanChannelProjections` 过滤删除/关闭频道后才允许 proactive sync。
    pub(super) fn dispatch_port_reply(
        &mut self,
        corr: helix_core::Correlation,
        outcome: &helix_core::tick::PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;
        if let Some(pending) = self.state.pending_schedule_media.remove(&corr) {
            return self.schedule_media_reply(pending, outcome, now_ms, out);
        }

        if self.state.pending_media_rehydrate_corr == Some(corr) {
            self.state.pending_media_rehydrate_corr = None;
            match outcome {
                PortOutcome::Ok(reply) => {
                    let recovered =
                        crate::send::upload_props::decode_durable_scan(reply.0.as_ref())?;
                    let recovered = recovered
                        .into_iter()
                        .filter(|operation| {
                            !self
                                .state
                                .pending_sends
                                .contains_key(operation.temporary_id())
                        })
                        .collect::<Vec<_>>();
                    if recovered.is_empty() {
                        self.state.media_recovery_ready = true;
                        return Ok(());
                    }

                    let temporary_ids = recovered
                        .iter()
                        .map(|operation| operation.temporary_id().0.clone())
                        .collect::<std::collections::BTreeSet<_>>();
                    let compensation_corr = self.alloc_corr_internal();
                    out.push(helix_core::Effect::PersistAtomic {
                        corr: compensation_corr,
                        ops: vec![helix_core::effect::StorageOp::BatchUpdate(
                            helix_core::effect::BatchUpdateSpec {
                                table: "message",
                                key_col: "temporary_id",
                                key_vals: temporary_ids
                                    .into_iter()
                                    .map(helix_core::effect::SqlValue::Text)
                                    .collect(),
                                patch: vec![
                                    (
                                        "send_status".to_string(),
                                        helix_core::effect::SqlValue::Text("unsend".to_string()),
                                    ),
                                    (
                                        "upload_progress_percent".to_string(),
                                        helix_core::effect::SqlValue::Integer(0),
                                    ),
                                ],
                            },
                        )],
                    });
                    self.state.pending_media_recovery_compensation =
                        Some((compensation_corr, recovered));
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(error = ?error, "pending_media recovery scan failed");
                }
            }
            return Ok(());
        }

        if self
            .state
            .pending_media_recovery_compensation
            .as_ref()
            .is_some_and(|(pending_corr, _)| *pending_corr == corr)
        {
            let (_, recovered) = self
                .state
                .pending_media_recovery_compensation
                .take()
                .expect("recovery compensation correlation was checked");
            match outcome {
                PortOutcome::Ok(_) => {
                    let channels = recovered
                        .iter()
                        .map(crate::send::upload_props::FailedMediaOp::channel_id)
                        .collect::<std::collections::BTreeSet<_>>();
                    for operation in recovered {
                        // on_start scan 与本会话新 send 可并行。快照中的 live temporary_id
                        // 可能已推进甚至完成，绝不能把旧 prepare/put 重新注入失败表；
                        // scan 只负责本进程启动前留下的孤儿 journal。
                        if self
                            .state
                            .pending_sends
                            .contains_key(operation.temporary_id())
                        {
                            continue;
                        }
                        self.state
                            .failed_media_ops
                            .entry((operation.temporary_id().clone(), operation.target().clone()))
                            .or_insert(operation);
                    }
                    self.state.media_recovery_ready = true;
                    for channel_id in channels {
                        self.refresh_attached_latest_timeline(channel_id, None, out)?;
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        "pending_media recovery compensation failed; retry remains closed"
                    );
                }
            }
            return Ok(());
        }

        if let Some(commit) = self.state.pending_media_completion_persists.remove(&corr) {
            return self.handle_media_completion_persist_reply(commit, outcome, out);
        }

        if let Some(reset) = self.state.pending_media_retry_resets.remove(&corr) {
            return crate::send::retry_send::handle_media_retry_reset_reply(
                self, reset, outcome, out,
            );
        }

        if let Some(route) = self.state.file_upload_progress_persists.remove(&corr) {
            return self.handle_file_upload_progress_persist_reply(corr, route, outcome, out);
        }

        if let Some(pending_media) = self.state.pending_media_ops.remove(&corr) {
            if matches!(
                &pending_media,
                crate::send::upload_props::PendingMediaOp::Put(_)
            ) {
                if matches!(outcome, PortOutcome::Ok(_))
                    && self.defer_media_put_until_progress_commits(corr, &pending_media)
                {
                    return Ok(());
                }
                self.finish_file_upload_progress(corr);
            }
            return self.handle_media_port_reply(pending_media, outcome, out);
        }

        if let Some(pending_media) = self.state.pending_media_stage_persists.remove(&corr) {
            return self.handle_media_stage_persist_reply(pending_media, outcome, out);
        }

        if let Some(pending_upload) = self.state.pending_uploads.remove(&corr) {
            self.finish_file_upload_progress(corr);
            return self.handle_upload_port_reply(pending_upload, outcome, out);
        }

        // 单次 remove 取出上下文（取代原 4 张表逐一 if-let 试探）。
        // 未命中（late/未知 corr）→ no-op，边界零信任（等价原"无人认领 → Ok(())"）。
        let Some(ctx) = self.state.corr_map.remove(&corr) else {
            return Ok(());
        };

        match ctx {
            CorrelationContext::Translation { request, timer } => {
                self.translation_reply(*request, timer, outcome, out)?;
            }
            CorrelationContext::CategoryChain { pending } => {
                self.handle_category_reply(*pending, outcome, out)?
            }
            CorrelationContext::ChainHttp { request } => {
                self.handle_chain_http_reply(*request, outcome, out)?;
            }
            CorrelationContext::ChainPersist {
                request,
                authority,
                event_name,
                mutation_state,
                error_code,
            } => self.handle_chain_persist_reply(
                *request,
                *authority,
                event_name,
                mutation_state,
                error_code,
                outcome,
                out,
            )?,
            CorrelationContext::ChainMutationPersist {
                request,
                state,
                entry_id,
                error_code,
                retry_reconcile,
            } => self.handle_chain_mutation_persist_reply(
                *request,
                state,
                entry_id,
                error_code,
                retry_reconcile,
                outcome,
                out,
            )?,
            // MessageV3 terminal events 只在同一业务写真实提交后释放一次。
            CorrelationContext::MessageV3Commit { terminal_events } => {
                if matches!(outcome, PortOutcome::Ok(_)) {
                    for event in terminal_events {
                        out.push(helix_core::Effect::Emit {
                            event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(event)),
                        });
                    }
                }
            }
            CorrelationContext::CategoryPostReadback { pending } => {
                self.handle_category_post_readback(*pending, outcome, out)?
            }
            CorrelationContext::MessageV3PostPersist {
                event,
                received_data,
            } => {
                self.handle_message_v3_post_persist_reply(*event, *received_data, outcome, out)?;
            }
            CorrelationContext::CanonicalChainPostHydration { event } => {
                self.handle_chain_post_hydration_reply(*event, outcome, out)?;
            }
            CorrelationContext::CanonicalStreamPersist { event } => {
                self.handle_canonical_stream_persist_reply(*event, outcome, out)?;
            }
            CorrelationContext::MessageV3PostReadback {
                received_data,
                channel_id,
                causation_id,
            } => self.handle_message_v3_post_readback_reply(
                *received_data,
                channel_id,
                causation_id,
                outcome,
                out,
            )?,
            CorrelationContext::MessageV3ClientAck {
                channel_id,
                platform,
            } => {
                self.handle_message_v3_client_ack_reply(channel_id, platform, outcome, out)?;
            }
            CorrelationContext::MessageV3RevokeChannelPersist { channel_id } => {
                self.handle_message_v3_revoke_channel_persist_reply(channel_id, outcome, out);
            }
            CorrelationContext::MessageV3RevokeChannelReadback => {
                self.handle_message_v3_revoke_channel_readback_reply(outcome, out)?;
            }
            CorrelationContext::MessageV3SyncDialogReadback => {
                self.handle_message_v3_sync_dialog_readback_reply(outcome, out)?;
            }
            CorrelationContext::MessageV3ReactionPersist { event } => {
                self.handle_message_v3_reaction_persist_reply(*event, outcome, out)?
            }
            CorrelationContext::MessageV3ReactionReadback { message_id } => {
                self.handle_message_v3_reaction_readback_reply(message_id, outcome, out)?;
            }
            CorrelationContext::MessageV3UrgentPersist { event } => {
                self.handle_message_v3_urgent_persist_reply(*event, outcome, out)?
            }
            CorrelationContext::MessageV3UrgentReadback { message_id } => {
                self.handle_message_v3_urgent_readback_reply(message_id, outcome, out)?;
            }
            CorrelationContext::MessageV3TemplatePersist { event } => {
                self.handle_message_v3_template_persist_reply(*event, outcome, out)?
            }
            CorrelationContext::MessageV3TemplateReadback => {
                self.handle_message_v3_template_readback_reply(outcome, out)?;
            }
            // 路由（E4）：on_start Scan 回报 → 加载 cursor + 触发 proactive resync
            CorrelationContext::ScanCursors => match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_scan_reply(reply, out)?;
                }
                PortOutcome::Err(e) => {
                    // 降级：cursor scan 失败（包括首次启动表不存在）仍读取 channel 投影，
                    // 已删除频道不能因为 cursor 表异常而重新进入补偿。
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?e,
                        "scan channel_event_cursor failed; continuing with channel projection scan"
                    );
                    self.request_channel_projection_scan(out);
                }
            },

            CorrelationContext::ScanChannelProjections => match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_channel_projection_scan_reply(reply, out)?;
                }
                PortOutcome::Err(e) => {
                    // 投影扫描失败时保持启动补偿闸门关闭，不伪造 terminal，也不让未知
                    // 删除状态的频道继续发 sync；下一次 on_start 会重新扫描。
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?e,
                        "scan channel projection failed; startup sync remains fail-closed"
                    );
                }
            },

            CorrelationContext::IncrementPullHttp => {
                self.handle_increment_pull_http(corr, outcome, now_ms, out)
            }
            CorrelationContext::IncrementPullPersist => {
                self.handle_increment_pull_persist(corr, outcome, out)
            }
            CorrelationContext::IncrementMessageTimestampScan {
                connection_id,
                cursors,
            } => self.handle_increment_message_timestamp_scan_reply(
                connection_id,
                cursors,
                outcome,
                out,
            )?,

            // 路由：channel sync Persist 回报（cursor 推进 + E3 续拉）
            CorrelationContext::ChannelPersist {
                channel_id,
                trigger,
                wants_continuation,
                channel_updates,
                pending_domain_events,
                has_category_posts,
                pending_chain_event_ids,
                pending_send_reconciliations,
                member_projection,
            } => {
                let effect_start = out.as_slice().len();
                let auto_req_id = (trigger == crate::state::SyncTrigger::Hydration)
                    .then(|| self.state.hydration_req_ids.get(&channel_id).cloned())
                    .flatten()
                    .filter(|req_id| self.scope_hydration.is_active(channel_id, req_id));
                self.handle_channel_persist_reply(
                    corr,
                    channel_id,
                    trigger,
                    wants_continuation,
                    channel_updates,
                    pending_domain_events,
                    has_category_posts,
                    pending_chain_event_ids,
                    pending_send_reconciliations,
                    member_projection,
                    outcome,
                    out,
                )?;
                if let Some(req_id) = auto_req_id.as_deref() {
                    if !crate::scope_hydration::has_hydration_stage(&self.state, channel_id, req_id)
                    {
                        self.scope_hydration.clear(channel_id);
                        crate::scope_hydration::drop_auto_read_emits(out, effect_start, req_id);
                    }
                }
            }

            CorrelationContext::ChannelTerminalPersist {
                channel_id,
                terminal_seq,
                trigger,
                pending_domain_events,
                has_category_posts,
                pending_chain_event_ids,
                member_projection,
            } => {
                let effect_start = out.as_slice().len();
                let auto_req_id = (trigger == crate::state::SyncTrigger::Hydration)
                    .then(|| self.state.hydration_req_ids.get(&channel_id).cloned())
                    .flatten()
                    .filter(|req_id| self.scope_hydration.is_active(channel_id, req_id));
                self.handle_channel_terminal_persist_reply(
                    corr,
                    channel_id,
                    terminal_seq,
                    trigger,
                    pending_domain_events,
                    has_category_posts,
                    pending_chain_event_ids,
                    member_projection,
                    outcome,
                    out,
                )?;
                if let Some(req_id) = auto_req_id.as_deref() {
                    if !crate::scope_hydration::has_hydration_stage(&self.state, channel_id, req_id)
                    {
                        self.scope_hydration.clear(channel_id);
                        crate::scope_hydration::drop_auto_read_emits(out, effect_start, req_id);
                    }
                }
            }

            CorrelationContext::IncrementBatchPersist {
                projections,
                batch_id,
            } => self.handle_increment_batch_persist_reply(projections, batch_id, outcome, out)?,

            CorrelationContext::SubtopicIncrementBatchPersist {
                parent_channel_id,
                batch_key,
                batch_id,
                projections,
            } => self.handle_subtopic_increment_batch_persist_reply(
                parent_channel_id,
                batch_key,
                batch_id,
                projections,
                outcome,
                out,
            )?,

            CorrelationContext::ChannelSyncPage {
                channel_sync_session_id,
                generation,
                offset,
                req_id,
            } => self.handle_channel_sync_page_reply(
                channel_sync_session_id,
                generation,
                offset,
                req_id,
                outcome,
                out,
            )?,

            CorrelationContext::ChannelSyncPageMemberSnapshot {
                channel_sync_session_id,
                generation,
                offset,
                req_id,
                channel_rows,
            } => self.handle_channel_sync_page_member_reply(
                channel_sync_session_id,
                generation,
                offset,
                req_id,
                channel_rows,
                outcome,
                out,
            )?,

            CorrelationContext::IncrementHydrationPersist {
                channel_id,
                req_id,
                need_sync,
                raw_increment,
                snapshot,
                emit_channel_increment,
            } => {
                let effect_start = out.as_slice().len();
                let auto_scope = self.scope_hydration.is_active(channel_id, &req_id);
                self.handle_increment_hydration_persist(
                    channel_id,
                    req_id.clone(),
                    need_sync,
                    raw_increment,
                    snapshot,
                    emit_channel_increment,
                    outcome,
                    out,
                )?;
                if auto_scope
                    && !crate::scope_hydration::has_hydration_stage(
                        &self.state,
                        channel_id,
                        &req_id,
                    )
                {
                    self.scope_hydration.clear(channel_id);
                    crate::scope_hydration::drop_auto_read_emits(out, effect_start, &req_id);
                }
            }

            CorrelationContext::HydrationChannelReadback { req_id, channel_id } => {
                let effect_start = out.as_slice().len();
                let auto_scope = self.scope_hydration.is_active(channel_id, &req_id);
                self.handle_hydration_channel_readback(req_id.clone(), channel_id, outcome, out)?;
                if auto_scope
                    && !crate::scope_hydration::has_hydration_stage(
                        &self.state,
                        channel_id,
                        &req_id,
                    )
                {
                    self.scope_hydration.clear(channel_id);
                    crate::scope_hydration::drop_auto_read_emits(out, effect_start, &req_id);
                }
            }
            CorrelationContext::HydrationMemberReadback {
                req_id,
                channel_id,
                channel,
            } => {
                let effect_start = out.as_slice().len();
                let auto_scope = self.scope_hydration.is_active(channel_id, &req_id);
                self.handle_hydration_member_readback(
                    req_id.clone(),
                    channel_id,
                    channel,
                    outcome,
                    out,
                )?;
                if auto_scope
                    && !crate::scope_hydration::has_hydration_stage(
                        &self.state,
                        channel_id,
                        &req_id,
                    )
                {
                    self.scope_hydration.clear(channel_id);
                    crate::scope_hydration::drop_auto_read_emits(out, effect_start, &req_id);
                }
            }
            CorrelationContext::HydrationMessagesReadback {
                req_id,
                channel_id,
                channel,
                member,
            } => {
                let effect_start = out.as_slice().len();
                let auto_scope = self.scope_hydration.is_active(channel_id, &req_id);
                self.handle_hydration_messages_readback(
                    req_id.clone(),
                    channel_id,
                    channel,
                    member,
                    outcome,
                    out,
                )?;
                if auto_scope
                    && !crate::scope_hydration::has_hydration_stage(
                        &self.state,
                        channel_id,
                        &req_id,
                    )
                {
                    self.scope_hydration.clear(channel_id);
                    crate::scope_hydration::drop_auto_read_emits(out, effect_start, &req_id);
                }
            }
            CorrelationContext::HydrationCursorReadback {
                req_id,
                channel_id,
                channel,
                member,
                messages,
            } => {
                let effect_start = out.as_slice().len();
                let auto_scope = self.scope_hydration.is_active(channel_id, &req_id);
                self.handle_hydration_cursor_readback(
                    req_id.clone(),
                    channel_id,
                    channel,
                    member,
                    messages,
                    outcome,
                    out,
                )?;
                if auto_scope
                    && !crate::scope_hydration::has_hydration_stage(
                        &self.state,
                        channel_id,
                        &req_id,
                    )
                {
                    self.scope_hydration.clear(channel_id);
                    crate::scope_hydration::drop_auto_read_emits(out, effect_start, &req_id);
                }
            }

            CorrelationContext::ChannelUpdateByPost { pending } => match outcome {
                PortOutcome::Ok(reply) => {
                    if let Some(data) = pending.event_data_from_channel_reply(reply.0.as_ref()) {
                        out.push(crate::event::channel::update(data)?.into_effect());
                        self.schedule_topic_activity_projection(&pending, reply.0.as_ref(), out);
                    } else {
                        tracing::warn!(
                            channel_id = pending.channel_id.as_str(),
                            corr = corr.raw(),
                            "channel row readback empty; skip MessageV3 update"
                        );
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = pending.channel_id.as_str(),
                        corr = corr.raw(),
                        error = ?e,
                        "channel update write/read failed"
                    );
                }
            },

            CorrelationContext::UpdateChannelDialogPersist {
                channel_id,
                channel,
                member_channel,
                causation_id: _,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    let member_read_event = member_channel
                        .as_deref()
                        .map(|projection| {
                            crate::event::read::channel_from_member_projection(
                                channel_id.as_str(),
                                projection,
                            )
                        })
                        .transpose()?;
                    let mut update = channel.as_ref().clone();
                    if !update.is_object() {
                        update = serde_json::json!({});
                    }
                    if let Some(object) = update.as_object_mut() {
                        object.insert(
                            "channelId".to_string(),
                            serde_json::Value::String(channel_id.as_str().to_string()),
                        );
                        if let Some(member) = member_channel.as_deref() {
                            if let Some(member_update) = member
                                .get("dialogPatch")
                                .and_then(serde_json::Value::as_object)
                            {
                                object.extend(member_update.clone());
                            }
                        }
                    }
                    out.push(crate::event::channel::update(update)?.into_effect());
                    if let Some(event) = member_read_event {
                        out.push(event.into_effect());
                    }
                    if let Some(event) = crate::event::channel::settings_from_authority(
                        channel_id.as_str(),
                        channel.as_ref(),
                    )? {
                        out.push(event.into_effect());
                    }
                    let _event_time = now_ms;
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?error,
                        "update_channel persistence failed; suppress MessageV3 updates"
                    );
                }
            },

            CorrelationContext::MemberProjectionPersist {
                channel_id,
                expected_revision,
                expected_effect_id,
                expected_projection,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    debug_assert_eq!(
                        expected_projection.projection_revision,
                        Some(expected_revision)
                    );
                    debug_assert_eq!(
                        expected_projection.effect_id.as_deref(),
                        Some(expected_effect_id.as_str())
                    );
                    self.queue_member_projection_readback(channel_id, expected_projection, out)?;
                }
                PortOutcome::Err(error) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    corr = corr.raw(),
                    error = ?error,
                    "member projection persistence failed"
                ),
            },

            CorrelationContext::MemberProjectionReadback {
                channel_id,
                expected_revision,
                expected_effect_id,
                expected_projection,
            } => match outcome {
                PortOutcome::Ok(reply) => {
                    debug_assert_eq!(
                        expected_projection.projection_revision,
                        Some(expected_revision)
                    );
                    debug_assert_eq!(
                        expected_projection.effect_id.as_deref(),
                        Some(expected_effect_id.as_str())
                    );
                    crate::ws::handlers::member_projection::emit_committed_projection(
                        reply.0.as_ref(),
                        channel_id,
                        &expected_projection,
                        self.config.auth_user_id.as_str(),
                        now_ms,
                        out,
                    )?;
                }
                PortOutcome::Err(error) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    corr = corr.raw(),
                    error = ?error,
                    "member projection readback failed"
                ),
            },

            CorrelationContext::NotifyChannelPersist { channel_id } => match outcome {
                PortOutcome::Ok(reply) => {
                    if let Some(update) =
                        notify_channel_update_from_reply(channel_id, reply.0.as_ref())?
                    {
                        out.push(crate::event::channel::update(update)?.into_effect());
                    } else {
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            corr = corr.raw(),
                            "notify channel readback empty; skip MessageV3 update"
                        );
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        corr = corr.raw(),
                        error = ?error,
                        "notify channel persistence failed; suppress MessageV3 update"
                    );
                }
            },

            CorrelationContext::TooLongReload {
                channel_id,
                reset_to,
                trigger,
            } => match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_too_long_reload_reply(channel_id, reset_to, trigger, reply, out)?;
                }
                PortOutcome::Err(e) => {
                    self.clear_too_long_recovery(channel_id);
                    self.fail_hydration_for_channel(
                        channel_id,
                        &format!("too_long reload failed: {e:?}"),
                        out,
                    );
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        reset_to = reset_to.0,
                        error = ?e,
                        "too_long getLatestPost reload failed"
                    );
                }
            },

            CorrelationContext::TooLongReloadPersist {
                channel_id,
                reset_to,
                channel_updates,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    self.clear_too_long_recovery(channel_id);
                    let committed = self
                        .state
                        .channels
                        .get_mut(&channel_id)
                        .is_some_and(|channel| channel.commit_too_long_after_atomic(reset_to));
                    if committed {
                        for pending in channel_updates {
                            // 原子写不返回行；提交成功后再读回绝对未读值，复用统一频道投影。
                            let read_corr = self.alloc_corr_internal();
                            out.push(helix_core::Effect::Persist {
                                corr: read_corr,
                                ops: vec![crate::acl::to_effect::get_channel_row_op(channel_id)],
                            });
                            self.state.corr_map.insert(
                                read_corr,
                                CorrelationContext::ChannelUpdateByPost { pending },
                            );
                        }
                        self.finish_increment_hydration(channel_id, out)?;
                        // resetTo 只覆盖原响应时刻；再从已提交cursor有界补拉，接住恢复期间的新事件。
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
                    } else {
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            reset_to = reset_to.0,
                            "too_long reload receipt no longer matches channel generation"
                        );
                        self.fail_hydration_for_channel(
                            channel_id,
                            "too_long recovery superseded",
                            out,
                        );
                    }
                }
                PortOutcome::Err(e) => {
                    self.clear_too_long_recovery(channel_id);
                    self.fail_hydration_for_channel(
                        channel_id,
                        &format!("too_long reload persist failed: {e:?}"),
                        out,
                    );
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        error = ?e,
                        "too_long reload persist failed"
                    );
                }
            },

            // 路由：发送 Persist 回报（P1 乐观落库）
            CorrelationContext::OptimisticSend { temporary_id } => {
                self.handle_optimistic_send_reply(temporary_id, outcome, out)?;
            }

            // posts/create HTTP 只结算 transport admission；sent 继续等待 WS echo。
            CorrelationContext::OutboundSendHttp {
                channel_id,
                temporary_id,
            } => {
                self.handle_outbound_send_http_reply(channel_id, temporary_id, outcome, out)?;
            }
            CorrelationContext::AuthoritativeSendTerminalPersist {
                temporary_id,
                terminal_event,
            } => {
                // corr已由统一dispatcher取走；重复/迟到回报不能再次释放终态。
                match outcome {
                    PortOutcome::Ok(_) => out.push(helix_core::Effect::Emit {
                        event: helix_core::effect::DomainEventBytes(terminal_event),
                    }),
                    PortOutcome::Err(error) => tracing::warn!(
                        tmp_id = temporary_id.0.as_str(), error = ?error,
                        "authoritative send terminal persist failed; suppress received"
                    ),
                }
            }
            CorrelationContext::AuthoritativeSendReconcilePersist { temporary_id } => {
                if let PortOutcome::Err(error) = outcome {
                    tracing::warn!(
                        tmp_id = temporary_id.0.as_str(),
                        error = ?error,
                        "authoritative retry reconcile persist failed"
                    );
                }
            }
            CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(window_token) = window_token.as_deref() {
                        self.refresh_attached_timeline(channel_id, window_token, causation_id, out)?
                    } else {
                        self.refresh_attached_latest_timeline(channel_id, causation_id, out)?
                    }
                }
                PortOutcome::Err(error) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?error,
                    "send terminal persist failed; suppressing timeline refresh"
                ),
            },
            CorrelationContext::OutboundScheduleCreate {
                channel_id,
                request_id,
            } => {
                let succeeded = match outcome {
                    PortOutcome::Ok(reply) => {
                        crate::http_envelope::unwrap_success_envelope(reply.0.as_ref(), "schedule")
                            .ok()
                            .and_then(|body| {
                                serde_json::from_slice::<serde_json::Value>(&body).ok()
                            })
                            .is_some_and(|body| body["status"] == "SUCCESS")
                    }
                    PortOutcome::Err(_) => false,
                };
                // 已由 WS 持久屏障结束的请求，不再接受迟到 HTTP 错误。
                if !succeeded {
                    if let Some(req_id) = request_id.filter(|request_id| {
                        self.state.pending_schedule_requests.get(&channel_id) == Some(request_id)
                    }) {
                        self.state.pending_schedule_requests.remove(&channel_id);
                        out.push(crate::read_relay::emit_read_error(
                            &req_id,
                            "SCHEDULE_REQUEST_FAILED",
                        ));
                    }
                }
            }
            CorrelationContext::OutboundScheduleCancel {
                channel_id,
                request_id,
            } => {
                let succeeded = match outcome {
                    PortOutcome::Ok(reply) => {
                        crate::http_envelope::unwrap_success_envelope(reply.0.as_ref(), "schedule")
                            .ok()
                            .and_then(|body| {
                                serde_json::from_slice::<serde_json::Value>(&body).ok()
                            })
                            .is_some_and(|body| body["status"] == "SUCCESS")
                    }
                    PortOutcome::Err(_) => false,
                };
                // 已由 WS 持久屏障结束的请求，不再接受迟到 HTTP 错误。
                if !succeeded {
                    if let Some(req_id) = request_id.filter(|request_id| {
                        self.state.pending_schedule_cancel_requests.get(&channel_id)
                            == Some(request_id)
                    }) {
                        self.state
                            .pending_schedule_cancel_requests
                            .remove(&channel_id);
                        out.push(crate::read_relay::emit_read_error(
                            &req_id,
                            "SCHEDULE_REQUEST_FAILED",
                        ));
                    }
                }
            }
            CorrelationContext::ScheduleCreatedPersist {
                channel_id,
                revision,
                causation_id,
            } => self.handle_message_v3_schedule_created_persist_reply(
                channel_id,
                revision,
                causation_id,
                outcome,
                out,
            ),
            CorrelationContext::MessageV3ScheduleCreatedReadback => {
                self.handle_message_v3_schedule_created_readback_reply(outcome, out)?;
            }
            CorrelationContext::ScheduleCanceledPersist {
                channel_id,
                revision,
                causation_id,
            } => self.handle_message_v3_schedule_canceled_persist_reply(
                channel_id,
                revision,
                causation_id,
                outcome,
                out,
            ),
            CorrelationContext::MessageV3ScheduleCanceledReadback => {
                self.handle_message_v3_schedule_canceled_readback_reply(outcome, out)?;
            }
            CorrelationContext::MediaFailureCompensation {
                temporary_id,
                channel_id,
                window_token,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    self.state.media_retry_inflight.remove(&temporary_id);
                    self.refresh_attached_timeline(
                        channel_id,
                        window_token.as_deref().unwrap_or("latest"),
                        causation_id,
                        out,
                    )?;
                }
                PortOutcome::Err(error) => tracing::warn!(
                    tmp_id = temporary_id.0.as_str(),
                    channel_id = channel_id.as_str(),
                    error = ?error,
                    "media failure compensation persist failed; retry remains blocked"
                ),
            },
            CorrelationContext::RetrySendRehydrate {
                message_id,
                requested_at_ms,
                request_id,
                action_channel_id,
                window_token,
                lookup_by_id,
            } => {
                crate::send::retry_send::handle_rehydrate_reply(
                    self,
                    message_id,
                    requested_at_ms,
                    request_id,
                    action_channel_id,
                    window_token,
                    lookup_by_id,
                    outcome,
                    out,
                )?;
            }
            // 路由：sync Http 回报（sync/notify 响应）→ 清 per-channel 在途标记（B1 gate_inflight）。
            CorrelationContext::SyncPull {
                channel_id,
                trigger,
            } => {
                let effect_start = out.as_slice().len();
                let auto_req_id = (trigger == crate::state::SyncTrigger::Hydration)
                    .then(|| self.state.hydration_req_ids.get(&channel_id).cloned())
                    .flatten()
                    .filter(|req_id| self.scope_hydration.is_active(channel_id, req_id));
                self.handle_sync_pull_reply(corr, channel_id, trigger, outcome, out)?;
                if let Some(req_id) = auto_req_id.as_deref() {
                    if !crate::scope_hydration::has_hydration_stage(&self.state, channel_id, req_id)
                    {
                        self.scope_hydration.clear(channel_id);
                        crate::scope_hydration::drop_auto_read_emits(out, effect_start, req_id);
                    }
                }
            }

            CorrelationContext::MessageQueryLocal {
                request,
                query_session_epoch,
                query_generation,
                allow_remote_fallback,
                causation_id,
                deferred_send_http,
            } => self.handle_message_query_local_reply(
                *request,
                query_session_epoch,
                query_generation,
                allow_remote_fallback,
                causation_id,
                deferred_send_http,
                now_ms,
                outcome,
                out,
            )?,

            CorrelationContext::MessageQueryRemote {
                request,
                local_rows_desc,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            } => self.handle_message_query_remote_reply(
                *request,
                *local_rows_desc,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
                now_ms,
                outcome,
                out,
            )?,

            CorrelationContext::MessageQueryCache {
                request,
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            } => self.handle_message_query_cache_reply(
                *request,
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
                now_ms,
                outcome,
                out,
            )?,

            CorrelationContext::MessageQueryReadback {
                request,
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            } => self.handle_message_query_readback_reply(
                *request,
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
                now_ms,
                outcome,
                out,
            )?,

            CorrelationContext::ForwardCommittedHttp { read } => {
                self.forward_committed_http(*read, outcome, out)?
            }
            CorrelationContext::ForwardCommittedPersist { read } => {
                self.forward_committed_persist(*read, outcome, out)?
            }
            CorrelationContext::ForwardCommittedReadback { read } => {
                self.forward_committed_readback(*read, outcome, out)?
            }

            CorrelationContext::DialogListQuery { causation_id } => match outcome {
                PortOutcome::Ok(reply) => {
                    out.push(self.emit_dialog_list_result_for_runtime(
                        causation_id.as_deref(),
                        reply.0.as_ref(),
                    ));
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(error = ?e, "dialog list scan failed");
                    out.push(
                        self.emit_dialog_list_result_for_runtime(causation_id.as_deref(), b"[]"),
                    );
                }
            },

            CorrelationContext::ChannelViewSnapshotQuery {
                channel_id,
                causation_id,
                auth_user_id,
                company_id,
            } => {
                let body = match outcome {
                    PortOutcome::Ok(reply)
                        if self.config.auth_user_id == auth_user_id
                            && self.config.company_id == company_id =>
                    {
                        crate::query::channel_view_snapshot::result_body(
                            reply.0.as_ref(),
                            &crate::query::DialogListScope::new(
                                auth_user_id.as_str(),
                                company_id.as_str(),
                            ),
                            channel_id,
                        )
                    }
                    PortOutcome::Ok(_) => {
                        tracing::warn!("channel view snapshot identity changed; returning miss");
                        serde_json::json!({ "snapshot": null })
                    }
                    PortOutcome::Err(error) => {
                        tracing::warn!(error = ?error, "channel view snapshot read failed");
                        serde_json::json!({ "snapshot": null })
                    }
                };
                out.push(crate::query::read_relay::emit_read_body(
                    causation_id.as_deref().unwrap_or_default(),
                    body,
                ));
            }

            // 兼容仍在迁移中的旧两段 dialog/member 绝对投影回读；生产入口已统一走 typed Result。
            CorrelationContext::DialogListMemberSnapshot {
                channel_rows,
                causation_id,
                auth_user_id,
            } => match outcome {
                PortOutcome::Ok(reply) => {
                    if self.config.auth_user_id.as_str() != auth_user_id.as_str() {
                        tracing::warn!(
                            "dialog member readback identity changed; refusing stale snapshot"
                        );
                        emit_dialog_list_failed(out, causation_id)?;
                        return Ok(());
                    }
                    let merged_rows = match crate::event::read::merge_dialog_rows_for_viewer(
                        channel_rows.as_ref(),
                        reply.0.as_ref(),
                        auth_user_id.as_str(),
                    ) {
                        Ok(rows) => rows,
                        Err(error) => {
                            tracing::warn!(error = ?error, "dialog member readback rejected");
                            emit_dialog_list_failed(out, causation_id)?;
                            return Ok(());
                        }
                    };
                    let read_events = crate::event::read::channels_from_dialog_rows(
                        &merged_rows,
                        auth_user_id.as_str(),
                    )?;
                    out.push(
                        crate::event::channel::list(serde_json::json!({
                            "channels": merged_rows,
                            "requestId": causation_id,
                        }))?
                        .into_effect(),
                    );
                    for event in read_events {
                        out.push(event.into_effect());
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(error = ?error, "dialog member scan failed");
                    emit_dialog_list_failed(out, causation_id)?;
                }
            },

            // G13d 本地 topic Scan 只发布一次 parent-scoped typed Result，不广播业务事件。
            CorrelationContext::SubtopicsQuery {
                parent_channel_id,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(reply) => {
                    out.push(self.emit_subtopics_result_for_runtime(
                        causation_id.as_deref(),
                        Some(parent_channel_id.as_str()),
                        reply.0.as_ref(),
                    ));
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(error = ?error, "subtopic scan failed");
                    out.push(self.emit_subtopics_result_for_runtime(
                        causation_id.as_deref(),
                        Some(parent_channel_id.as_str()),
                        b"[]",
                    ));
                }
            },

            CorrelationContext::PinnedProjectionQuery { req_id, channel_id } => {
                let body = match outcome {
                    PortOutcome::Ok(reply) => {
                        crate::query::pinned_projection::result_body(&reply.0)?
                    }
                    PortOutcome::Err(error) => {
                        tracing::warn!(channel_id = channel_id.as_str(), error = ?error, "pinned projection read failed");
                        serde_json::json!({ "cached": false })
                    }
                };
                out.push(crate::read_relay::emit_read_body(req_id.as_str(), body));
            }

            CorrelationContext::OutboundPinnedReply {
                req_id,
                account_id,
                channel_id,
                projection_key,
                epoch,
            } => {
                self.handle_outbound_pinned_reply(
                    req_id,
                    account_id,
                    channel_id,
                    projection_key,
                    epoch,
                    outcome,
                    out,
                )?;
            }

            CorrelationContext::PinnedProjectionPersist { req_id, raw_body } => {
                if let PortOutcome::Err(error) = outcome {
                    tracing::warn!(error = ?error, "pinned projection persist failed; returning authority body without cache");
                }
                out.push(crate::read_relay::emit_read_result(
                    req_id.as_str(),
                    raw_body.as_slice(),
                ));
            }

            // spec06 缺陷A：读族 outbound HTTP 回报 → 透传响应体回灌前端（杀 fire-and-forget 丢响应）。
            // Ok → 剥 ADR-007 信封（base64 body）得裸后端响应体 → emit `im:read:result{req_id, body}`
            // （后端返回什么吐什么，零改业务数据）。信封畸形 → 当作 read error 回灌（不丢 req_id 关联，
            // 前端 reject 而非永久挂起）。Err（网络/HTTP 层）→ emit `im:read:result{req_id, error}`。
            CorrelationContext::ForwardDetail { request, timer } => {
                self.forward_detail_reply(*request, timer, outcome, out);
            }
            CorrelationContext::ReactionUnreadProjection {
                req_id,
                channel_id,
                body,
                expected,
                readback,
            } => {
                self.reaction_unread_projection_reply(
                    req_id, channel_id, body, expected, readback, outcome, now_ms, out,
                )?;
            }
            CorrelationContext::OutboundReadReply {
                req_id,
                command,
                channel_id,
            } => {
                self.handle_outbound_read_reply(
                    &req_id,
                    command.as_str(),
                    channel_id.as_deref(),
                    outcome,
                    now_ms,
                    out,
                )?;
            }

            CorrelationContext::OutboundExactPosts {
                req_id,
                requested_ids,
            } => {
                self.handle_exact_posts_http_reply(&req_id, requested_ids, outcome, out);
            }

            CorrelationContext::ExactPostsPersist {
                req_id,
                accepted_keys,
            } => {
                self.handle_exact_posts_persist_reply(req_id, accepted_keys, outcome, out);
            }

            CorrelationContext::ExactPostsReadback {
                req_id,
                accepted_keys,
                next_index,
                rows,
            } => {
                self.handle_exact_posts_readback_reply(
                    req_id,
                    accepted_keys,
                    next_index,
                    rows,
                    outcome,
                    out,
                );
            }

            CorrelationContext::OutboundInitialWindow {
                req_id,
                post_id,
                page_size,
            } => {
                self.handle_initial_window_http_reply(req_id, post_id, page_size, outcome, out);
            }

            CorrelationContext::InitialWindowPersist {
                req_id,
                post_id,
                accepted_keys,
            } => {
                self.handle_initial_window_persist_reply(
                    req_id,
                    post_id,
                    accepted_keys,
                    outcome,
                    out,
                );
            }

            CorrelationContext::InitialWindowReadback {
                req_id,
                post_id,
                accepted_keys,
                next_index,
                rows,
            } => {
                self.handle_initial_window_readback_reply(
                    req_id,
                    post_id,
                    accepted_keys,
                    next_index,
                    rows,
                    outcome,
                    out,
                );
            }

            CorrelationContext::RecentHistoryRead { command } => {
                crate::recent_history::read_reply(self, command, outcome, out)?;
            }
            CorrelationContext::RecentHistoryWrite { command, result } => {
                crate::recent_history::write_reply(self, command, result, outcome, out)?;
            }
            CorrelationContext::MessageV3DraftReadback { command } => {
                self.handle_draft_readback_reply(command, outcome, out)?;
            }

            CorrelationContext::OutboundLeave { channel_id } => match outcome {
                PortOutcome::Ok(_) if lifecycle_http_succeeded(outcome) => {
                    let auth_user_id = self.config.auth_user_id.as_str().to_string();
                    let persist_corr = self.alloc_corr_internal();
                    self.state.corr_map.insert(
                        persist_corr,
                        CorrelationContext::ChannelLifecyclePersist {
                            channel_id,
                            transition: crate::state::ChannelLifecycleTransition::Left,
                            causation_id: None,
                        },
                    );
                    let mut ops = vec![crate::acl::to_effect_s1::channel_set_cols_op(
                        channel_id,
                        vec![("is_remove", helix_core::effect::SqlValue::Integer(1))],
                    )];
                    // 退出者不会再收到面向在群成员的 roster delta，故 HTTP 成功事务必须自行清掉本人的成员行。
                    if !auth_user_id.is_empty() {
                        if let Some(delete_member) =
                            crate::acl::to_effect_s1::delete_channel_members_op(
                                channel_id,
                                vec![auth_user_id],
                            )
                        {
                            ops.push(delete_member);
                        }
                    }
                    out.push(helix_core::Effect::Persist {
                        corr: persist_corr,
                        ops,
                    });
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        error = ?e,
                        "channel leave http failed; local dialog remains active"
                    );
                }
                PortOutcome::Ok(_) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    "channel leave business response rejected; local dialog remains active"
                ),
            },

            CorrelationContext::ChannelLifecyclePersist {
                channel_id,
                transition,
                causation_id: _,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    // close/leave 的本地字段已经成功落库；立即封住当前内存聚合，
                    // 避免同一连接后续的 increment、PongGap 或 gate 再发补偿。
                    if let Some(channel) = self.state.channels.get_mut(&channel_id) {
                        channel.mark_projection_terminal(channel.cursor.value());
                    }
                    out.push(match transition {
                        crate::state::ChannelLifecycleTransition::Closed { delete_at } => {
                            crate::acl::to_effect_s1::emit_channel_closed(channel_id, delete_at)
                        }
                        crate::state::ChannelLifecycleTransition::Left => {
                            crate::acl::to_effect_s1::emit_channel_left(channel_id)
                        }
                    });
                }
                PortOutcome::Err(error) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?error,
                    "channel lifecycle persist failed; suppressing terminal projections"
                ),
            },

            CorrelationContext::ChannelMemberUpdatePersist {
                channel_id,
                event_seq,
                viewer_rejoined,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    // durable is_remove=0 成功后才解除 viewer-local terminal，失败时继续保持离场态。
                    if viewer_rejoined {
                        if let Some(channel) = self.state.channels.get_mut(&channel_id) {
                            channel.resume_after_rejoin();
                        }
                    }
                    if let Some(seq) = event_seq {
                        self.state
                            .inflight_member_update_seqs
                            .remove(&(channel_id, seq));
                        self.state
                            .committed_member_update_seqs
                            .insert((channel_id, seq));
                    }
                    let corr = self.alloc_corr_internal();
                    self.state.corr_map.insert(
                        corr,
                        CorrelationContext::ChannelMemberUpdateChannelReadback {
                            channel_id,
                            causation_id,
                        },
                    );
                    out.push(helix_core::Effect::Persist {
                        corr,
                        ops: vec![helix_core::effect::StorageOp::Get(
                            helix_core::effect::GetSpec {
                                table: "channel",
                                key_col: "id",
                                key_val: helix_core::effect::SqlValue::Text(
                                    channel_id.as_str().to_string(),
                                ),
                            },
                        )],
                    });
                }
                PortOutcome::Err(error) => {
                    if let Some(seq) = event_seq {
                        self.state
                            .inflight_member_update_seqs
                            .remove(&(channel_id, seq));
                    }
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        error = ?error,
                        "channel member update persist failed; suppressing terminal projections"
                    );
                }
            },

            CorrelationContext::ChannelMemberUpdateChannelReadback {
                channel_id,
                causation_id,
            } => self.handle_channel_member_update_channel_readback_reply(
                channel_id,
                causation_id,
                outcome,
                out,
            )?,
            CorrelationContext::ChannelMemberUpdateReadback {
                channel_id,
                channel,
                causation_id,
            } => self.handle_channel_member_update_readback_reply(
                channel_id,
                channel.map(|value| *value),
                causation_id,
                outcome,
                out,
            )?,

            CorrelationContext::ChannelMemberRoleScan {
                channel_id,
                user_ids,
                role,
            } => self
                .handle_channel_member_role_scan_reply(channel_id, user_ids, role, outcome, out)?,
            CorrelationContext::ChannelMemberRolePersist { channel_id } => {
                self.handle_channel_member_role_persist_reply(channel_id, outcome, out)?;
            }
            CorrelationContext::ChannelMemberRoleChannelReadback { channel_id } => {
                self.handle_channel_member_role_channel_readback_reply(channel_id, outcome, out)?;
            }
            CorrelationContext::ChannelMemberRoleReadback {
                channel_id,
                channel,
            } => {
                self.handle_channel_member_role_readback_reply(
                    channel_id,
                    channel.map(|value| *value),
                    outcome,
                    out,
                )?;
            }

            CorrelationContext::OutboundChannelCreate {
                members,
                request_id,
            } => {
                self.handle_outbound_channel_create_reply(
                    members, request_id, outcome, now_ms, out,
                );
            }
            CorrelationContext::ChannelCreatePersist {
                channel_id,
                channel,
                member_rows,
                causation_id,
            } => {
                self.handle_channel_create_persist_reply(
                    channel_id,
                    *channel,
                    member_rows,
                    causation_id,
                    outcome,
                    now_ms,
                    out,
                )?;
            }

            CorrelationContext::OutboundMakeTopic {
                root_message_id,
                req_id,
                display_name,
            } => {
                self.handle_outbound_make_topic_reply(
                    &root_message_id,
                    &req_id,
                    &display_name,
                    outcome,
                    out,
                );
            }

            CorrelationContext::OutboundMakeTopicPersist { projection } => {
                self.handle_make_topic_persist_reply(projection, outcome, out);
            }

            CorrelationContext::OutboundMakeTopicRootRead { projection } => {
                self.handle_make_topic_thread_readback(projection, outcome, out)?;
            }

            CorrelationContext::OutboundMakeTopicRelationPersist {
                projection,
                parent_channel_id,
                root_row,
            } => {
                self.handle_make_topic_relation_persist_reply(
                    projection,
                    parent_channel_id,
                    root_row,
                    outcome,
                    out,
                )?;
            }

            CorrelationContext::OutboundChannelSettings {
                channel_id,
                causation_id,
            } => {
                self.handle_outbound_channel_settings_reply(channel_id, causation_id, outcome, out);
            }

            CorrelationContext::OutboundChannelSettingsPersist { projection } => {
                self.handle_channel_settings_persist_reply(projection, outcome, now_ms, out)?;
            }

            // UC-4.5：仍保留通用 read result，同时把 HTTP data=*IncrementChannel 送入既有
            // increment 应用规则，相关落 channel/member 后再进入单频道 sync hydration。
            CorrelationContext::OutboundIncrementHydration {
                req_id,
                emit_channel_increment,
                scope_channel,
            } => {
                let effect_start = out.as_slice().len();
                let auto_scope = scope_channel
                    .is_some_and(|channel_id| self.scope_hydration.is_active(channel_id, &req_id));
                // A disconnect or RuntimeIdentity transition invalidates the
                // internal lease.  Late replies must not recreate the old
                // company's durable projection or UI result.
                if scope_channel.is_some() && !auto_scope {
                    return Ok(());
                }
                self.handle_increment_hydration_reply(
                    &req_id,
                    emit_channel_increment,
                    outcome,
                    now_ms,
                    out,
                )?;
                if let Some(channel_id) = scope_channel {
                    if !crate::scope_hydration::has_increment_persist(
                        &self.state,
                        channel_id,
                        &req_id,
                    ) {
                        self.scope_hydration.clear(channel_id);
                        crate::scope_hydration::drop_auto_read_emits(out, effect_start, &req_id);
                    }
                }
            }

            // S7（issue #56）：byIds 成员快照读族回报 → 双 emit。
            // Ok → ① 照旧透传 im:read:result{req_id, body}（UC-6.4 ② 冻结契约面）；
            //       ② 解析 byIds body（map[channelId][]）逐 channel emit render-ready im:channel:members
            //       （壳退纯绑定·载族自愈）。信封畸形 / Err → 仅回灌 read error（不丢 req_id，前端 reject）。
            CorrelationContext::OutboundMembersByIds { req_id } => {
                crate::port_reply_emit::emit_members_by_ids(
                    &req_id,
                    outcome,
                    |channel| self.render_scope.visible(&self.config, channel),
                    out,
                );
            }

            CorrelationContext::OutboundContactCandidates { req_id } => {
                crate::port_reply_emit::emit_contact_candidates(&req_id, outcome, out);
            }

            // G12 回复族只发布完整 thread 领域事件；HTTP/raw reply 不再是消费合同。
            CorrelationContext::OutboundReplies { request } => {
                let projection = crate::port_reply_emit::normalize_replies(
                    &request,
                    outcome,
                    &mut self.state.reply_projection_revisions,
                    &mut self.state.reply_projection_seen_ids,
                );
                if let Some(projection) = projection {
                    let projection_mode = match projection.projection_mode {
                        crate::render_ready_replies::ReplyProjectionMode::Snapshot => "snapshot",
                        crate::render_ready_replies::ReplyProjectionMode::Append => "append",
                    };
                    out.push(
                        crate::event::timeline::thread(serde_json::json!({
                            "requestId": request.req_id,
                            "channelId": projection.channel_id,
                            "rootMessageId": projection.root_message_id,
                            "projectionMode": projection_mode,
                            "revision": projection.revision,
                            "replyIds": projection.reply_ids,
                            "messages": crate::event::timeline::thread_messages(&projection.nodes),
                            "replyCount": projection.reply_count,
                            "hasMore": projection.has_more,
                        }))?
                        .into_effect(),
                    );
                }
            }

            CorrelationContext::OutboundLocatePost { req_id, post_id } => {
                let Some(expected_channel_id) = self
                    .state
                    .timeline_state
                    .located_target_channel(&post_id)
                    .map(str::to_string)
                else {
                    tracing::warn!(
                        req_id,
                        post_id,
                        "locate target is absent from attached timeline"
                    );
                    return Ok(());
                };
                let PortOutcome::Ok(reply) = outcome else {
                    tracing::warn!(req_id, post_id, "locate HTTP authority failed");
                    return Ok(());
                };
                let raw_body = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref())
                    .map_err(|error| {
                        ImError::Parse(format!("locate response envelope: {error}"))
                    })?;
                let rows = crate::render_ready_locate::normalize_located_window(
                    raw_body.as_ref(),
                    &expected_channel_id,
                    &post_id,
                    self.config.auth_user_id.as_str(),
                )
                .map_err(|error| ImError::Parse(format!("locate authority: {error}")))?;
                out.push(
                    crate::event::timeline::located(serde_json::json!({
                        "requestId": req_id,
                        "channelId": expected_channel_id,
                        "windowToken": format!("locate-{post_id}"),
                        "state": "ready",
                        "messages": rows,
                        "targetMessageId": post_id,
                        "revealPostId": post_id,
                    }))?
                    .into_effect(),
                );
            }

            CorrelationContext::OutboundCreatePosts { req_id } => {
                self.forward_acceptance(&req_id, outcome, out)?
            }

            CorrelationContext::TopicActivityRootRead { projection } => {
                self.handle_topic_activity_root_read(projection, outcome, out)?;
            }

            CorrelationContext::TopicActivityPersist {
                parent_channel_id,
                parent_topic_message_count,
            } => self.handle_topic_activity_persist(
                parent_channel_id,
                parent_topic_message_count,
                outcome,
                out,
            )?,

            CorrelationContext::OutboundPostReaders { req_id } => {
                crate::port_reply_emit::emit_post_readers(&req_id, outcome, out);
            }

            CorrelationContext::PostReadPersist {
                channel_id,
                message_id,
                receipt_revision,
                read_bits,
                terminal_event,
            } => self.handle_message_v3_post_read_persist_reply(
                channel_id,
                message_id,
                receipt_revision,
                read_bits,
                terminal_event,
                outcome,
                out,
            ),

            CorrelationContext::PostsUpdateAtomic {
                channel_id,
                target_seq,
                pending_domain_events,
                has_category_posts,
                refresh_timeline,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    let committed = self
                        .state
                        .channels
                        .get_mut(&channel_id)
                        .map(|channel| {
                            channel.commit_contiguous_range_after_atomic(target_seq, out)
                        })
                        .transpose()?
                        .unwrap_or(false);
                    if !committed {
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            target_seq = target_seq.0,
                            "posts_update atomic reply no longer matches contiguous cursor"
                        );
                    } else {
                        self.state.invalidate_recent_message_coverage(channel_id);
                        self.release_post_events(pending_domain_events, has_category_posts, out)?;
                        if refresh_timeline {
                            self.refresh_attached_latest_timeline(channel_id, None, out)?;
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        target_seq = target_seq.0,
                        error = ?error,
                        "posts_update atomic persist failed; cursor and projections remain unchanged"
                    );
                }
            },

            CorrelationContext::PostUpdateAtomic {
                event,
                pending_domain_event,
                has_category_posts,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    let channel_id = event.channel_id;
                    let target_seq = event.seq;
                    if has_category_posts {
                        let Some(channel) = self.state.channels.get_mut(&channel_id) else {
                            return Ok(());
                        };
                        if channel.is_terminal()
                            || target_seq.0 != channel.cursor.value().0.saturating_add(1)
                        {
                            return Ok(());
                        }
                        let next = channel.commit_message_v3_post(target_seq);
                        self.release_post_events(vec![pending_domain_event], true, out)?;
                        if let Some(event) = next {
                            self.queue_next_message_v3_event(event, out)?;
                        }
                        return Ok(());
                    }
                    let (committed, next) = self
                        .state
                        .channels
                        .get_mut(&channel_id)
                        .map(|channel| {
                            let expected =
                                crate::state::Seq(channel.cursor.value().0.saturating_add(1));
                            if target_seq != expected {
                                return (false, None);
                            }
                            let next = channel.commit_message_v3_post(target_seq);
                            (channel.cursor.value() == target_seq, next)
                        })
                        .unwrap_or((false, None));
                    if committed {
                        self.state.invalidate_recent_message_coverage(channel_id);
                        out.push(helix_core::Effect::Emit {
                            event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(
                                pending_domain_event,
                            )),
                        });
                        self.refresh_attached_latest_timeline(channel_id, None, out)?;
                        if let Some(next_event) = next {
                            self.queue_next_message_v3_event(next_event, out)?;
                        }
                    } else {
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            target_seq = target_seq.0,
                            "post_update atomic reply no longer matches contiguous cursor"
                        );
                    }
                }
                PortOutcome::Err(error) => {
                    let channel_id = event.channel_id;
                    let target_seq = event.seq;
                    if let Some(channel) = self.state.channels.get_mut(&channel_id) {
                        channel.restore_message_v3_post(*event, out);
                    }
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        target_seq = target_seq.0,
                        error = ?error,
                        "post_update atomic persist failed; cursor and projection remain unchanged"
                    );
                }
            },

            // UC-10 通知/待办：global increment-end 触发的 `queryTodoList` HTTP 回报 → 装配
            // `im:todo:updated{items}`（解 data.channels[].posts[]，真源 channel.rs:252-296）。
            // Ok → 剥 ADR-007 信封得裸响应体 → emit_todo_updated（status≠SUCCESS/结构缺失 → 空
            // items 仍 emit，前端无害刷新）。信封畸形 / Err → 不 emit（待办非关键路径，避免误清前端
            // 已渲染待办；下次 hello 重连会重拉，边界零信任不 panic）。
            CorrelationContext::TodoQuery => self.handle_todo_query_reply(outcome, out),

            // G10a 上拉翻页：HTTP 回报只推进分页态；Done 必须进入 message persist barrier。
            CorrelationContext::LoadOlderContext { state } => {
                self.handle_load_older_context_reply(state, outcome, now_ms, out)?;
            }

            // G10a message write barrier：只有成功回执才允许扫描完整 bounded window。
            CorrelationContext::LoadOlderCache { state } => match outcome {
                PortOutcome::Ok(_) => self.schedule_load_older_readback(state, out)?,
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "load_older_context message persist failed"
                    );
                    self.emit_load_older_failed_event(&state, now_ms, out)?;
                }
            },

            // G10a durable readback：完整 storage rows 是唯一成功事件输入。
            CorrelationContext::LoadOlderReadback { state } => {
                self.handle_load_older_readback_reply(state, outcome, now_ms, out)?;
            }

            // G10b/G11a HTTP 回包必须先进入 message upsert 屏障。
            CorrelationContext::TimelineNavigationHttp { state } => {
                self.handle_timeline_navigation_http_reply(state, outcome, now_ms, out)?;
            }

            // matching PersistOk 后才允许启动页内 readback。
            CorrelationContext::TimelineNavigationCache { state } => match outcome {
                PortOutcome::Ok(_) => {
                    self.schedule_timeline_navigation_readback(state, out)?;
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline navigation persist failed"
                    );
                    self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                }
            },

            // readback 是成功事件的最后一道本地事实屏障。
            CorrelationContext::TimelineNavigationReadback { state } => {
                self.handle_timeline_navigation_readback_reply(state, outcome, now_ms, out)?;
            }

            // older/newer 页的每个 authority identity 都必须由 durable storage 逐项确认。
            CorrelationContext::TimelineNavigationExactReadback {
                state,
                accepted_keys,
                next_index,
                rows,
            } => {
                self.handle_timeline_navigation_exact_readback_reply(
                    state,
                    accepted_keys,
                    next_index,
                    rows,
                    outcome,
                    now_ms,
                    out,
                )?;
            }
        }

        Ok(())
    }
}

/// 接受空测试回包或显式 SUCCESS 信封，拒绝 transport-ok/business-failed 假终态。
fn lifecycle_http_succeeded(outcome: &helix_core::tick::PortOutcome) -> bool {
    let helix_core::tick::PortOutcome::Ok(reply) = outcome else {
        return false;
    };
    if reply.0.is_empty() {
        return true;
    }
    let Ok(raw) = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()) else {
        return false;
    };
    let Ok(response) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return false;
    };
    response
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("SUCCESS"))
}

#[cfg(test)]
mod phase2_contract_tests {
    use super::*;
    use bytes::Bytes;
    use helix_core::effect::Effect;
    use helix_core::tick::{PortOutcome, ReplyBytes};
    use helix_core::EffectSink;

    const CHANNEL_ID: &str = "chfixx0000000000000000002b";

    fn member_projection(unread_count: i64) -> crate::channel_update::MemberChannelUpdate {
        let channel_id = crate::state::ChannelId::from_str(CHANNEL_ID).unwrap();
        crate::channel_update::member_channel_from_update_channel(
            &serde_json::json!({
                "channelId": CHANNEL_ID,
                "userId": "viewer-29",
                "projectionRevision": 3,
                "effectId": "effect-3",
                "unreadCount": unread_count,
                "lastReadSeq": 7
            }),
            channel_id,
            "viewer-29",
            1,
        )
        .unwrap()
        .1
    }

    fn member_projection_reply(unread_count: i64) -> PortOutcome {
        PortOutcome::Ok(ReplyBytes(Bytes::from(
            serde_json::json!([{
                "channel_id": CHANNEL_ID,
                "user_id": "viewer-29",
                "projection_revision": 3,
                "effect_id": "effect-3",
                "unread_count": unread_count,
                "last_read_seq": 7
            }])
            .to_string(),
        )))
    }

    #[test]
    fn member_projection_write_ack_schedules_readback_before_emit() {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "viewer-29".to_string();
        let mut module = ImModule::new(config);
        let channel_id = crate::state::ChannelId::from_str(CHANNEL_ID).unwrap();
        let corr = module.alloc_corr();
        module.state.corr_map.insert(
            corr,
            CorrelationContext::MemberProjectionPersist {
                channel_id,
                expected_revision: 3,
                expected_effect_id: "effect-3".to_string(),
                expected_projection: Box::new(member_projection(1)),
            },
        );
        let mut out = EffectSink::new();

        module
            .handle_port_reply(corr, &PortOutcome::Ok(ReplyBytes::default()), 1, &mut out)
            .unwrap();

        let [Effect::Persist {
            corr: read_corr, ..
        }] = out.as_slice()
        else {
            panic!("write ack must schedule exactly one readback");
        };
        assert!(matches!(
            module.state.corr_map.get(read_corr),
            Some(CorrelationContext::MemberProjectionReadback { .. })
        ));
    }

    #[test]
    fn member_projection_emits_only_matching_durable_snapshot() {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "viewer-29".to_string();
        let mut module = ImModule::new(config);
        let channel_id = crate::state::ChannelId::from_str(CHANNEL_ID).unwrap();
        let corr = module.alloc_corr();
        module.state.corr_map.insert(
            corr,
            CorrelationContext::MemberProjectionReadback {
                channel_id,
                expected_revision: 3,
                expected_effect_id: "effect-3".to_string(),
                expected_projection: Box::new(member_projection(1)),
            },
        );
        let mut out = EffectSink::new();

        module
            .handle_port_reply(corr, &member_projection_reply(1), 1, &mut out)
            .unwrap();

        assert_eq!(
            out.as_slice()
                .iter()
                .filter(|effect| matches!(effect, Effect::Emit { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn member_projection_suppresses_same_revision_payload_conflict() {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "viewer-29".to_string();
        let mut module = ImModule::new(config);
        let channel_id = crate::state::ChannelId::from_str(CHANNEL_ID).unwrap();
        let corr = module.alloc_corr();
        module.state.corr_map.insert(
            corr,
            CorrelationContext::MemberProjectionReadback {
                channel_id,
                expected_revision: 3,
                expected_effect_id: "effect-3".to_string(),
                expected_projection: Box::new(member_projection(2)),
            },
        );
        let mut out = EffectSink::new();

        module
            .handle_port_reply(corr, &member_projection_reply(1), 1, &mut out)
            .unwrap();

        assert!(out.as_slice().is_empty());
    }

    fn canonical_stream_event(event_type: u8) -> crate::sync_session::EventEnvelope {
        crate::ws::handlers::channel_stream_event::parse_stream_event(
            &serde_json::json!({
                "channelId": CHANNEL_ID,
                "streamSeq": 1,
                "eventId": "event-1",
                "effectId": "effect-1",
                "eventType": event_type,
                "actorId": "author-1",
                "msgId": "message-1",
                "redacted": false,
                "payload": {"id": "message-1", "channelId": CHANNEL_ID, "message": "hello"}
            }),
            "viewer-29",
        )
        .unwrap()
        .unwrap()
    }

    /// 构造与生产路径一致的 canonical persist correlation，先占用连续 eventSeq 槽位。
    fn admitted_canonical_stream_event(
        module: &mut ImModule,
        channel_id: crate::state::ChannelId,
        event_type: u8,
    ) -> crate::sync_session::EventEnvelope {
        let mut admission = EffectSink::new();
        module
            .state
            .channels
            .get_mut(&channel_id)
            .expect("test channel must be registered")
            .admit_message_v3_post(canonical_stream_event(event_type), &mut admission)
            .expect("canonical event admission must not fail")
            .expect("contiguous canonical event must claim the pending slot")
    }

    #[test]
    fn canonical_stream_ack_controls_cursor_emit_and_retry() {
        let channel_id = crate::state::ChannelId::from_str(CHANNEL_ID).unwrap();
        let mut success = ImModule::new(crate::module::ImConfig::default());
        success.register_channel(channel_id, 0);
        let corr = success.alloc_corr();
        let event = admitted_canonical_stream_event(&mut success, channel_id, 1);
        success.state.corr_map.insert(
            corr,
            CorrelationContext::CanonicalStreamPersist {
                event: Box::new(event),
            },
        );
        let mut out = EffectSink::new();
        success
            .handle_port_reply(corr, &PortOutcome::Ok(ReplyBytes::default()), 1, &mut out)
            .unwrap();
        assert_eq!(success.cursor_for(channel_id), Some(1));
        assert_eq!(
            out.as_slice()
                .iter()
                .filter(|effect| matches!(effect, Effect::Emit { .. }))
                .count(),
            1
        );

        let mut failed = ImModule::new(crate::module::ImConfig::default());
        failed.register_channel(channel_id, 0);
        let corr = failed.alloc_corr();
        let event = admitted_canonical_stream_event(&mut failed, channel_id, 1);
        failed.state.corr_map.insert(
            corr,
            CorrelationContext::CanonicalStreamPersist {
                event: Box::new(event),
            },
        );
        let mut out = EffectSink::new();
        failed
            .handle_port_reply(
                corr,
                &PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
                1,
                &mut out,
            )
            .unwrap();
        assert_eq!(failed.cursor_for(channel_id), Some(0));
        assert!(failed.state.channels[&channel_id]
            .buffer
            .contains_key(&crate::state::Seq(1)));
        assert!(!out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })));
    }

    #[test]
    fn canonical_terminal_ack_commits_tombstone_and_emits_closed() {
        let channel_id = crate::state::ChannelId::from_str(CHANNEL_ID).unwrap();
        let mut module = ImModule::new(crate::module::ImConfig::default());
        module.register_channel(channel_id, 0);
        let corr = module.alloc_corr();
        module.state.corr_map.insert(
            corr,
            CorrelationContext::CanonicalStreamPersist {
                event: Box::new(canonical_stream_event(7)),
            },
        );
        let mut out = EffectSink::new();

        module
            .handle_port_reply(corr, &PortOutcome::Ok(ReplyBytes::default()), 1, &mut out)
            .unwrap();

        assert_eq!(module.cursor_for(channel_id), Some(1));
        assert!(module.state.channels[&channel_id].is_terminal());
        assert!(out.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Emit { event }
                if String::from_utf8_lossy(event.0.as_ref()).contains("im:channel:closed")
        )));
    }

    /// Persist read-back 只在匹配 correlation 下释放一次成功事件。
    #[test]
    fn timeline_readback_requires_matching_rows() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id =
            crate::state::ChannelId::from_str(CHANNEL_ID).expect("test channel id is valid");
        let page = crate::timeline_state::WindowPage {
            window_token: "latest".to_string(),
            ..Default::default()
        };
        let mut state = crate::timeline_navigation::TimelineNavigationState::older(
            channel_id,
            page,
            20,
            Some("phase2-readback-1".to_string()),
            "anchor-1".to_string(),
            crate::timeline_state::TimelineEntityKey {
                create_at: 1,
                temporary_id: "tmp-1".to_string(),
            },
        );
        state.replace_rows(vec![serde_json::json!({
            "id": "target-1",
            "channelId": CHANNEL_ID,
            "createAt": 2,
            "temporaryId": "tmp-2"
        })]);
        let corr = module.alloc_corr();
        module.state.corr_map.insert(
            corr,
            CorrelationContext::TimelineNavigationReadback {
                state: Box::new(state),
            },
        );
        let outcome = PortOutcome::Ok(ReplyBytes(Bytes::from_static(br#"[{"id":"target-1"}]"#)));
        let mut out = EffectSink::new();

        module
            .handle_port_reply(corr, &outcome, 1, &mut out)
            .expect("matching read-back is handled");
        assert_eq!(
            out.as_slice()
                .iter()
                .filter(|effect| matches!(effect, Effect::Emit { .. }))
                .count(),
            1
        );
    }

    /// 同一 correlation 的迟到 terminal 必须被单表 remove 闸门丢弃。
    #[test]
    fn duplicate_terminal_correlation_is_ignored() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let corr = module.alloc_corr();
        let event = crate::event::MessageV3Event::new(
            "im:phase2:test",
            serde_json::json!({ "reqId": "phase2-terminal-1" }),
        )
        .expect("test event is valid")
        .into_bytes();
        module.state.corr_map.insert(
            corr,
            CorrelationContext::MessageV3Commit {
                terminal_events: vec![event],
            },
        );
        let outcome = PortOutcome::Ok(ReplyBytes::default());
        let mut out = EffectSink::new();

        module
            .handle_port_reply(corr, &outcome, 1, &mut out)
            .expect("first terminal is handled");
        assert_eq!(
            out.as_slice()
                .iter()
                .filter(|effect| matches!(effect, Effect::Emit { .. }))
                .count(),
            1
        );
        out.clear();
        module
            .handle_port_reply(corr, &outcome, 2, &mut out)
            .expect("late terminal is ignored");
        assert!(out.as_slice().is_empty());
    }
}
