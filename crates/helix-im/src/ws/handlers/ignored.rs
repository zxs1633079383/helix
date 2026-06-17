use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const STATUS_CHANGE_ACTION: &str = "status_change";
const MULTIPLE_CHANNELS_VIEWED_ACTION: &str = "multiple_channels_viewed";
const TYPING_ACTION: &str = "typing";
const PRESENCE_ACTION: &str = "presence";

struct IgnoredHandler {
    action: &'static str,
}

impl WsMessageHandler for IgnoredHandler {
    fn action(&self) -> &'static str {
        self.action
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        Ok(())
    }
}

static STATUS_CHANGE_HANDLER: IgnoredHandler = IgnoredHandler {
    action: STATUS_CHANGE_ACTION,
};
static MULTIPLE_CHANNELS_VIEWED_HANDLER: IgnoredHandler = IgnoredHandler {
    action: MULTIPLE_CHANNELS_VIEWED_ACTION,
};
static TYPING_HANDLER: IgnoredHandler = IgnoredHandler {
    action: TYPING_ACTION,
};
static PRESENCE_HANDLER: IgnoredHandler = IgnoredHandler {
    action: PRESENCE_ACTION,
};

#[distributed_slice(WS_HANDLERS)]
static STATUS_CHANGE_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: STATUS_CHANGE_ACTION,
    handler: &STATUS_CHANGE_HANDLER,
};

#[distributed_slice(WS_HANDLERS)]
static MULTIPLE_CHANNELS_VIEWED_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: MULTIPLE_CHANNELS_VIEWED_ACTION,
    handler: &MULTIPLE_CHANNELS_VIEWED_HANDLER,
};

#[distributed_slice(WS_HANDLERS)]
static TYPING_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: TYPING_ACTION,
    handler: &TYPING_HANDLER,
};

#[distributed_slice(WS_HANDLERS)]
static PRESENCE_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: PRESENCE_ACTION,
    handler: &PRESENCE_HANDLER,
};
