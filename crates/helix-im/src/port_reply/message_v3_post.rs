//! G-01 MessageV3 post 的原子提交、复合键读回与双事件释放。

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{CorrelationContext, ImState};
use crate::sync_session::EventEnvelope;
use helix_core::effect::{Correlation, Effect, HttpRequest, SqlValue};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;

/// 将已通过 channel gate 的 post 绑定到 message/channel/member/cursor 原子提交。
pub(crate) fn queue_commit(
    state: &mut ImState,
    auth_user_id: &str,
    corr: Correlation,
    event: EventEnvelope,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let projection = crate::event::post::authority_projection(&event)?;
    let ops =
        crate::channel_write::message_v3_commit_ops(&event, auth_user_id, &projection.last_post);
    if ops.is_empty() {
        return Err(ImError::Parse(
            "MessageV3 post requires viewer identity and SQLite-range eventSeq".to_string(),
        ));
    }
    state.corr_map.insert(
        corr,
        CorrelationContext::MessageV3PostPersist {
            event: Box::new(event),
            received_data: Box::new(projection.received_data),
        },
    );
    out.push(Effect::PersistAtomic { corr, ops });
    Ok(())
}

/// 将已连续的普通 type=2 edit 绑定到 message patch 与 cursor 的同一原子提交。
pub(crate) fn queue_post_update_commit(
    state: &mut ImState,
    auth_user_id: &str,
    corr: Correlation,
    event: EventEnvelope,
    out: &mut EffectSink,
) {
    let message_id = event
        .msg_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(event.fields.id.as_str());
    let pending_domain_event = match crate::acl::to_effect::emit_post_updated_for_viewer(
        event.channel_id,
        event.seq.0,
        message_id,
        &event.fields,
        auth_user_id,
    ) {
        Effect::Emit { event } => event.0.to_vec(),
        _ => unreachable!("post_update projection constructor must emit"),
    };
    let ops = vec![
        crate::channel::edit_content_op(message_id, &event.fields),
        crate::acl::to_effect::advance_cursor_op(event.channel_id, event.seq),
    ];
    state.corr_map.insert(
        corr,
        CorrelationContext::PostUpdateAtomic {
            has_category_posts: event.fields.msg_type == "CATEGORY_CHAIN",
            event: Box::new(event),
            pending_domain_event,
        },
    );
    out.push(Effect::PersistAtomic { corr, ops });
}

