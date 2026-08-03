//! UC-4.5 陌生 channel 单频道 hydration 编排。
//!
//! HTTP `incrementByChannelId` 仍保留通用 read relay，但其 `data=*IncrementChannel` 还会复用
//! WS increment 解析/应用规则。channel/member 确认落库后才启动既有 sync；terminal sync commit
//! 后从本地 channel/message 表读回并复用 render-ready query 投影，前端不参与业务计算。

use crate::error::ImError;
use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext, HydrationPersistSnapshot};
use helix_core::effect::{Effect, GetSpec, ScanOrder, ScanSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, EffectSink};

const HYDRATION_MESSAGE_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("create_at"),
    ScanOrder::desc("temporary_id"),
];

impl ImModule {
    /// B-rest 测试 / driver 入口：直接喂一个解析好的 `increment_channel` 帧。
    pub fn ingest_increment(
        &mut self,
        inc: &crate::sync_session::IncrementChannel,
        out: &mut EffectSink,
    ) {
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, 0, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::increment_channel::apply_increment(&mut ctx, inc, out);
        });
    }

    fn ingest_increment_hydration(
        &mut self,
        inc: &crate::sync_session::IncrementChannel,
        persist_corr: Correlation,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> bool {
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, now_ms, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::increment_channel::apply_increment_hydration(
                &mut ctx,
                inc,
                persist_corr,
                out,
            )
        })
    }

    /// 记录 increment 应用在内存中写入的临时壳，供持久化失败时恢复旧状态。
    fn hydration_snapshot(&self, channel_id: ChannelId) -> HydrationPersistSnapshot {
        HydrationPersistSnapshot {
            had_channel: self.state.channels.contains_key(&channel_id),
            previous_target: self.state.increment_target.get(&channel_id).copied(),
            was_increment_fetched: self.state.increment_fetched.contains(&channel_id),
            was_need_sync_skip: self.state.need_sync_skip.contains(&channel_id),
            previous_about_me_len: self.state.about_me_post_ids.len(),
        }
    }

    /// 回滚尚未通过 durable channel/member barrier 的本地临时事实。
    fn restore_hydration_snapshot(
        &mut self,
        channel_id: ChannelId,
        snapshot: &HydrationPersistSnapshot,
    ) {
        if !snapshot.had_channel {
            self.state.channels.remove(&channel_id);
        }
        if let Some(target) = snapshot.previous_target {
            self.state.increment_target.insert(channel_id, target);
        } else {
            self.state.increment_target.remove(&channel_id);
        }
        if !snapshot.was_increment_fetched {
            self.state.increment_fetched.remove(&channel_id);
            self.state.increment_order.retain(|id| *id != channel_id);
        }
        if snapshot.was_need_sync_skip {
            self.state.need_sync_skip.insert(channel_id);
        } else {
            self.state.need_sync_skip.remove(&channel_id);
        }
        self.state
            .about_me_post_ids
            .truncate(snapshot.previous_about_me_len);
    }

    /// 从 driver 的 rows JSON 回包解出对象列表，畸形回包 fail closed。
    fn hydration_rows(
        reply: &helix_core::tick::ReplyBytes,
    ) -> Result<Vec<serde_json::Value>, ImError> {
        let value = serde_json::from_slice::<serde_json::Value>(reply.0.as_ref())
            .map_err(|error| ImError::Parse(format!("hydration read-back rows: {error}")))?;
        value
            .as_array()
            .cloned()
            .ok_or_else(|| ImError::Parse("hydration read-back must be an array".to_string()))
    }

    /// 发送 channel row 读回，后续阶段均由 matching corr 串联。
    fn start_hydration_channel_readback(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        out: &mut EffectSink,
    ) {
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "channel",
                key_col: "id",
                key_val: SqlValue::Text(channel_id.as_str().to_string()),
            })],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::HydrationChannelReadback { req_id, channel_id },
        );
    }

    /// channel row 读回成功后，读取当前 viewer 的 member 绝对态。
    pub(crate) fn handle_hydration_channel_readback(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let rows = match outcome {
            PortOutcome::Ok(reply) => Self::hydration_rows(reply),
            PortOutcome::Err(error) => Err(ImError::Parse(format!(
                "channel read-back failed: {error:?}"
            ))),
        };
        let channel = match rows {
            Ok(mut rows) => rows.pop(),
            Err(error) => {
                self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
                return Ok(());
            }
        };
        let Some(channel) = channel.filter(|row| {
            row.get("id").and_then(serde_json::Value::as_str) == Some(channel_id.as_str())
                && row.get("team_id").and_then(serde_json::Value::as_str)
                    == Some(self.config.company_id.as_str())
                && row.get("user_id").and_then(serde_json::Value::as_str)
                    == Some(self.config.auth_user_id.as_str())
        }) else {
            self.finish_hydration_error(
                &req_id,
                channel_id,
                "channel read-back scope mismatch",
                out,
            );
            return Ok(());
        };
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::ScopedGet(helix_core::effect::ScopedGetSpec {
                table: "channel_member",
                scope_col: "channel_id",
                scope_val: SqlValue::Text(channel_id.as_str().to_string()),
                key_col: "user_id",
                key_val: SqlValue::Text(self.config.auth_user_id.clone()),
            })],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::HydrationMemberReadback {
                req_id,
                channel_id,
                channel: Box::new(channel),
            },
        );
        Ok(())
    }

    /// member row 读回成功后，读取频道消息窗口。
    pub(crate) fn handle_hydration_member_readback(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let rows = match outcome {
            PortOutcome::Ok(reply) => Self::hydration_rows(reply),
            PortOutcome::Err(error) => Err(ImError::Parse(format!(
                "member read-back failed: {error:?}"
            ))),
        };
        let Some(member) = (match rows {
            Ok(mut rows) => rows.pop(),
            Err(error) => {
                self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
                return Ok(());
            }
        }) else {
            self.finish_hydration_error(&req_id, channel_id, "member read-back missing", out);
            return Ok(());
        };
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "message",
                limit: Some(50),
                filter: Some((
                    "channel_id",
                    SqlValue::Text(channel_id.as_str().to_string()),
                )),
                order_by: HYDRATION_MESSAGE_ORDER,
            })],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::HydrationMessagesReadback {
                req_id,
                channel_id,
                channel,
                member: Box::new(member),
            },
        );
        Ok(())
    }

    /// 消息窗口读回成功后，读取 durable cursor。
    pub(crate) fn handle_hydration_messages_readback(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        member: Box<serde_json::Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let messages = match outcome {
            PortOutcome::Ok(reply) => match Self::hydration_rows(reply) {
                Ok(rows) => serde_json::Value::Array(rows),
                Err(error) => {
                    self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
                    return Ok(());
                }
            },
            PortOutcome::Err(error) => {
                self.finish_hydration_error(
                    &req_id,
                    channel_id,
                    &format!("message read-back failed: {error:?}"),
                    out,
                );
                return Ok(());
            }
        };
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "channel_event_cursor",
                key_col: "channel_id",
                key_val: SqlValue::Text(channel_id.as_str().to_string()),
            })],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::HydrationCursorReadback {
                req_id,
                channel_id,
                channel,
                member,
                messages: Box::new(messages),
            },
        );
        Ok(())
    }

    /// cursor 读回成功后只释放一个 ChannelHydrationResult。
    pub(crate) fn handle_hydration_cursor_readback(
        &mut self,
        req_id: String,
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        member: Box<serde_json::Value>,
        messages: Box<serde_json::Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let cursor = match outcome {
            PortOutcome::Ok(reply) => match Self::hydration_rows(reply) {
                Ok(mut rows) => rows
                    .pop()
                    .and_then(|row| {
                        row.get("last_event_seq")
                            .and_then(serde_json::Value::as_i64)
                    })
                    .unwrap_or_else(|| {
                        self.state
                            .channels
                            .get(&channel_id)
                            .map(|channel| channel.cursor.value().0 as i64)
                            .unwrap_or(0)
                    }),
                Err(error) => {
                    self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
                    return Ok(());
                }
            },
            PortOutcome::Err(error) => {
                self.finish_hydration_error(
                    &req_id,
                    channel_id,
                    &format!("cursor read-back failed: {error:?}"),
                    out,
                );
                return Ok(());
            }
        };
        self.state.hydration_req_ids.remove(&channel_id);
        out.push(crate::read_relay::emit_read_body(
            &req_id,
            serde_json::json!({
                "channelId": channel_id.as_str(),
                "channel": *channel,
                "member": *member,
                "messages": *messages,
                "cursor": cursor,
                "completion": "hydrated",
            }),
        ));
        Ok(())
    }

    /// 失败终态统一清理 hydration bookkeeping 并只回灌一次 error Result。
    pub(crate) fn finish_hydration_error(
        &mut self,
        req_id: &str,
        channel_id: ChannelId,
        reason: &str,
        out: &mut EffectSink,
    ) {
        self.state.hydration_pending.remove(&channel_id);
        self.state.hydration_req_ids.remove(&channel_id);
        out.push(crate::read_relay::emit_read_error(req_id, reason));
    }

    /// 以 channel 取回 hydration reqId 并结算一次失败，供 sync caller 处理解析/网络错误。
    pub(crate) fn fail_hydration_for_channel(
        &mut self,
        channel_id: ChannelId,
        reason: &str,
        out: &mut EffectSink,
    ) {
        let Some(req_id) = self.state.hydration_req_ids.get(&channel_id).cloned() else {
            return;
        };
        self.finish_hydration_error(&req_id, channel_id, reason, out);
    }

    pub(crate) fn handle_increment_hydration_reply(
        &mut self,
        req_id: &str,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    let body: serde_json::Value = match serde_json::from_slice(&raw_body) {
                        Ok(body) => body,
                        Err(e) => {
                            tracing::warn!(req_id, error = ?e, "increment hydration body is not json");
                            out.push(crate::read_relay::emit_read_error(
                                req_id,
                                "increment hydration body is not json",
                            ));
                            return Ok(());
                        }
                    };
                    let Some(data) = body.get("data") else {
                        tracing::warn!(req_id, "increment hydration reply missing data");
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            "increment hydration reply missing data",
                        ));
                        return Ok(());
                    };
                    if data.is_null() {
                        out.push(crate::read_relay::emit_read_body(
                            req_id,
                            serde_json::Value::Null,
                        ));
                        return Ok(());
                    }
                    let remote_company = data
                        .get("teamId")
                        .or_else(|| data.get("team_id"))
                        .and_then(serde_json::Value::as_str);
                    if remote_company != Some(self.config.company_id.as_str()) {
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            "increment hydration tenant mismatch",
                        ));
                        return Ok(());
                    }
                    let Some(increment) = crate::ws::parser::parse_increment_channel(data) else {
                        tracing::warn!(
                            req_id,
                            "increment hydration reply has invalid IncrementChannel"
                        );
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            "increment hydration reply has invalid channel",
                        ));
                        return Ok(());
                    };
                    let snapshot = self.hydration_snapshot(increment.channel_id);
                    let persist_corr = self.alloc_corr_internal();
                    let has_persist =
                        self.ingest_increment_hydration(&increment, persist_corr, now_ms, out);
                    // needSync 是服务端按其会话水位给的提示，不能覆盖本端持久化事实。独立窗口
                    // 首次见到频道时本地 cursor=0；只要仍落后 lastEventSeq，就必须从本地 cursor
                    // 拉 sync，把 events/messages 与 cursor 在同一 commit 链落稳后再做终态投影。
                    let local_cursor = self
                        .state
                        .channels
                        .get(&increment.channel_id)
                        .map(|channel| channel.cursor.value())
                        .unwrap_or(crate::state::Seq(0));
                    let requires_sync =
                        increment.need_sync || local_cursor < increment.last_event_seq;
                    if requires_sync {
                        self.state.need_sync_skip.remove(&increment.channel_id);
                    }
                    if has_persist {
                        self.state.corr_map.insert(
                            persist_corr,
                            CorrelationContext::IncrementHydrationPersist {
                                channel_id: increment.channel_id,
                                req_id: req_id.to_string(),
                                need_sync: requires_sync,
                                raw_increment: increment.raw.as_ref().to_vec(),
                                snapshot,
                            },
                        );
                    } else {
                        tracing::warn!(
                            channel_id = increment.channel_id.as_str(),
                            "increment hydration produced no durable writes; final projection suppressed"
                        );
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            "increment hydration produced no durable writes",
                        ));
                    }
                }
                Err(e) => {
                    tracing::warn!(req_id, error = ?e, "increment hydration envelope decode failed");
                    out.push(crate::read_relay::emit_read_error(
                        req_id,
                        "response envelope decode failed",
                    ));
                }
            },
            PortOutcome::Err(e) => {
                tracing::warn!(req_id, error = ?e, "increment hydration http failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "http request failed",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn handle_increment_hydration_persist(
        &mut self,
        channel_id: ChannelId,
        req_id: String,
        need_sync: bool,
        raw_increment: Vec<u8>,
        snapshot: HydrationPersistSnapshot,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                let _ = raw_increment;
                self.after_increment_persist(channel_id, req_id, need_sync, out)?;
            }
            PortOutcome::Err(e) => {
                self.restore_hydration_snapshot(channel_id, &snapshot);
                tracing::warn!(
                    channel_id = channel_id.as_str(),
                    error = ?e,
                    "increment hydration channel/member persist failed"
                );
                self.finish_hydration_error(
                    &req_id,
                    channel_id,
                    "channel/member persist failed",
                    out,
                );
            }
        }
        Ok(())
    }

    fn after_increment_persist(
        &mut self,
        channel_id: ChannelId,
        req_id: String,
        need_sync: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.hydration_pending.insert(channel_id);
        self.state
            .hydration_req_ids
            .insert(channel_id, req_id.clone());
        if !need_sync {
            return self.finish_increment_hydration(channel_id, out);
        }
        let api_base_url = self.config.api_base_url.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            crate::sync_scheduler::enqueue_and_drain_with_trigger(
                state,
                &api_base_url,
                &[channel_id],
                crate::state::SyncTrigger::Hydration,
                alloc,
                out,
            );
        });
        Ok(())
    }

    /// terminal sync 后从本地表读回完整 dialogList 与该 channel messages。
    pub(crate) fn finish_increment_hydration(
        &mut self,
        channel_id: ChannelId,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !self.state.hydration_pending.remove(&channel_id) {
            return Ok(());
        }
        let Some(req_id) = self.state.hydration_req_ids.get(&channel_id).cloned() else {
            return Ok(());
        };
        self.start_hydration_channel_readback(req_id, channel_id, out);
        Ok(())
    }
}
