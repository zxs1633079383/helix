use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{CorrelationContext, SendStatus};
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
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

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
            } => match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.on_persist_ok(corr, out)?;
                    }
                    // E3：续拉检查（cursor 已推进后才发，确保 fromSeq 严格递增）
                    if wants_continuation {
                        self.maybe_continue_sync(channel_id, out);
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
                }
            },

            // 路由：发送 Persist 回报（P1 乐观落库）
            CorrelationContext::OptimisticSend { temporary_id } => match outcome {
                PortOutcome::Ok(_) => {
                    // P1 落库成功：将 PendingSend 状态推进到 Sending
                    if let Some(ps) = self.state.pending_sends.get_mut(&temporary_id) {
                        ps.status = SendStatus::Sending;
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        tmp_id = ?temporary_id,
                        error = ?e,
                        "P1 optimistic persist failed"
                    );
                }
            },

            // F1：posts/create HTTP 200 兜底对账（真 Go self-omit echo，见 `send_reconcile`）。
            CorrelationContext::OutboundSendHttp { temporary_id } => match outcome {
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
                        "",
                        temporary_id.0.as_str(),
                    ));
                }
            },
            // 路由：sync Http 回报（sync/notify 响应）→ 清 per-channel 在途标记（B1 gate_inflight）。
            CorrelationContext::SyncPull { channel_id } => {
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.inflight_sync = None;
                }
                // B4：释放 1 个全局 sync 窗口（Ok/Err 都释放——否则失败 sync 永久占窗 → 队列饿死）。
                self.state.sync_scheduler.release_window();
                match outcome {
                    PortOutcome::Ok(reply) => {
                        self.handle_sync_reply(corr, channel_id, reply, out)?
                    }
                    PortOutcome::Err(e) => tracing::warn!(
                        channel_id = channel_id.as_str(), corr = corr.raw(), error = ?e,
                        "sync/notify failed"
                    ),
                }
                // 窗口已释放 → drain 队首续发下一 channel（B4，HX-C011 在途恒 ≤ K）。
                self.drain_sync_queue(out);
            }

            // P6 投影查询 **Scan** 回报：把消息行 emit 成 im:messages:query_result 投影
            // （driver `{items:[]}` 包装）。Err → 空投影（前端拿空列表，不卡渲染）。
            CorrelationContext::MessageQuery { channel_id } => match outcome {
                PortOutcome::Ok(reply) => {
                    out.push(crate::query::emit_message_query_result(
                        &channel_id,
                        reply.0.as_ref(),
                    ));
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        error = ?e,
                        "message query scan failed"
                    );
                    out.push(crate::query::emit_message_query_result(&channel_id, b"[]"));
                }
            },

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
            CorrelationContext::OutboundReadReply { req_id } => match outcome {
                PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                    Ok(raw_body) => {
                        out.push(crate::read_relay::emit_read_result(
                            &req_id,
                            raw_body.as_ref(),
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(
                            req_id = req_id.as_str(),
                            error = ?e,
                            "read reply envelope decode failed"
                        );
                        out.push(crate::read_relay::emit_read_error(
                            &req_id,
                            "response envelope decode failed",
                        ));
                    }
                },
                PortOutcome::Err(e) => {
                    tracing::warn!(req_id = req_id.as_str(), error = ?e, "read outbound http failed");
                    out.push(crate::read_relay::emit_read_error(
                        &req_id,
                        "http request failed",
                    ));
                }
            },

            // S7（issue #56）：byIds 成员快照读族回报 → 双 emit。
            // Ok → ① 照旧透传 im:read:result{req_id, body}（UC-6.4 ② 冻结契约面）；
            //       ② 解析 byIds body（map[channelId][]）逐 channel emit render-ready im:channel:members
            //       （壳退纯绑定·载族自愈）。信封畸形 / Err → 仅回灌 read error（不丢 req_id，前端 reject）。
            CorrelationContext::OutboundMembersByIds { req_id } => {
                crate::port_reply_emit::emit_members_by_ids(&req_id, outcome, out);
            }

            // S8 回复族（UC-2.4·issue #57·终局切）：getReplies/getReplyBranch HTTP 回报 → **双 emit**。
            // Ok → 剥信封得裸响应体 → emit `im:read:result{req_id, body}`（冻结 verbatim ② 契约面照旧）
            //   + emit `im:channel:replies{reqId, replyIds}`（render-ready·helix 抽好 postId·壳纯绑定）。
            // 信封畸形 / Err → emit `im:read:result{req_id, error}` + emit 空 replyIds（前端 reject·清抽屉）。
            CorrelationContext::OutboundReplies { req_id } => {
                crate::port_reply_emit::emit_replies(&req_id, outcome, out);
            }

            // UC-10 通知/待办：global increment-end 触发的 `queryTodoList` HTTP 回报 → 装配
            // `im:todo:updated{items}`（解 data.channels[].posts[]，真源 channel.rs:252-296）。
            // Ok → 剥 ADR-007 信封得裸响应体 → emit_todo_updated（status≠SUCCESS/结构缺失 → 空
            // items 仍 emit，前端无害刷新）。信封畸形 / Err → 不 emit（待办非关键路径，避免误清前端
            // 已渲染待办；下次 hello 重连会重拉，边界零信任不 panic）。
            CorrelationContext::TodoQuery => match outcome {
                PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                    Ok(raw_body) => {
                        out.push(crate::todo::emit_todo_updated(raw_body.as_ref()));
                    }
                    Err(e) => {
                        tracing::warn!(error = ?e, "todo query reply envelope decode failed");
                    }
                },
                PortOutcome::Err(e) => {
                    tracing::warn!(error = ?e, "todo query outbound http failed");
                }
            },

            // D3 上拉翻页：postContext HTTP 回报 → ingest_round 推进 → 续（再发一轮 Http）/停（emit）。
            // Ok → 剥信封 → 取 data 数组喂 ingest_round；Continue 则 alloc 新 corr 重发 + 重注册，
            // Done 则 emit im:messages:older_loaded。Err / 信封畸形 → 用当前已收集态收尾 emit（前端拿
            // 部分历史 + hasMore，不永久挂起；边界零信任不 panic）。
            CorrelationContext::LoadOlderContext { mut state } => match outcome {
                PortOutcome::Ok(reply) => {
                    // 剥信封 → 取 wire Post 数组；信封/base64 畸形 → 空批 → stop（零信任不 panic）。
                    let rows = unwrap_sync_envelope(reply.0.as_ref())
                        .map(|raw| crate::older_context::extract_post_rows(&raw))
                        .unwrap_or_default();
                    match state.ingest_round(&rows) {
                        crate::older_context::RoundDecision::Continue => {
                            let next_corr = self.alloc_corr_internal();
                            let (_, body) = crate::older_context::build_post_context_body(&state);
                            out.push(crate::older_context::post_context_http(
                                &self.config.api_base_url,
                                &body,
                                next_corr,
                                self.state.connection_id.as_deref(),
                            ));
                            self.state
                                .corr_map
                                .insert(next_corr, CorrelationContext::LoadOlderContext { state });
                        }
                        crate::older_context::RoundDecision::Done => {
                            out.push(crate::older_context::emit_older_loaded(&state));
                        }
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(error = ?e, "load_older_context postContext http failed");
                    // 收尾：把当前已收集的更早消息（可能为空）emit，前端结束 loading。
                    // 用空批喂一轮触发 stop(has_more=false) → 已收集态被 sort+truncate+定 has_more。
                    state.ingest_round(&[]);
                    out.push(crate::older_context::emit_older_loaded(&state));
                }
            },
        }

        Ok(())
    }
}