impl ImModule {
    /// G-01 原子提交成功后推进 cursor，并发起当前 viewer 复合键读回。
    pub(super) fn handle_message_v3_post_persist_reply(
        &mut self,
        event: EventEnvelope,
        received_data: serde_json::Value,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.diagnose(crate::diagnostics::Observation {
            event: "delivery_stage",
            stage: "persist_terminal",
            domain: "legacy_event_seq",
            business_event_id: event.event_id.as_str(),
            path: "live_ws",
            result: if matches!(outcome, PortOutcome::Ok(_)) {
                "success"
            } else {
                "failed"
            },
            channel: event.channel_id.as_str(),
            seq: Some(event.seq.0),
            count: 1,
            ..Default::default()
        });
        if !matches!(outcome, PortOutcome::Ok(_)) {
            if let Some(req_id) = event.causation_id.as_deref() {
                self.fail_forward(req_id, "forward post persist failed", out)?;
            }
            if let Some(channel) = self.state.channels.get_mut(&event.channel_id) {
                channel.restore_message_v3_post(event, out);
            }
            return Ok(());
        }

        let next = self
            .state
            .channels
            .get_mut(&event.channel_id)
            .and_then(|channel| channel.commit_message_v3_post(event.seq));
        self.diagnose_checkpoint(event.channel_id, "persist_committed");
        let readback_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: readback_corr,
            ops: vec![crate::channel_write::message_v3_member_read_op(
                event.channel_id,
                self.config.auth_user_id.as_str(),
            )],
        });
        self.state.corr_map.insert(
            readback_corr,
            CorrelationContext::MessageV3PostReadback {
                received_data: Box::new(received_data),
                channel_id: event.channel_id,
                causation_id: event.causation_id.clone(),
            },
        );
        // PersistOk only proves the user-scoped message fact.  When the channel
        // has no trusted durable owner row yet, proactively start the existing
        // channel increment hydration; render_scope remains fail-closed until
        // that lifecycle writes and reads back the current company/viewer row.
        self.start_scope_hydration(event.channel_id, out);

        if let Some(next_event) = next {
            self.queue_next_message_v3_event(next_event, out)?;
        }
        Ok(())
    }

    /// 按已冻结 Gate 语义继续处理刚变为连续的 MessageV3 缓冲事件。
    pub(super) fn queue_next_message_v3_event(
        &mut self,
        event: EventEnvelope,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let requires_viewer = matches!(
            event.kind,
            crate::sync_session::EventKind::PostUpsert | crate::sync_session::EventKind::PostEdit
        );
        if i64::try_from(event.seq.0).is_err()
            || (requires_viewer && self.config.auth_user_id.is_empty())
        {
            let channel_id = event.channel_id;
            if let Some(channel) = self.state.channels.get_mut(&channel_id) {
                channel.restore_message_v3_post(event, out);
            }
            return Err(ImError::Parse(
                "MessageV3 buffered event requires viewer identity and SQLite-range eventSeq"
                    .to_string(),
            ));
        }
        let corr = self.alloc_corr_internal();
        if crate::ws::handlers::channel_stream_event::requires_chain_post_hydration(&event) {
            return crate::ws::handlers::channel_stream_event::queue_or_hydrate_stream_commit(
                &mut self.state, corr, event, self.config.api_base_url.as_str(), out,
            );
        }
        if !requires_viewer {
            crate::ws::handlers::channel_stream_event::queue_stream_commit(
                &mut self.state,
                corr,
                event,
                out,
            );
            return Ok(());
        }
        if event.fields.msg_type == "CATEGORY_CHAIN"
            && matches!(event.kind, crate::sync_session::EventKind::PostEdit)
        {
            crate::category_chain::post::queue_edit(&mut self.state, corr, event, out);
            return Ok(());
        }
        if matches!(event.kind, crate::sync_session::EventKind::PostEdit)
            && crate::ws::handlers::post_update::has_quick_reply_items(
                event.fields.quick_reply.as_str(),
            )
        {
            super::message_v3_reaction::queue_commit(&mut self.state, corr, event, out);
            return Ok(());
        }
        if matches!(event.kind, crate::sync_session::EventKind::PostEdit)
            && !event.fields.expedite_map.is_empty()
        {
            super::message_v3_urgent::queue_commit(&mut self.state, corr, event, out);
            return Ok(());
        }
        if matches!(event.kind, crate::sync_session::EventKind::PostEdit)
            && crate::event::post::has_template_confirmation(event.fields.props.as_str())
        {
            super::message_v3_template::queue_commit(&mut self.state, corr, event, out);
            return Ok(());
        }
        if matches!(event.kind, crate::sync_session::EventKind::PostEdit) {
            queue_post_update_commit(
                &mut self.state,
                self.config.auth_user_id.as_str(),
                corr,
                event,
                out,
            );
            return Ok(());
        }
        queue_commit(
            &mut self.state,
            self.config.auth_user_id.as_str(),
            corr,
            event,
            out,
        )
    }

    /// G-01 复合键读回成功后只释放 post 与 channel 两个 MessageV3 终态。
    pub(super) fn handle_message_v3_post_readback_reply(
        &mut self,
        mut received_data: serde_json::Value,
        channel_id: crate::state::ChannelId,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let channel = match channel_update_from_member_readback(outcome) {
            Ok(Some(channel)) => channel,
            other => {
                if let Some(req_id) = causation_id.as_deref() {
                    self.fail_forward(req_id, "forward post readback failed", out)?;
                }
                return other.map(|_| ());
            }
        };
        self.diagnose(crate::diagnostics::Observation {
            event: "delivery_stage",
            stage: "persist_verified",
            path: "live_ws",
            result: "success",
            channel: channel_id.as_str(),
            count: 1,
            ..Default::default()
        });
        self.queue_message_v3_client_ack(&received_data, out)?;
        let category = received_data
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some("CATEGORY_CHAIN");
        // Only the final unique durable post completes a multi-item target.
        let forward_post_id = received_data["id"].as_str().unwrap_or_default().to_owned();
        if let Some(request_id) = causation_id.as_deref().and_then(|id| {
            self.state
                .pending_forward_deliveries
                .matching_request(id, channel_id)
                .filter(|req| {
                    self.state.pending_forward_deliveries.is_final_post(
                        req,
                        channel_id,
                        &forward_post_id,
                    )
                })
        }) {
            received_data["requestId"] = serde_json::json!(request_id);
        }
        let received = crate::event::post::received(received_data)?;
        if category {
            self.release_post_events(vec![received.into_bytes(), channel.into_bytes()], true, out)?;
        } else {
            out.push(received.into_effect());
            out.push(channel.into_effect());
        }
        if let Some(causation_id) = causation_id {
            self.complete_forward_post(&causation_id, channel_id, &forward_post_id, out);
        }
        Ok(())
    }

    /// Sync member 读回成功后只释放唯一 `im:channel:update` 绝对态。
    pub(super) fn handle_message_v3_sync_dialog_readback_reply(
        &mut self,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some(channel) = channel_update_from_member_readback(outcome)? {
            out.push(channel.into_effect());
        }
        Ok(())
    }

    /// 本地原子提交与 viewer 读回都成功后，发起带回报的真实客户端 ACK。
    fn queue_message_v3_client_ack(
        &mut self,
        received_data: &serde_json::Value,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let channel_id = received_data
            .get("channelId")
            .and_then(serde_json::Value::as_str)
            .and_then(crate::state::ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("client ACK missing channelId".to_string()))?;
        let post_id = received_data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ImError::Parse("client ACK missing post id".to_string()))?;
        let event_seq = received_data
            .get("eventSeq")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ImError::Parse("client ACK missing event seq".to_string()))?;
        let platform = self.config.client_platform;
        let body = serde_json::to_vec(&serde_json::json!({
            "postId": post_id,
            "ackId": format!("{post_id}:{event_seq}"),
            "platform": platform.as_str(),
        }))
        .map_err(|error| ImError::Parse(format!("client ACK body: {error}")))?;
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageV3ClientAck {
                channel_id,
                platform,
            },
        );
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        headers.extend(crate::acl::sync_http_effects::session_auth_headers(
            self.state.connection_id.as_deref(),
        ));
        out.push(Effect::Http {
            corr,
            req: HttpRequest {
                method: "POST".to_string(),
                url: format!("{}/post/clientAck", self.config.api_base_url),
                headers,
                body: Some(bytes::Bytes::from(body)),
            },
        });
        Ok(())
    }

    /// ACK 回报必须同时满足 transport 2xx 与 Go `status=SUCCESS` 才记录成功。
    pub(super) fn handle_message_v3_client_ack_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        platform: crate::module::ClientPlatform,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let succeeded = client_ack_succeeded(outcome);
        out.push(
            crate::event::post::client_ack_terminal(channel_id, platform, succeeded)?.into_effect(),
        );
        Ok(())
    }
}

