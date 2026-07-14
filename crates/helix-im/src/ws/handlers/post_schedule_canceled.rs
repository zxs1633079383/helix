//! `post_schedule_canceled` action handler（排程消息撤：channel has_schedule_post=0，spec §S1）。
//!
//! 行为真源：现网 `handlers/schedule.rs:35 handle_canceled` → `Channel::update_fields(
//! has_schedule_post=false)` → emit `im:channel:schedule-canceled{channelId, hasSchedulePost:false}`。
//!
//! wire（`SchedulePostData` types.rs:367）：`channelId`。helix：定点列 patch
//! （`channel_set_cols`：UPDATE channel SET has_schedule_post=0 WHERE id=?）。缺 channelId → noop。

use helix_core::effect::SqlValue;
use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const SCHEDULE_CANCELED_ACTION: &str = "post_schedule_canceled";

struct ScheduleCanceledHandler;

impl WsMessageHandler for ScheduleCanceledHandler {
    fn action(&self) -> &'static str {
        SCHEDULE_CANCELED_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        out.push(crate::acl::to_effect_s1::channel_set_cols(
            channel_id,
            vec![("has_schedule_post", SqlValue::Integer(0))],
        ));
        out.push(crate::acl::to_effect_s1::emit_schedule_canceled(channel_id));
        Ok(())
    }
}

static SCHEDULE_CANCELED_HANDLER: ScheduleCanceledHandler = ScheduleCanceledHandler;

inventory::submit! {
    WsHandlerRegistration {
        action: SCHEDULE_CANCELED_ACTION,
        handler: &SCHEDULE_CANCELED_HANDLER,
    }
}
