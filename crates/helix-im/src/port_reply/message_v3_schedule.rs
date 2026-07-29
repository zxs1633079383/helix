//! Schedule authority 的提交后读回与 creator-only 事件释放。

use crate::error::ImError;
use crate::module::ImModule;
use helix_core::effect::{GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    /// G-08 提交成功后记录 revision，并读取 creator 本地 schedule 绝对态。
    pub(super) fn handle_message_v3_schedule_created_persist_reply(
        &mut self,
        channel_id: crate::state::ChannelId,
        revision: u64,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if self
            .state
            .inflight_schedule_revisions
            .get(&channel_id)
            .is_some_and(|inflight| *inflight == revision)
        {
            self.state.inflight_schedule_revisions.remove(&channel_id);
        }
        let PortOutcome::Ok(_) = outcome else {
            tracing::warn!(
                channel_id = channel_id.as_str(),
                revision,
                "schedule authority persist failed; suppressing MessageV3 event"
            );
            return;
        };
        self.state
            .committed_schedule_revisions
            .insert(channel_id, revision);
        if causation_id.as_ref().is_some_and(|request_id| {
            self.state.pending_schedule_requests.get(&channel_id) == Some(request_id)
        }) {
            self.state.pending_schedule_requests.remove(&channel_id);
        }
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::MessageV3ScheduleCreatedReadback,
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "channel_schedule",
                key_col: "channel_id",
                key_val: SqlValue::Text(channel_id.as_str().to_string()),
            })],
        });
    }

    /// G-08 从 durable channel_schedule row 发布唯一 creator 终态。
    pub(super) fn handle_message_v3_schedule_created_readback_reply(
        &mut self,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            return Ok(());
        };
        let rows = helix_core::port_codec::rows_from_reply_bytes(&reply.0)
            .map_err(|error| ImError::Parse(format!("schedule readback: {error}")))?;
        let Some(row) = rows.first() else {
            return Ok(());
        };
        let Some(event) = crate::event::schedule::created_from_row(row)? else {
            return Ok(());
        };
        out.push(event.into_effect());
        Ok(())
    }
}