/// 校验 ACK 的 HTTP 信封与 Go 业务状态，拒绝 transport-ok/business-failed 假成功。
fn client_ack_succeeded(outcome: &PortOutcome) -> bool {
    let PortOutcome::Ok(reply) = outcome else {
        return false;
    };
    let Ok(raw) =
        crate::http_envelope::unwrap_success_envelope(reply.0.as_ref(), "message client ACK")
    else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&raw)
        .ok()
        .and_then(|response| response.get("status").cloned())
        .and_then(|status| status.as_str().map(str::to_owned))
        .is_some_and(|status| status.eq_ignore_ascii_case("SUCCESS"))
}

#[cfg(test)]
mod buffered_event_tests {
    use super::*;
    use crate::state::Seq;
    use crate::sync_session::{EventKind, PostFields};

    #[test]
    fn buffered_post_read_uses_kind_aware_stream_commit() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id = crate::state::test_channel_id(91);
        module.register_channel(channel_id, 0);
        let event = EventEnvelope::new(
            channel_id,
            Seq(1),
            EventKind::PostRead,
            PostFields::default(),
        );
        let mut out = EffectSink::new();

        module.queue_next_message_v3_event(event, &mut out).unwrap();

        assert!(matches!(
            out.as_slice(),
            [Effect::PersistAtomic { ops, .. }]
                if matches!(ops.first(), Some(helix_core::effect::StorageOp::BatchUpdate(spec)) if spec.patch.is_empty())
                    && !ops.iter().any(|op| matches!(op, helix_core::effect::StorageOp::BatchUpsert(_)))
        ));
        assert!(module
            .state
            .corr_map
            .values()
            .any(|context| matches!(context, CorrelationContext::CanonicalStreamPersist { .. })));
        assert!(!module
            .state
            .corr_map
            .values()
            .any(|context| matches!(context, CorrelationContext::MessageV3PostPersist { .. })));
    }

    /// canonical stream 明确携带 readBits 时仍生成覆盖式单列写，不能误伤合法 type=6。
    #[test]
    fn buffered_post_read_with_explicit_bits_applies_read_patch() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id = crate::state::test_channel_id(92);
        module.register_channel(channel_id, 0);
        let mut fields = PostFields {
            id: "post-stream-read".to_string(),
            read_bits: "10".to_string(),
            ..PostFields::default()
        };
        fields.present_fields = crate::sync_session::POST_FIELD_READ_BITS;
        let event = EventEnvelope::new(channel_id, Seq(1), EventKind::PostRead, fields)
            .with_msg_id(Some("post-stream-read".to_string()));
        let mut out = EffectSink::new();

        module.queue_next_message_v3_event(event, &mut out).unwrap();

        assert!(matches!(
            out.as_slice(),
            [Effect::PersistAtomic { ops, .. }]
                if matches!(ops.first(), Some(helix_core::effect::StorageOp::BatchUpdate(spec))
                    if spec.patch.iter().any(|(column, value)| column == "read_bits"
                        && matches!(value, helix_core::effect::SqlValue::Text(bits) if bits == "10")))
        ));
    }
}

