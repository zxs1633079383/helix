use crate::error::ImError;
use crate::module::ImModule;
use crate::state::CorrelationContext;
use helix_core::EffectSink;

impl ImModule {
    /// 处理 PortReply（Persist/Http 的异步结果回报）。
    ///
    /// 统一 `corr_map` 单表 + `match CorrelationContext` 分发（类别互斥靠 corr 全局唯一，见 state.rs
    /// `CorrelationContext` 不变量）：`remove(corr)` 取上下文 → `ChannelPersist` 委托
    /// `channel.on_persist_ok` 推 cursor / `OptimisticSend` 找 PendingSend / `ScanCursors`（E4）解析
    /// on_start Scan rows（channel_id TEXT + last_event_seq INTEGER）→ register_channel + proactive sync。
    pub(crate) fn handle_port_reply(
        &mut self,
        corr: helix_core::Correlation,
        outcome: &helix_core::tick::PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

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
            CorrelationContext::MessageV3PostPersist {
                event,
                received_data,
            } => {
                self.handle_message_v3_post_persist_reply(*event, *received_data, outcome, out)?;
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
            CorrelationContext::MessageV3RevokeChannelPersist { channel_id } => {
                self.handle_message_v3_revoke_channel_persist_reply(channel_id, outcome, out);
            }
            CorrelationContext::MessageV3RevokeChannelReadback => {
                self.handle_message_v3_revoke_channel_readback_reply(outcome, out)?;
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
                    // 降级：scan 失败（包括首次启动表不存在 / no such table）不崩，空 channel 集仍可用
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?e,
                        "scan channel_event_cursor failed (likely first run, no cursors yet), starting with empty channel set"
                    );
                }
            },

            // 路由：channel sync Persist 回报（cursor 推进 + E3 续拉）
            CorrelationContext::ChannelPersist {
                channel_id,
                trigger,
                wants_continuation,
                dialog_patches,
                pending_domain_events,
            } => self.handle_channel_persist_reply(
                corr,
                channel_id,
                trigger,
                wants_continuation,
                dialog_patches,
                pending_domain_events,
                outcome,
                out,
            )?,

            CorrelationContext::ChannelTerminalPersist {
                channel_id,
                terminal_seq,
                trigger,
                pending_domain_events,
            } => self.handle_channel_terminal_persist_reply(
                corr,
                channel_id,
                terminal_seq,
                trigger,
                pending_domain_events,
                outcome,
                out,
            )?,

            CorrelationContext::IncrementBatchPersist { projections } => {
                self.handle_increment_batch_persist_reply(projections, outcome, out)?
            }

            CorrelationContext::IncrementHydrationPersist {
                channel_id,
                need_sync,
                raw_increment,
            } => self.handle_increment_hydration_persist(
                channel_id,
                need_sync,
                raw_increment,
                outcome,
                out,
            )?,

            CorrelationContext::DialogPatchByPost { pending } => match outcome {
                PortOutcome::Ok(reply) => {
                    if let Some(data) =
                        pending.frame_patch_data_from_channel_reply(reply.0.as_ref())
                    {
                        let terminal_causation = (pending.source == "sync_events")
                            .then(|| pending.causation_id.clone())
                            .flatten();
                        let mut frame =
                            crate::dialog_frame::patch_from_dialog_projection_with_causation(
                                &mut self.state.dialog_list_frame,
                                &data,
                                terminal_causation.clone(),
                            );
                        if frame.is_none() {
                            if let (Some(scope), Some(causation_id)) = (
                                self.state.dialog_list_frame.scope().cloned(),
                                terminal_causation,
                            ) {
                                frame = self
                                    .state
                                    .dialog_list_frame
                                    .authority_refresh_with_causation(scope, causation_id);
                            }
                        }
                        if let Some(frame) = frame {
                            out.push(frame);
                        }
                        self.schedule_topic_activity_projection(&pending, reply.0.as_ref(), out);
                    } else {
                        tracing::warn!(
                            channel_id = pending.channel_id.as_str(),
                            corr = corr.raw(),
                            "dialog patch channel row readback empty; skip emit to avoid delta-as-total"
                        );
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = pending.channel_id.as_str(),
                        corr = corr.raw(),
                        error = ?e,
                        "dialog patch channel write/read failed"
                    );
                }
            },

