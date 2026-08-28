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

impl ImModule {
    /// G-01 原子提交成功后推进 cursor，并发起当前 viewer 复合键读回。
    pub(super) fn handle_message_v3_post_persist_reply(
        &mut self,
        event: EventEnvelope,
        received_data: serde_json::Value,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
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
        let corr = self.alloc_corr_internal();
        if matches!(event.kind, crate::sync_session::EventKind::PostEdit)
            && !event.fields.quick_reply.is_empty()
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
        received_data: serde_json::Value,
        channel_id: crate::state::ChannelId,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(channel) = channel_update_from_member_readback(outcome)? else {
            return Ok(());
        };
        self.queue_message_v3_client_ack(&received_data, out)?;
        let received = crate::event::post::received(received_data)?;
        out.push(received.into_effect());
        out.push(channel.into_effect());
        if let Some(causation_id) = causation_id {
            self.state
                .pending_forward_deliveries
                .complete_target(&causation_id, channel_id);
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
        self.state
            .corr_map
            .insert(corr, CorrelationContext::MessageV3ClientAck { platform });
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
        platform: crate::module::ClientPlatform,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let succeeded = client_ack_succeeded(outcome);
        out.push(crate::event::post::client_ack_terminal(platform, succeeded)?.into_effect());
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