/// 从当前 viewer 的复合键读回构造唯一 channel 绝对态事件。
pub(super) fn channel_update_from_member_readback(
    outcome: &PortOutcome,
) -> Result<Option<crate::event::MessageV3Event>, ImError> {
    let PortOutcome::Ok(reply) = outcome else {
        return Ok(None);
    };
    let rows = helix_core::port_codec::rows_from_reply_bytes(&reply.0)
        .map_err(|error| ImError::Parse(format!("channel member readback: {error}")))?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let channel_id = text_column(row, "channel_id");
    let unread_count = integer_column(row, "unread_count");
    let last_post = text_column(row, "last_post")
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok());
    let (Some(channel_id), Some(unread_count), Some(last_post)) =
        (channel_id, unread_count, last_post)
    else {
        return Ok(None);
    };
    crate::event::channel::update(serde_json::json!({
        "channelId": channel_id,
        "lastPost": last_post,
        "unreadCount": unread_count,
    }))
    .map(Some)
}

/// 从 driver row 读取文本列，不接受隐式类型转换。
fn text_column<'a>(row: &'a helix_core::effect::Row, column: &str) -> Option<&'a str> {
    row.iter().find_map(|(name, value)| {
        (name == column)
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
    })
}

/// 从 driver row 读取整数列，不接受字符串数字别名。
fn integer_column(row: &helix_core::effect::Row, column: &str) -> Option<i64> {
    row.iter().find_map(|(name, value)| {
        (name == column)
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Integer(value) => Some(*value),
                _ => None,
            })
    })
}