            CorrelationContext::UpdateChannelDialogPersist {
                scope,
                channel_id,
                channel,
                member_dialog,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(scope) = scope {
                        if self.state.dialog_list_frame.scope() != Some(&scope) {
                            tracing::debug!(
                                corr = corr.raw(),
                                "update_channel PersistOk belongs to a detached dialog scope"
                            );
                        } else {
                            let frame = match member_dialog.as_deref() {
                                Some(dialog) => self
                                    .state
                                    .dialog_list_frame
                                    .patch_from_channel_and_dialog_projection(
                                        scope,
                                        channel.as_ref(),
                                        dialog,
                                    ),
                                None => self
                                    .state
                                    .dialog_list_frame
                                    .patch_from_channel_update(scope, channel.as_ref()),
                            };
                            if let Some(frame) = frame {
                                out.push(frame);
                            }
                        }
                    }
                    self.project_channel_settings_value_frame(
                        channel_id,
                        channel.as_ref(),
                        causation_id,
                        now_ms,
                        out,
                    )?;
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?error,
                        "update_channel persistence failed; suppress paired dialog/unread Frame"
                    );
                }
            },

            CorrelationContext::TooLongReload {
                channel_id,
                reset_to,
            } => match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_too_long_reload_reply(channel_id, reset_to, reply, out)?;
                }
                PortOutcome::Err(e) => {
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
                dialog_patches,
            } => match outcome {
                PortOutcome::Ok(reply) => {
                    for pending in dialog_patches {
                        if let Some(data) =
                            pending.frame_patch_data_from_channel_reply(reply.0.as_ref())
                        {
                            if let Some(frame) = crate::dialog_frame::patch_from_dialog_projection(
                                &mut self.state.dialog_list_frame,
                                &data,
                            ) {
                                out.push(frame);
                            }
                        }
                    }
                    self.finish_increment_hydration(channel_id, out)?;
                }
                PortOutcome::Err(e) => {
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

            // F1：posts/create HTTP 200 兜底对账（真 Go self-omit echo，见 `send_reconcile`）。
            CorrelationContext::OutboundSendHttp {
                channel_id,
                temporary_id,
            } => {
                self.handle_outbound_send_http_reply(channel_id, temporary_id, outcome, out)?;
            }
            CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                window_token,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => self.refresh_attached_timeline(
                    channel_id,
                    window_token.as_deref().unwrap_or("latest"),
                    causation_id,
                    out,
                )?,
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
                if matches!(outcome, PortOutcome::Err(_))
                    && request_id.as_ref().is_some_and(|request_id| {
                        self.state.pending_schedule_requests.get(&channel_id) == Some(request_id)
                    })
                {
                    self.state.pending_schedule_requests.remove(&channel_id);
                }
            }
            CorrelationContext::OutboundScheduleCancel {
                channel_id,
                request_id,
            } => {
                if matches!(outcome, PortOutcome::Err(_))
                    && request_id.as_ref().is_some_and(|request_id| {
                        self.state.pending_schedule_cancel_requests.get(&channel_id)
                            == Some(request_id)
                    })
                {
                    self.state
                        .pending_schedule_cancel_requests
                        .remove(&channel_id);
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
            } => {
                if self
                    .state
                    .inflight_schedule_revisions
                    .get(&channel_id)
                    .is_some_and(|inflight| *inflight == revision)
                {
                    self.state.inflight_schedule_revisions.remove(&channel_id);
                }
                match outcome {
                    PortOutcome::Ok(_) => {
                        self.state
                            .committed_schedule_revisions
                            .insert(channel_id, revision);
                        if causation_id.as_ref().is_some_and(|request_id| {
                            self.state.pending_schedule_cancel_requests.get(&channel_id)
                                == Some(request_id)
                        }) {
                            self.state
                                .pending_schedule_cancel_requests
                                .remove(&channel_id);
                        }
                        self.refresh_attached_channel_summary_with_causation(
                            channel_id,
                            causation_id,
                            out,
                        )?;
                    }
                    PortOutcome::Err(error) => tracing::warn!(
                        channel_id = channel_id.as_str(),
                        revision,
                        error = ?error,
                        "schedule cancel authority persist failed; suppressing channel summary Frame"
                    ),
                }
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
                self.handle_sync_pull_reply(corr, channel_id, trigger, outcome, out)?;
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
                merged_rows_desc,
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            } => self.handle_message_query_cache_reply(
                *request,
                *merged_rows_desc,
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
                now_ms,
                outcome,
                out,
            )?,

            CorrelationContext::SurfaceStorageSnapshot {
                request,
                refresh_existing,
            } => {
                self.handle_surface_storage_snapshot(
                    request,
                    refresh_existing,
                    outcome,
                    now_ms,
                    out,
                )?;
            }

            CorrelationContext::SurfaceThreadSnapshot {
                request,
                expected_revision,
            } => {
                self.handle_surface_thread_snapshot(request, expected_revision, outcome, out)?;
            }

            CorrelationContext::SurfaceReceiptSnapshot { request } => {
                self.handle_surface_receipt_snapshot(request, outcome, out)?;
            }

            CorrelationContext::ForwardSourceLookup { assembly } => {
                if let Err(error) =
                    crate::forward::handle_lookup_reply(self, *assembly, outcome, out)
                {
                    tracing::warn!(error = ?error, "forward source assembly failed");
                }
            }

            CorrelationContext::ForwardSourceChannelLookup { assembly } => {
                if let Err(error) =
                    crate::forward::handle_source_channel_reply(self, *assembly, outcome, out)
                {
                    tracing::warn!(error = ?error, "forward source channel assembly failed");
                }
            }

            // 左侧会话列表 Scan 回报首先发布完整 render-ready frame；Err 同样发布空 snapshot，
            // 使 frame 客户端能原子结束 loading，而不是用 raw channel 行补算 UI。
            CorrelationContext::DialogListQuery { causation_id } => match outcome {
                PortOutcome::Ok(reply) => {
                    let surface_epoch = self.action_surface_epoch()?;
                    self.state
                        .dialog_list_frame
                        .set_surface_epoch(surface_epoch);
                    let scope = crate::dialog_frame::DialogListScope::new(
                        self.config.auth_user_id.as_str(),
                        self.config.company_id.as_str(),
                    );
                    let terminal_channel_ids = self
                        .state
                        .channels
                        .iter()
                        .filter_map(|(channel_id, channel)| {
                            channel
                                .is_terminal()
                                .then(|| channel_id.as_str().to_string())
                        })
                        .collect();
                    if let Some(frame) =
                        crate::query::emit_dialog_list_frame_result_with_terminal_channels(
                            reply.0.as_ref(),
                            scope,
                            &mut self.state.dialog_list_frame,
                            &terminal_channel_ids,
                            causation_id.clone(),
                        )
                    {
                        out.push(frame);
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(error = ?e, "dialog list scan failed");
                    let surface_epoch = self.action_surface_epoch()?;
                    self.state
                        .dialog_list_frame
                        .set_surface_epoch(surface_epoch);
                    let scope = crate::dialog_frame::DialogListScope::new(
                        self.config.auth_user_id.as_str(),
                        self.config.company_id.as_str(),
                    );
                    let terminal_channel_ids = self
                        .state
                        .channels
                        .iter()
                        .filter_map(|(channel_id, channel)| {
                            channel
                                .is_terminal()
                                .then(|| channel_id.as_str().to_string())
                        })
                        .collect();
                    if let Some(frame) =
                        crate::query::emit_dialog_list_frame_result_with_terminal_channels(
                            b"[]",
                            scope,
                            &mut self.state.dialog_list_frame,
                            &terminal_channel_ids,
                            causation_id,
                        )
                    {
                        out.push(frame);
                    }
                }
            },

            // spec06 缺陷A：读族 outbound HTTP 回报 → 透传响应体回灌前端（杀 fire-and-forget 丢响应）。
            // Ok → 剥 ADR-007 信封（base64 body）得裸后端响应体 → emit `im:read:result{req_id, body}`
            // （后端返回什么吐什么，零改业务数据）。信封畸形 → 当作 read error 回灌（不丢 req_id 关联，
            // 前端 reject 而非永久挂起）。Err（网络/HTTP 层）→ emit `im:read:result{req_id, error}`。
            CorrelationContext::OutboundReadReply { req_id } => {
                self.handle_outbound_read_reply(&req_id, outcome, out);
            }

            CorrelationContext::OutboundLeave { channel_id } => match outcome {
                PortOutcome::Ok(_) if lifecycle_http_succeeded(outcome) => {
                    let persist_corr = self.alloc_corr_internal();
                    self.state.corr_map.insert(
                        persist_corr,
                        CorrelationContext::ChannelLifecyclePersist {
                            channel_id,
                            transition: crate::state::ChannelLifecycleTransition::Left,
                            causation_id: None,
                        },
                    );
                    out.push(helix_core::Effect::Persist {
                        corr: persist_corr,
                        ops: vec![crate::acl::to_effect_s1::channel_set_cols_op(
                            channel_id,
                            vec![("is_remove", helix_core::effect::SqlValue::Integer(1))],
                        )],
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
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    let (state, reason) = match transition {
                        crate::state::ChannelLifecycleTransition::Closed { .. } => {
                            ("closed", Some("channelClosed"))
                        }
                        crate::state::ChannelLifecycleTransition::Left => {
                            ("left", Some("memberLeft"))
                        }
                    };
                    self.project_channel_lifecycle_frame(
                        channel_id,
                        state,
                        reason,
                        causation_id,
                        out,
                    )?;
                    if let Some(scope) = self.state.dialog_list_frame.scope().cloned() {
                        let frame = match transition {
                            crate::state::ChannelLifecycleTransition::Closed { .. } => self
                                .state
                                .dialog_list_frame
                                .patch_closed(scope, channel_id.as_str()),
                            crate::state::ChannelLifecycleTransition::Left => {
                                let patch = serde_json::json!({
                                    "channelId": channel_id.as_str(),
                                    "left": true,
                                });
                                self.state
                                    .dialog_list_frame
                                    .patch_from_channel_update(scope, &patch)
                            }
                        };
                        if let Some(frame) = frame {
                            out.push(frame);
                        }
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
                raw_channel,
                members,
                leaves,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(seq) = event_seq {
                        self.state
                            .inflight_member_update_seqs
                            .remove(&(channel_id, seq));
                        self.state
                            .committed_member_update_seqs
                            .insert((channel_id, seq));
                    }
                    out.push(crate::acl::to_effect_s1::emit_channel_member_updated(
                        channel_id,
                        &raw_channel,
                    ));
                    out.push(crate::render_ready_members::emit_channel_members(
                        channel_id.as_str(),
                        members,
                        leaves,
                        crate::render_ready_members::MemberProjectionMode::Incremental,
                    ));
                    self.refresh_attached_member_rosters_with_causation(
                        channel_id,
                        causation_id,
                        out,
                    )?;
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

            CorrelationContext::OutboundMakeTopicFrameRead { projection } => {
                self.handle_make_topic_thread_readback(projection, outcome, out)?;
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
            CorrelationContext::OutboundIncrementHydration { req_id } => {
                self.handle_increment_hydration_reply(&req_id, outcome, now_ms, out)?;
            }

            // S7（issue #56）：byIds 成员快照读族回报 → 双 emit。
            // Ok → ① 照旧透传 im:read:result{req_id, body}（UC-6.4 ② 冻结契约面）；
            //       ② 解析 byIds body（map[channelId][]）逐 channel emit render-ready im:channel:members
            //       （壳退纯绑定·载族自愈）。信封畸形 / Err → 仅回灌 read error（不丢 req_id，前端 reject）。
            CorrelationContext::OutboundMembersByIds { req_id } => {
                crate::port_reply_emit::emit_members_by_ids(&req_id, outcome, out);
            }

            CorrelationContext::OutboundContactCandidates { req_id } => {
                crate::port_reply_emit::emit_contact_candidates(&req_id, outcome, out);
            }

            // G12 回复族只发布完整 `message.thread-detail` Frame；HTTP/raw reply 不再是渲染合同。
            CorrelationContext::OutboundReplies { request } => {
                let projection = crate::port_reply_emit::normalize_replies(
                    &request,
                    outcome,
                    &mut self.state.reply_projection_revisions,
                    &mut self.state.reply_projection_seen_ids,
                );
                if let Some(projection) = projection {
                    if let Err(error) =
                        self.project_reply_thread_frame(&request.req_id, projection, out)
                    {
                        tracing::warn!(
                            req_id = request.req_id,
                            error = ?error,
                            "reply authority did not produce a valid thread Frame"
                        );
                    }
                }
            }

            CorrelationContext::OutboundLocatePost { req_id, post_id } => {
                let Some(expected_channel_id) = self
                    .state
                    .timeline_projector
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
                crate::render_ready_locate::normalize_located_window(
                    raw_body.as_ref(),
                    &expected_channel_id,
                    &post_id,
                    self.config.auth_user_id.as_str(),
                )
                .map_err(|error| ImError::Parse(format!("locate authority: {error}")))?;
                if let Some(mut mutation) = self
                    .state
                    .timeline_projector
                    .patch_located_target(&post_id, None)
                    .map_err(|error| {
                        ImError::Parse(format!("locate timeline projection: {error}"))
                    })?
                {
                    mutation.causation_id = Some(req_id.clone());
                    out.push(self.build_registered_timeline_frame_with_directives(
                        format!("locate-{req_id}"),
                        vec![mutation],
                        vec![crate::render_ready_locate::reveal_directive(format!(
                            "reveal-{req_id}"
                        ))],
                        now_ms,
                    )?);
                    self.state
                        .action_registry
                        .complete_after_terminal_frame(&req_id);
                }
            }

            CorrelationContext::OutboundCreatePosts { req_id } => {
                if let PortOutcome::Ok(reply) = outcome {
                    if let Ok(raw) = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()) {
                        if let Ok(body) = serde_json::from_slice::<serde_json::Value>(&raw) {
                            self.state
                                .pending_forward_deliveries
                                .retain_http_accepted_targets(&req_id, &body);
                        }
                    }
                }
                // createPosts 仅确认各 target admission，不是 delivery authority。只有携同一
                // csesTrackId 的 WS post 经 dialog 持久化读回后才允许发布 terminal Frame；
                // 因而这里也不能完成 ActionRegistry，避免 WS-first 后迟到 HTTP 重复确认。
                crate::port_reply_emit::emit_create_posts_result(&req_id, outcome, out)?;
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
                if let PortOutcome::Ok(reply) = outcome {
                    if let Ok(raw_body) =
                        crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref())
                    {
                        if let Ok(body) =
                            serde_json::from_slice::<serde_json::Value>(raw_body.as_ref())
                        {
                            let receipts = crate::query::render_ready::receipts::project(&body);
                            let mutations = self
                                .state
                                .timeline_projector
                                .patch_receipts(&receipts)
                                .map_err(|error| {
                                    ImError::Parse(format!("timeline receipt projection: {error}"))
                                })?;
                            if !mutations.is_empty() {
                                out.push(self.build_registered_timeline_frame(
                                    format!("timeline-receipts-{req_id}"),
                                    mutations,
                                    now_ms,
                                )?);
                            }
                        }
                    }
                }
            }

            CorrelationContext::InteractionViewReceipt {
                request_id,
                channel_id,
                message_id,
            } => {
                self.handle_action_view_receipt_reply(
                    &request_id,
                    channel_id,
                    message_id,
                    outcome,
                    out,
                )?;
            }

            CorrelationContext::InteractionMutationAck { pending } => {
                if let PortOutcome::Err(error) = outcome {
                    self.state
                        .pending_authority_actions
                        .remove(&(pending.kind, pending.message_id.clone()));
                    self.state
                        .action_registry
                        .complete_after_terminal_frame(&pending.request_id);
                    tracing::warn!(error = ?error, "interaction mutation authority request failed");
                }
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
                pending_action,
                refresh_legacy_timeline,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    let committed = self
                        .state
                        .channels
                        .get_mut(&channel_id)
                        .map(|channel| channel.commit_contiguous_after_atomic(target_seq, out))
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
                        for bytes in pending_domain_events {
                            out.push(helix_core::Effect::Emit {
                                event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(
                                    bytes,
                                )),
                            });
                        }
                        let causation_id = pending_action
                            .as_ref()
                            .map(|pending| pending.request_id.clone());
                        if let Some(pending) = pending_action {
                            self.state
                                .pending_authority_actions
                                .remove(&(pending.kind, pending.message_id.clone()));
                        }
                        if refresh_legacy_timeline {
                            self.refresh_attached_latest_timeline(channel_id, causation_id, out)?;
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
                channel_id,
                target_seq,
                pending_domain_event,
                pending_action,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    let committed = self
                        .state
                        .channels
                        .get_mut(&channel_id)
                        .map(|channel| channel.commit_contiguous_after_atomic(target_seq, out))
                        .transpose()?
                        .unwrap_or(false);
                    if committed {
                        self.state.invalidate_recent_message_coverage(channel_id);
                        out.push(helix_core::Effect::Emit {
                            event: helix_core::effect::DomainEventBytes(bytes::Bytes::from(
                                pending_domain_event,
                            )),
                        });
                        let causation_id = pending_action
                            .as_ref()
                            .map(|pending| pending.request_id.clone());
                        if let Some(pending) = pending_action {
                            self.state
                                .pending_authority_actions
                                .remove(&(pending.kind, pending.message_id.clone()));
                        }
                        self.refresh_attached_latest_timeline(channel_id, causation_id, out)?;
                    } else {
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            target_seq = target_seq.0,
                            "post_update atomic reply no longer matches contiguous cursor"
                        );
                    }
                }
                PortOutcome::Err(error) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    target_seq = target_seq.0,
                    error = ?error,
                    "post_update atomic persist failed; cursor and projection remain unchanged"
                ),
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
                    self.emit_load_older_failed_frame(&state, now_ms, out)?;
                }
            },

            // G10a durable readback：完整 storage rows 是唯一 success Frame 输入。
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
                    self.emit_timeline_navigation_failed_frame(&state, now_ms, out)?;
                }
            },

            // readback 是 success Frame 的最后一道本地事实屏障。
            CorrelationContext::TimelineNavigationReadback { state } => {
                self.handle_timeline_navigation_readback_reply(state, outcome, now_ms, out)?;
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
