//! `post_schedule_created` action handler（排程消息建：channel has_schedule_post=1，spec §S1）。
//!
//! 行为真源：现网 `handlers/schedule.rs:11 handle_created` → `Channel::update_fields(
//! has_schedule_post=true)` → emit `im:channel:schedule-created{channelId, hasSchedulePost:true}`。
//!
//! wire（`SchedulePostData` types.rs:367）：`channelId`。helix：定点列 patch
//! （`channel_set_cols`：UPDATE channel SET has_schedule_post=1 WHERE id=?）。缺 channelId → noop。

use helix_core::{Effect, EffectSink};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};
use crate::error::ImError;

const SCHEDULE_CREATED_ACTION: &str = "post_schedule_created";

struct ScheduleCreatedHandler;

impl WsMessageHandler for ScheduleCreatedHandler {
    fn action(&self) -> &'static str {
        SCHEDULE_CREATED_ACTION
    }

    /// 将 G08 authority 放入原子持久屏障，禁止直接发布 raw 或终态事件。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(fact) = crate::channel::write::ScheduleFact::from_created_ws(
            data,
            frame.event_seq().map(|seq| seq.0),
        ) else {
            return Ok(());
        };
        if fact.owner_user_id.is_empty() || fact.owner_user_id != ctx.auth_user_id {
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
            .pending_schedule_requests
            .get(&channel_id)
            .cloned();
        let has_multipart_intent = ctx
            .state
            .schedule_media_submitting
            .contains_key(&channel_id);
        let corr = ctx.alloc_corr();
        ctx.state
            .inflight_schedule_revisions
            .insert(channel_id, revision);
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ScheduleCreatedPersist {
                channel_id,
                revision,
                causation_id,
            },
        );
        let mut ops = fact.storage_ops();
        // A multipart schedule owns an independent intent until this
        // authority write succeeds. Delete it in the same atomic checkpoint
        // so a restart cannot resume an already-created schedule.
        if has_multipart_intent {
            ops.push(crate::schedule_media::schedule_intent_delete(channel_id));
        }
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }
}

static SCHEDULE_CREATED_HANDLER: ScheduleCreatedHandler = ScheduleCreatedHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&SCHEDULE_CREATED_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: SCHEDULE_CREATED_ACTION,
        handler: &SCHEDULE_CREATED_HANDLER,
    }
}
