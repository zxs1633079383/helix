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
                    self.state.failed_media_ops.clear();
                    for operation in
                        crate::send::upload_props::decode_durable_scan(reply.0.as_ref())?
                    {
                        self.state.failed_media_ops.insert(
                            (operation.temporary_id().clone(), operation.target().clone()),
                            operation,
                        );
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(error = ?error, "pending_media recovery scan failed");
                }
            }
            return Ok(());
        }

        if let Some(pending_media) = self.state.pending_media_ops.remove(&corr) {
            return self.handle_media_port_reply(pending_media, outcome, out);
        }

        if let Some(pending_media) = self.state.pending_media_stage_persists.remove(&corr) {
            return self.handle_media_stage_persist_reply(pending_media, outcome, out);
        }

        if let Some(pending_upload) = self.state.pending_uploads.remove(&corr) {
            return self.handle_upload_port_reply(pending_upload, outcome, out);
        }

        // 单次 remove 取出上下文（取代原 4 张表逐一 if-let 试探）。
        // 未命中（late/未知 corr）→ no-op，边界零信任（等价原"无人认领 → Ok(())"）。
        let Some(ctx) = self.state.corr_map.remove(&corr) else {
            return Ok(());
        };

        match ctx {
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
            } => self.handle_channel_terminal_persist_reply(
                channel_id,
                terminal_seq,
                trigger,
                outcome,
                out,
            )?,

            CorrelationContext::IncrementBatchPersist { projections } => {
                self.handle_increment_batch_persist_reply(projections, outcome, out)
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
                        let request_id = pending.causation_id.clone();
                        let is_final_target = request_id.as_ref().is_some_and(|request_id| {
                            self.state
                                .pending_forward_deliveries
                                .is_final_target(request_id, pending.channel_id)
                        });
                        let terminal_causation = if is_final_target {
                            request_id.clone()
                        } else {
                            None
                        };
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
                            if let Some(request_id) = request_id {
                                let batch_complete = self
                                    .state
                                    .pending_forward_deliveries
                                    .complete_target(&request_id, pending.channel_id);
                                if batch_complete {
                                    self.state
                                        .action_registry
                                        .complete_after_terminal_frame(&request_id);
                                }
                            }
                        }
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
            } => self.handle_outbound_send_http_reply(channel_id, temporary_id, outcome, out),
            CorrelationContext::TimelineRefreshAfterSendPersist {
                channel_id,
                causation_id,
            } => match outcome {
                PortOutcome::Ok(_) => {
                    self.refresh_attached_latest_timeline(channel_id, causation_id, out)?
                }
                PortOutcome::Err(error) => tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?error,
                    "send terminal persist failed; suppressing timeline refresh"
                ),
            },
            CorrelationContext::RetrySendRehydrate {
                message_id,
                requested_at_ms,
                request_id,
                lookup_by_id,
            } => {
                crate::send::retry_send::handle_rehydrate_reply(
                    self,
                    message_id,
                    requested_at_ms,
                    request_id,
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

            CorrelationContext::SurfaceStorageSnapshot { request } => {
                self.handle_surface_storage_snapshot(request, outcome, out)?;
            }

            CorrelationContext::SurfaceThreadSnapshot { request } => {
                self.handle_surface_thread_snapshot(request, outcome, out)?;
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

            // 左侧会话列表 Scan 回报首先发布完整 render-ready frame；Err 同样发布空 snapshot，
            // 使 frame 客户端能原子结束 loading，而不是用 raw channel 行补算 UI。
            CorrelationContext::DialogListQuery => match outcome {
                PortOutcome::Ok(reply) => {
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
                        )
                    {
                        out.push(frame);
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(error = ?e, "dialog list scan failed");
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
                PortOutcome::Ok(_) => {
                    out.push(crate::acl::to_effect_s1::channel_set_cols(
                        channel_id,
                        vec![("is_remove", helix_core::effect::SqlValue::Integer(1))],
                    ));
                    if let Some(scope) = self.state.dialog_list_frame.scope().cloned() {
                        let patch = serde_json::json!({
                            "channelId": channel_id.as_str(),
                            "left": true,
                        });
                        if let Some(frame) = self
                            .state
                            .dialog_list_frame
                            .patch_from_channel_update(scope, &patch)
                        {
                            out.push(frame);
                        }
                    }
                    out.push(crate::acl::to_effect_s1::emit_channel_left(channel_id));
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        error = ?e,
                        "channel leave http failed; local dialog remains active"
                    );
                }
            },

            CorrelationContext::OutboundChannelCreate { members } => {
                self.handle_outbound_channel_create_reply(members, outcome, now_ms, out);
            }

            CorrelationContext::OutboundMakeTopic {
                root_message_id,
                req_id,
            } => {
                self.handle_outbound_make_topic_reply(&root_message_id, &req_id, outcome, out);
            }

            CorrelationContext::OutboundMakeTopicPersist { projection } => {
                self.handle_make_topic_persist_reply(projection, outcome, out);
            }

            CorrelationContext::OutboundChannelSettings { channel_id } => {
                self.handle_outbound_channel_settings_reply(channel_id, outcome, out);
            }

            CorrelationContext::OutboundChannelSettingsPersist { projection } => {
                self.handle_channel_settings_persist_reply(projection, outcome, out);
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

            // S8 回复族（UC-2.4·issue #57·终局切）：getReplies/getReplyBranch HTTP 回报 → **双 emit**。
            // Ok → 剥信封得裸响应体 → emit `im:read:result{req_id, body}`（冻结 verbatim ② 契约面照旧）
            //   + emit `im:channel:replies`（显式 snapshot/append + revision + 分页元数据）。
            // 信封畸形 / Err → emit `im:read:result{req_id, error}` + emit 空 replyIds（前端 reject·清抽屉）。
            CorrelationContext::OutboundReplies { request } => {
                crate::port_reply_emit::emit_replies(
                    &request,
                    outcome,
                    &mut self.state.reply_projection_revisions,
                    &mut self.state.reply_projection_seen_ids,
                    out,
                );
            }

            CorrelationContext::OutboundLocatePost { req_id, post_id } => {
                crate::port_reply_emit::emit_located_post(&req_id, outcome, out);
                if matches!(outcome, PortOutcome::Ok(_)) && !post_id.is_empty() {
                    if let Some(mutation) = self
                        .state
                        .timeline_projector
                        .patch_located_target(&post_id)
                        .map_err(|error| {
                            ImError::Parse(format!("locate timeline projection: {error}"))
                        })?
                    {
                        out.push(self.build_registered_timeline_frame(
                            format!("locate-{req_id}"),
                            vec![mutation],
                            now_ms,
                        )?);
                    }
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
                crate::port_reply_emit::emit_create_posts_result(&req_id, outcome, out);
            }

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

            CorrelationContext::PostsUpdateAtomic {
                channel_id,
                target_seq,
                pending_domain_events,
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
                        self.refresh_attached_latest_timeline(channel_id, causation_id, out)?;
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

            // D3 上拉翻页：postContext HTTP 回报 → ingest_round 推进 → 续（再发一轮 Http）/停（emit）。
            // Ok → 剥信封 → 取 data 数组喂 ingest_round；Continue 则 alloc 新 corr 重发 + 重注册，
            // Done 则 emit im:messages:older_loaded。Err / 信封畸形 → 用当前已收集态收尾 emit（前端拿
            // 部分历史 + hasMore，不永久挂起；边界零信任不 panic）。
            CorrelationContext::LoadOlderContext { state } => {
                self.handle_load_older_context_reply(state, outcome, out);
            }
        }

        Ok(())
    }
}
