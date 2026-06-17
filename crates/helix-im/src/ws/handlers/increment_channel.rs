use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::channel::Channel;
use crate::error::ImError;
use crate::state::Seq;
use crate::sync_session::IncrementChannel;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const INCREMENT_CHANNEL_ACTION: &str = "increment_channel";

pub(crate) fn apply_increment(
    ctx: &mut ImWsContext<'_>,
    inc: &IncrementChannel,
    out: &mut EffectSink,
) {
    let channel_id = inc.channel_id;
    ctx.state
        .channels
        .entry(channel_id)
        .or_insert_with(|| Channel::new(channel_id, inc.last_event_seq.0));

    ctx.state.increment_fetched.insert(channel_id);
    if !inc.need_sync {
        ctx.state.need_sync_skip.insert(channel_id);
    }

    let target = ctx
        .state
        .increment_target
        .entry(channel_id)
        .or_insert(Seq(0));
    if inc.last_event_seq > *target {
        *target = inc.last_event_seq;
    }

    out.push(crate::acl::to_effect::emit_channel_increment(
        channel_id,
        inc.raw.as_ref(),
    ));
}

struct IncrementChannelHandler;

impl WsMessageHandler for IncrementChannelHandler {
    fn action(&self) -> &'static str {
        INCREMENT_CHANNEL_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(inc) = crate::parser::parse_increment_channel(data) else {
            return Ok(());
        };

        apply_increment(ctx, &inc, out);
        Ok(())
    }
}

static INCREMENT_CHANNEL_HANDLER: IncrementChannelHandler = IncrementChannelHandler;

#[distributed_slice(WS_HANDLERS)]
static INCREMENT_CHANNEL_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: INCREMENT_CHANNEL_ACTION,
    handler: &INCREMENT_CHANNEL_HANDLER,
};
