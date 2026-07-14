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
                wants_continuation,
                dialog_patches,
            } => self.handle_channel_persist_reply(
                corr,
                channel_id,
                wants_continuation,
                dialog_patches,
                outcome,
                out,
            )?,

            CorrelationContext::IncrementHydrationPersist {
                channel_id,
                need_sync,
            } => self.handle_increment_hydration_persist(channel_id, need_sync, outcome, out)?,

            CorrelationContext::DialogPatchByPost { pending } => match outcome {
                PortOutcome::Ok(reply) => {
                    if let Some(effect) = pending.emit_from_channel_reply(reply.0.as_ref()) {
                        out.push(effect);
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
                        if let Some(effect) = pending.emit_from_channel_reply(reply.0.as_ref()) {
                            out.push(effect);
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
                self.handle_optimistic_send_reply(temporary_id, outcome);
            }

            // F1：posts/create HTTP 200 兜底对账（真 Go self-omit echo，见 `send_reconcile`）。
            CorrelationContext::OutboundSendHttp {
                channel_id,
                temporary_id,
            } => self.handle_outbound_send_http_reply(channel_id, temporary_id, outcome, out),
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
            CorrelationContext::SyncPull { channel_id } => {
                self.handle_sync_pull_reply(corr, channel_id, outcome, out)?;
            }

            // P6 投影查询 **Scan** 回报：把消息行 emit 成 im:messages:query_result 投影
            // （driver `{items:[]}` 包装）。Err → 空投影（前端拿空列表，不卡渲染）。
            CorrelationContext::MessageQuery { channel_id } => {
                self.handle_message_query_reply(channel_id, outcome, out);
            }

            CorrelationContext::ForwardSourceLookup { assembly } => {
                if let Err(error) =
                    crate::forward::handle_lookup_reply(self, *assembly, outcome, out)
                {
                    tracing::warn!(error = ?error, "forward source assembly failed");
                }
            }

            // UC-3 dialogList 投影 **Scan** 回报：把 channel 行 emit 成 im:channels:projection
            // 完整 dialogList（含 unread_count，前端零计算）。Err → 空 dialogList（不卡渲染）。
            CorrelationContext::DialogListQuery => match outcome {
                PortOutcome::Ok(reply) => {
                    out.push(crate::query::emit_dialog_list_result(reply.0.as_ref()));
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(error = ?e, "dialog list scan failed");
                    out.push(crate::query::emit_dialog_list_result(b"[]"));
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

            CorrelationContext::OutboundLocatePost { req_id } => {
                crate::port_reply_emit::emit_located_post(&req_id, outcome, out);
            }

            CorrelationContext::OutboundCreatePosts { req_id } => {
                crate::port_reply_emit::emit_create_posts_result(&req_id, outcome, out);
            }

            CorrelationContext::OutboundPostReaders { req_id } => {
                crate::port_reply_emit::emit_post_readers(&req_id, outcome, out);
            }

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
