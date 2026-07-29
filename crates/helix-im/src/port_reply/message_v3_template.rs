//! G-14a 模板 props 的原子提交后读回与终态释放。

use crate::error::ImError;
use crate::module::ImModule;
use helix_core::effect::{GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Effect, EffectSink};

/// 将连续 G-14a authority 绑定到 props/cursor 原子提交。
pub(crate) fn queue_commit(
    state: &mut crate::state::ImState,
    corr: Correlation,
    event: crate::sync_session::EventEnvelope,
    out: &mut EffectSink,
) {
    let message_id = event
        .msg_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .unwrap_or(event.fields.id.as_str());
    let template_op = crate::channel::write::message_v3_template_op(
        message_id,
        event.fields.props.clone(),
        event.seq.0,
        event.fields.update_at,
    );
    let cursor_op = crate::acl::to_effect::advance_cursor_op(event.channel_id, event.seq);
    state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::MessageV3TemplatePersist {
            event: Box::new(event),
        },
    );
    out.push(Effect::PersistAtomic {
        corr,
        ops: vec![template_op, cursor_op],
    });
}

impl ImModule {
    /// 提交模板 props/cursor 后推进内存 cursor，并读取 durable message 绝对态。
    pub(super) fn handle_message_v3_template_persist_reply(
        &mut self,
        event: crate::sync_session::EventEnvelope,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            if let Some(channel) = self.state.channels.get_mut(&event.channel_id) {
                channel.restore_message_v3_post(event, out);
            }
            return Ok(());
        }
        let message_id = event
            .msg_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(event.fields.id.as_str())
            .to_string();
        let next = self
            .state
            .channels
            .get_mut(&event.channel_id)
            .and_then(|channel| channel.commit_message_v3_post(event.seq));
        self.state
            .invalidate_recent_message_coverage(event.channel_id);
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::MessageV3TemplateReadback,
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "message",
                key_col: "id",
                key_val: SqlValue::Text(message_id),
            })],
        });
        if let Some(next_event) = next {
            self.queue_next_message_v3_event(next_event, out)?;
        }
        Ok(())
    }

    /// 从 durable message row 发布唯一模板确认终态。
    pub(super) fn handle_message_v3_template_readback_reply(
        &mut self,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            return Ok(());
        };
        let rows = helix_core::port_codec::rows_from_reply_bytes(&reply.0)
            .map_err(|error| ImError::Parse(format!("template readback: {error}")))?;
        let Some(row) = rows.first() else {
            return Ok(());
        };
        let Some(event) = crate::event::post::template_update_from_row(row)? else {
            return Ok(());
        };
        out.push(event.into_effect());
        Ok(())
    }
}
