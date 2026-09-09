//! Schedule authority 的提交后读回与 creator-only 事件释放。

use crate::error::ImError;
use crate::module::ImModule;
use helix_core::effect::{GetSpec, Row, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

/// durable `channel_schedule` 行是否仍承载一条**待发**定时消息。
///
/// 判据只看真实列，不看「这是哪条链的读回」：创建链写 `schedule_id` 非空 + 服务端 status
/// （缺省 `scheduled`），取消链把 `schedule_id` 清空并把 status 写成 `canceled`
/// （`channel/write/schedule.rs` 的 `from_created_ws` / `from_canceled_ws`）。
///
/// 存在的意义是**方向守卫**：两条读回都发绝对频道投影，而 `hasSchedulePost` 是由发布者
/// 单方面写死的（created→true / canceled→false）。若读回落在方向相反的行上——例如取消提交后
/// 读回被同频道新建的行抢先，或迟到的创建读回落在已取消的行上——照发就会产出与 durable 事实
/// 相反的绝对态：前者把仍会到点的定时消息的提示条抹掉（用户以为没设上 → 重设 → 到点发两条），
/// 后者让已取消的提示条复活（MV3-G04b 负向断言「不得复活」）。所以方向不符时一律抑制并告警。
fn row_carries_an_active_schedule(row: &Row) -> bool {
    let schedule_id = row_text(row, "schedule_id").unwrap_or_default();
    let status = row_text(row, "status").unwrap_or_default();
    !schedule_id.is_empty() && status != "canceled"
}

/// 读取文本列；非文本列一律视为缺失（不做隐式类型转换，与 `event::schedule` 同口径）。
fn row_text<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    row.iter().find_map(|(name, value)| match value {
        SqlValue::Text(value) if name == column => Some(value.as_str()),
        _ => None,
    })
}

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
        // 请求关联只消费一次；失败同样释放，重放 authority 仍可独立恢复持久事实。
        let causation_id = causation_id.filter(|request_id| {
            self.state.pending_schedule_requests.get(&channel_id) == Some(request_id)
        });
        if causation_id.is_some() {
            self.state.pending_schedule_requests.remove(&channel_id);
        }
        let PortOutcome::Ok(_) = outcome else {
            if let Some(req_id) = causation_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "SCHEDULE_PERSIST_FAILED",
                ));
            }
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
        if let Some(req_id) = causation_id.as_deref() {
            out.push(crate::read_relay::emit_read_body(
                req_id,
                serde_json::json!({"ok": true}),
            ));
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
            tracing::warn!(
                "schedule created readback returned no durable row; terminal projection suppressed"
            );
            return Ok(());
        };
        if !row_carries_an_active_schedule(row) {
            tracing::warn!(
                "schedule created readback landed on a canceled/empty durable row; \
                 hasSchedulePost=true projection suppressed"
            );
            return Ok(());
        }
        let Some(event) = crate::event::schedule::created_from_row(row)? else {
            return Ok(());
        };
        out.push(event.into_effect());
        Ok(())
    }

    /// G-09 提交成功后记录 terminal revision，并读取 creator 本地 canceled 绝对态。
    pub(super) fn handle_message_v3_schedule_canceled_persist_reply(
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
        // 请求关联只消费一次；失败同样释放，重放 authority 仍可独立恢复持久事实。
        let causation_id = causation_id.filter(|request_id| {
            self.state.pending_schedule_cancel_requests.get(&channel_id) == Some(request_id)
        });
        if causation_id.is_some() {
            self.state
                .pending_schedule_cancel_requests
                .remove(&channel_id);
        }
        let PortOutcome::Ok(_) = outcome else {
            if let Some(req_id) = causation_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "SCHEDULE_PERSIST_FAILED",
                ));
            }
            tracing::warn!(
                channel_id = channel_id.as_str(),
                revision,
                "schedule cancel persist failed; suppressing MessageV3 event"
            );
            return;
        };
        self.state
            .committed_schedule_revisions
            .insert(channel_id, revision);
        if let Some(req_id) = causation_id.as_deref() {
            out.push(crate::read_relay::emit_read_body(
                req_id,
                serde_json::json!({"ok": true}),
            ));
        }
        let corr = self.alloc_corr_internal();
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::MessageV3ScheduleCanceledReadback,
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

    /// G-09 / MV3-G04b 从 durable channel_schedule row 发布唯一 creator 取消终态。
    ///
    /// 事件是**绝对频道投影**（`channelId` + `hasSchedulePost=false` + `status` + `revision`），
    /// 由真实读回行构造而非由取消请求回显——所以「取消后仍能被 MV3-G04d 读到」不可能发生：
    /// 事件与 durable 行同源。读回缺行时不发事件（提示条不会被凭空清掉），但必须留告警，
    /// 否则一次静默丢行会表现为「删除定时后提示条不消失」且无任何线索。
    pub(super) fn handle_message_v3_schedule_canceled_readback_reply(
        &mut self,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            tracing::warn!("schedule cancel readback failed; suppressing MessageV3 event");
            return Ok(());
        };
        let rows = helix_core::port_codec::rows_from_reply_bytes(&reply.0)
            .map_err(|error| ImError::Parse(format!("schedule cancel readback: {error}")))?;
        let Some(row) = rows.first() else {
            tracing::warn!("schedule cancel readback returned no durable row; MV3-G04b terminal projection suppressed");
            return Ok(());
        };
        if row_carries_an_active_schedule(row) {
            tracing::warn!(
                "schedule cancel readback landed on an active durable row; \
                 hasSchedulePost=false projection suppressed"
            );
            return Ok(());
        }
        let Some(event) = crate::event::schedule::canceled_from_row(row)? else {
            return Ok(());
        };
        out.push(event.into_effect());
        Ok(())
    }
}
