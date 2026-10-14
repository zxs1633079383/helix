//! `post_schedule_canceled` action handler（排程消息撤：channel has_schedule_post=0，spec §S1）。
//!
//! 行为真源：现网 `handlers/schedule.rs:35 handle_canceled` → `Channel::update_fields(
//! has_schedule_post=false)` → emit `im:channel:schedule-canceled{channelId, hasSchedulePost:false}`。
//!
//! wire（`SchedulePostData` types.rs:367）：`channelId`。helix：定点列 patch
//! （`channel_set_cols`：UPDATE channel SET has_schedule_post=0 WHERE id=?）。缺 channelId → noop。

use helix_core::{Effect, EffectSink};

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const SCHEDULE_CANCELED_ACTION: &str = "post_schedule_canceled";

struct ScheduleCanceledHandler;

impl WsMessageHandler for ScheduleCanceledHandler {
    /// 返回该 handler 消费的唯一 G09 authority action。
    fn action(&self) -> &'static str {
        SCHEDULE_CANCELED_ACTION
    }

    /// 将 G09 cancel authority 放入原子持久屏障，成功前不发布终态事件。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(fact) = crate::channel::write::ScheduleFact::from_canceled_ws(
            data,
            frame.event_seq().map(|seq| seq.0),
        ) else {
            return Ok(());
        };
        let requested_by_viewer = ctx
            .state
            .pending_schedule_cancel_requests
            .contains_key(&fact.channel_id);
        if !requested_by_viewer
            && (fact.owner_user_id.is_empty() || fact.owner_user_id != ctx.auth_user_id)
        {
            return Ok(());
        }
        let committed = ctx
            .state
            .committed_schedule_revisions
            .get(&fact.channel_id)
            .copied()
            .unwrap_or(0);
        let inflight = ctx
            .state
            .inflight_schedule_revisions
            .get(&fact.channel_id)
            .copied()
            .unwrap_or(0);
        if fact.revision <= committed.max(inflight) {
            return Ok(());
        }
        let channel_id = fact.channel_id;
        let revision = fact.revision;
        let causation_id = ctx
            .state
            .pending_schedule_cancel_requests
            .get(&channel_id)
            .cloned();
        let corr = ctx.alloc_corr();
        ctx.state
            .inflight_schedule_revisions
            .insert(channel_id, revision);
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ScheduleCanceledPersist {
                channel_id,
                revision,
                causation_id,
            },
        );
        let mut ops = fact.storage_ops();
        // A canceled schedule invalidates any independent multipart intent on
        // this channel.  Keep this delete in the same authority checkpoint;
        // the in-memory route is released only after its PersistOk.
        ops.push(crate::schedule_media::schedule_intent_delete(channel_id));
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }
}

static SCHEDULE_CANCELED_HANDLER: ScheduleCanceledHandler = ScheduleCanceledHandler;
#[cfg(target_arch = "wasm32")]
/// 在 wasm inventory 不可自动发现时保留静态 handler。
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&SCHEDULE_CANCELED_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: SCHEDULE_CANCELED_ACTION,
        handler: &SCHEDULE_CANCELED_HANDLER,
    }
}
