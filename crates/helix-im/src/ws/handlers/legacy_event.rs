use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{
    ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, LEGACY_EVENT_ACTION, WS_HANDLERS,
};

struct LegacyEventHandler;

impl WsMessageHandler for LegacyEventHandler {
    fn action(&self) -> &'static str {
        LEGACY_EVENT_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 复用 WsFrame::parse 已解析的 root Value，消除第二趟 from_slice（重构 A「痛点 1」）。
        match crate::parser::parse_inbound_value(frame.root()) {
            Ok(ev) => {
                if let Some(ch) = ctx.state.channels.get_mut(&ev.channel_id) {
                    ch.ingest(ev, out, ctx.now_ms)?;
                }
                Ok(())
            }
            Err(ImError::Parse(msg)) => {
                tracing::debug!("im: inbound parse error: {}", msg);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

static LEGACY_EVENT_HANDLER: LegacyEventHandler = LegacyEventHandler;

#[distributed_slice(WS_HANDLERS)]
static LEGACY_EVENT_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: LEGACY_EVENT_ACTION,
    handler: &LEGACY_EVENT_HANDLER,
};
