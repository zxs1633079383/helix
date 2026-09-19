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

/// Classify a missing-data reply using fixed values; never log remote payloads or messages.
fn hydration_reply_diagnostic(body: &serde_json::Value) -> (&'static str, &'static str) {
    match body.get("status").and_then(serde_json::Value::as_str) {
        Some("failed") => (
            "failed",
            if body.get("message").and_then(serde_json::Value::as_str)
                == Some("user not member of channel")
            {
                "remote_not_member"
            } else {
                "remote_business_failure"
            },
        ),
        Some("SUCCESS") => ("SUCCESS", "missing_data"),
        Some("FINISH") => ("FINISH", "missing_data"),
        Some(_) => ("unknown", "invalid_business_status"),
        None => ("missing_or_invalid", "invalid_business_status"),
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::hydration_reply_diagnostic;
    use serde_json::json;

    /// Preserve the actionable failure category without exporting arbitrary server text.
    #[test]
    fn hydration_diagnostic_classifies_business_failure_without_remote_text() {
        for (body, expected) in [
            (
                json!({"status":"failed","message":"user not member of channel"}),
                ("failed", "remote_not_member"),
            ),
            (
                json!({"status":"failed","message":"secret=private\nraw payload"}),
                ("failed", "remote_business_failure"),
            ),
            (json!({"status":"SUCCESS"}), ("SUCCESS", "missing_data")),
            (json!({"status":"FINISH"}), ("FINISH", "missing_data")),
            (
                json!({"status":"private-value"}),
                ("unknown", "invalid_business_status"),
            ),
            (
                json!({"status":200}),
                ("missing_or_invalid", "invalid_business_status"),
            ),
            (json!({}), ("missing_or_invalid", "invalid_business_status")),
        ] {
            assert_eq!(hydration_reply_diagnostic(&body), expected);
        }
    }
}

impl ImModule {
    /// B-rest 测试 / driver 入口：直接喂一个解析好的 `increment_channel` 帧。
    pub fn ingest_increment(
        &mut self,
        inc: &crate::sync_session::IncrementChannel,
        out: &mut EffectSink,
    ) {
        let start = out.as_slice().len();
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, 0, &api_base_url, &auth_user_id, alloc);
            crate::ws::handlers::increment_channel::apply_increment(&mut ctx, inc, out);
        });
        self.render_scope.guard_effects(&self.config, start, out);
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

    /// 从 Go G09d raw snapshot 冻结有序全角色 roster，并校验 memberCount 与唯一 user id。
    fn ordered_hydration_roster(raw_increment: &[u8]) -> Result<Vec<serde_json::Value>, ImError> {
        let data: serde_json::Value = serde_json::from_slice(raw_increment)
            .map_err(|error| ImError::Parse(format!("hydration roster parse: {error}")))?;
        let members = data
            .get("members")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ImError::Parse("hydration roster members missing".to_string()))?;
        let member_count = data
            .get("memberCount")
            .or_else(|| data.get("member_count"))
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ImError::Parse("hydration roster memberCount missing".to_string()))?;
        if member_count as usize != members.len() || members.is_empty() {
            return Err(ImError::Parse(
                "hydration roster count mismatch or empty".to_string(),
            ));
        }
        let mut seen = std::collections::HashSet::with_capacity(members.len());
        let mut ordered = Vec::with_capacity(members.len());
        for member in members {
            let user_id = member
                .get("userId")
                .or_else(|| member.get("id"))
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ImError::Parse("hydration roster member id missing".to_string()))?;
            if !seen.insert(user_id.to_string()) {
                return Err(ImError::Parse(
                    "hydration roster contains duplicate member".to_string(),
                ));
            }
            ordered.push(serde_json::json!({
                "userId": user_id,
                "teamId": member.get("teamId").and_then(serde_json::Value::as_str).unwrap_or_default(),
                "role": member.get("role").and_then(serde_json::Value::as_str).unwrap_or("MEMBER"),
                "nickName": member.get("nickName").and_then(serde_json::Value::as_str).unwrap_or_default(),
            }));
        }
        Ok(ordered)
    }

    /// 将 durable member rows 按冻结位序重建完整 channel roster，并拒绝租户/集合漂移。
    fn attach_hydration_roster(
        &self,
        channel: &mut serde_json::Value,
        durable_rows: &[serde_json::Value],
        ordered: &[serde_json::Value],
    ) -> Result<(), ImError> {
        let mut durable = std::collections::HashMap::with_capacity(durable_rows.len());
        let expected_teams: std::collections::HashMap<_, _> = ordered
            .iter()
            .filter_map(|member| Some((member["userId"].as_str()?, member["teamId"].as_str()?)))
            .collect();
        for row in durable_rows {
            let user_id = row
                .get("user_id")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ImError::Parse("hydration durable member id missing".to_string()))?;
            // member.teamId 是成员自身公司，允许与频道/当前 UI 公司不同。
            let expected_team = expected_teams.get(user_id).copied();
            if expected_team
                .filter(|team| !team.is_empty())
                .is_some_and(|team| {
                    row.get("team_id").and_then(serde_json::Value::as_str) != Some(team)
                })
            {
                return Err(ImError::Parse(
                    "hydration durable member tenant mismatch".to_string(),
                ));
            }
            if durable.insert(user_id, row).is_some() {
                return Err(ImError::Parse(
                    "hydration durable member duplicate".to_string(),
                ));
            }
        }
        if durable.len() != ordered.len() {
            return Err(ImError::Parse(
                "hydration durable roster count mismatch".to_string(),
            ));
        }
        let object = channel
            .as_object_mut()
            .ok_or_else(|| ImError::Parse("hydration channel shape invalid".to_string()))?;
        let mut admins = Vec::new();
        let mut bosses = Vec::new();
        let mut owner = serde_json::Value::Null;
        let mut projected_members = Vec::with_capacity(ordered.len());
        for member in ordered {
            let user_id = member["userId"]
                .as_str()
                .ok_or_else(|| ImError::Parse("hydration ordered member invalid".to_string()))?;
            let row = durable.get(user_id).ok_or_else(|| {
                ImError::Parse("hydration durable roster set mismatch".to_string())
            })?;
            let role = row
                .get("role")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("MEMBER");
            let projected = serde_json::json!({
                "userId": user_id,
                "teamId": row.get("team_id").and_then(serde_json::Value::as_str).unwrap_or_default(),
                "role": role,
                "nickName": row.get("nick_name").and_then(serde_json::Value::as_str).unwrap_or_default(),
            });
            projected_members.push(projected.clone());
            match role {
                "ADMIN" | "MANAGER" => admins.push(projected.clone()),
                "BOSS" => bosses.push(projected.clone()),
                "OWNER" | "CREATOR" => owner = projected.clone(),
                _ => {}
            }
        }
        object.insert(
            "members".to_string(),
            serde_json::Value::Array(projected_members),
        );
        object.insert("adminUsers".to_string(), serde_json::Value::Array(admins));
        object.insert("boss".to_string(), serde_json::Value::Array(bosses));
        object.insert("owner".to_string(), owner);
        object.insert("memberCount".to_string(), serde_json::json!(ordered.len()));
        Ok(())
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

    /// channel row 读回成功后，扫描完整 durable roster 并在下一阶段验证 viewer。
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
                    == self.render_scope.company_for(channel_id.as_str())
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
        let channel = crate::query::render_ready::channel::shape_channel_row(&channel);
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "channel_member",
                limit: None,
                filter: Some((
                    "channel_id",
                    SqlValue::Text(channel_id.as_str().to_string()),
                )),
                order_by: &[],
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
        let durable_rows = match rows {
            Ok(rows) => rows,
            Err(error) => {
                self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
                return Ok(());
            }
        };
        let Some(member) = durable_rows
            .iter()
            .find(|row| {
                row.get("user_id").and_then(serde_json::Value::as_str)
                    == Some(self.config.auth_user_id.as_str())
            })
            .cloned()
        else {
            self.finish_hydration_error(&req_id, channel_id, "member read-back missing", out);
            return Ok(());
        };
        let Some(ordered) = self
            .state
            .hydration_ordered_rosters
            .get(&channel_id)
            .cloned()
        else {
            self.finish_hydration_error(&req_id, channel_id, "ordered roster missing", out);
            return Ok(());
        };
        let mut channel = *channel;
        if let Err(error) = self.attach_hydration_roster(&mut channel, &durable_rows, &ordered) {
            self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
            return Ok(());
        }
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
                channel: Box::new(channel),
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
        let emit_channel_increment = self
            .state
            .hydration_emit_channel_increment
            .remove(&channel_id);
        self.state.hydration_req_ids.remove(&channel_id);
        self.state.hydration_ordered_rosters.remove(&channel_id);
        let unread_reconcile = self
            .state
            .hydration_authority_unreads
            .remove(&channel_id)
            .and_then(|authority| {
                member
                    .get("unread_count")
                    .or_else(|| member.get("unreadCount"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|local| {
                        serde_json::json!({
                            "authority": authority,
                            "local": local,
                            "status": if authority == local { "match" } else { "mismatch" },
                        })
                    })
            });
        if emit_channel_increment {
            out.push(
                crate::event::MessageV3Event::new("im:channel:increment", (*channel).clone())?
                    .into_effect(),
            );
        }
        out.push(crate::read_relay::emit_read_body(
            &req_id,
            serde_json::json!({
                "channelId": channel_id.as_str(),
                "channel": *channel,
                "member": *member,
                "messages": *messages,
                "cursor": cursor,
                "completion": "hydrated",
                "unreadReconcile": unread_reconcile,
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
        self.state
            .hydration_emit_channel_increment
            .remove(&channel_id);
        self.state.hydration_ordered_rosters.remove(&channel_id);
        self.state.hydration_authority_unreads.remove(&channel_id);
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

    /// Decode hydration replies; diagnostics do not change the existing Result contract.
    pub(crate) fn handle_increment_hydration_reply(
        &mut self,
        req_id: &str,
        emit_channel_increment: bool,
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
                        let (business_status, reason) = hydration_reply_diagnostic(&body);
                        tracing::warn!(
                            req_id,
                            operation = "channel_hydration",
                            phase = "http_reply",
                            business_status,
                            reason,
                            "hydration failed"
                        );
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
                    if remote_company.is_none_or(str::is_empty) {
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            "increment hydration missing channel company",
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
                    if let Some(authority_unread) = data
                        .get("unreadCount")
                        .or_else(|| data.get("unread_count"))
                        .and_then(serde_json::Value::as_i64)
                    {
                        self.state
                            .hydration_authority_unreads
                            .insert(increment.channel_id, authority_unread);
                    }
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
                                emit_channel_increment,
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
        emit_channel_increment: bool,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(_) => {
                let ordered_roster = match Self::ordered_hydration_roster(&raw_increment) {
                    Ok(roster) => roster,
                    Err(error) => {
                        self.restore_hydration_snapshot(channel_id, &snapshot);
                        self.finish_hydration_error(&req_id, channel_id, &error.to_string(), out);
                        return Ok(());
                    }
                };
                self.state
                    .hydration_ordered_rosters
                    .insert(channel_id, ordered_roster);
                self.after_increment_persist(
                    channel_id,
                    req_id,
                    need_sync,
                    emit_channel_increment,
                    out,
                )?;
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
        emit_channel_increment: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.state.hydration_pending.insert(channel_id);
        if emit_channel_increment {
            self.state
                .hydration_emit_channel_increment
                .insert(channel_id);
        }
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
