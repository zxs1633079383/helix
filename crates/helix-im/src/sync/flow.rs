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
        let (cursor_seq, prev_from_seq, has_inflight) = match self.state.channels.get(&channel_id) {
            Some(ch) => (
                ch.cursor.value(),
                ch.last_sync_from_seq,
                ch.inflight_sync.is_some(),
            ),
            None => return,
        };

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
        use crate::sync_session::SyncResponse;

        // #6 信封解层（仅 SyncPull 路径）：剥 ADR-007 `{status,headers,body:base64(raw_go_body)}` 信封
        // + base64 decode 出裸 Go body 再解析（直喂信封会找不到 data.entries → 静默 NoChange 丢消息）。
        // 边界零信任（HX-C 不变量 4）：信封 / base64 畸形走 Result::Err，不 unwrap / panic / 静默吞。
        let raw_body = unwrap_sync_envelope(reply.0.as_ref())?;
        let response = parse_sync_response(raw_body.as_ref(), channel_id)
            .map_err(|e| ImError::Parse(format!("sync response parse error: {}", e)))?;

        match response {
            SyncResponse::NoChange => {
                self.finish_increment_hydration(channel_id, out)?;
            }
            SyncResponse::Events {
                events,
                messages,
                next_seq,
                needs_continuation,
            } => {
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

                let max_event_seq = events.iter().map(|e| e.seq).max().unwrap_or(Seq(0));
                let commit_target = next_seq.max(max_event_seq);
                let persist_corr = self.alloc_corr_internal();
                let auth_user_id = self.config.auth_user_id.as_str();
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
                // G-S2/G-S3：message/channel 派生写与 cursor 必须属于同一个相关事务。
                // Persist 回执前既不推进内存 cursor，也不发布 mutation/dialog/sync-state。
                persist_ops.push(crate::acl::to_effect::advance_cursor_op(
                    channel_id,
                    commit_target,
                ));
                if !dialog_patches.is_empty() {
                    persist_ops.push(crate::acl::to_effect::get_channel_row_op(channel_id));
                }
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.insert(persist_corr, commit_target);
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

    /// B4：drain SyncScheduler 队列至满窗（拆借 state + next_corr 分配器，避免 &mut self 双借）。
    pub(crate) fn drain_sync_queue(&mut self, out: &mut EffectSink) {
        let api_base_url = self.config.api_base_url.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            crate::sync_scheduler::drain(state, &api_base_url, alloc, out);
        });
    }
}
