//! G-01 MessageV3 post 的原子提交、复合键读回与双事件释放。

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{CorrelationContext, ImState};
use crate::sync_session::EventEnvelope;
use helix_core::effect::{Correlation, Effect, SqlValue};
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
            },
        );

        if let Some(next_event) = next {
            let next_corr = self.alloc_corr_internal();
            queue_commit(
                &mut self.state,
                self.config.auth_user_id.as_str(),
                next_corr,
                next_event,
                out,
            )?;
        }
        Ok(())
    }

    /// G-01 复合键读回成功后只释放 post 与 channel 两个 MessageV3 终态。
    pub(super) fn handle_message_v3_post_readback_reply(
        &mut self,
        received_data: serde_json::Value,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            return Ok(());
        };
        let rows = helix_core::port_codec::rows_from_reply_bytes(&reply.0)
            .map_err(|error| ImError::Parse(format!("G-01 member readback: {error}")))?;
        let Some(row) = rows.first() else {
            return Ok(());
        };
        let channel_id = text_column(row, "channel_id");
        let unread_count = integer_column(row, "unread_count");
        let last_post = text_column(row, "last_post")
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok());
        let (Some(channel_id), Some(unread_count), Some(last_post)) =
            (channel_id, unread_count, last_post)
        else {
            return Ok(());
        };
        let received = crate::event::post::received(received_data)?;
        let channel = crate::event::channel::update(serde_json::json!({
            "channelId": channel_id,
            "lastPost": last_post,
            "unreadCount": unread_count,
        }))?;
        out.push(received.into_effect());
        out.push(channel.into_effect());
        Ok(())
    }
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
