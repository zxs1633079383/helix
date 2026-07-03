use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::{ChannelId, ConnState, Seq};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const HELLO_ACTION: &str = "hello";

struct HelloHandler;

impl WsMessageHandler for HelloHandler {
    fn action(&self) -> &'static str {
        HELLO_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(connection_id) = frame
            .data()
            .and_then(|data| data.get("connectionId"))
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(());
        };

        ctx.state.conn = ConnState::Connected;
        ctx.state.connection_id = Some(connection_id.to_string());
        ctx.state.reset_increment_batch();
        out.push(crate::acl::to_effect::emit_connection_established(
            connection_id,
        ));

        let mut cursors: Vec<(ChannelId, Seq)> = ctx
            .state
            .channels
            .iter()
            .map(|(&id, ch)| (id, ch.cursor.value()))
            .collect();
        cursors.sort_unstable_by_key(|(id, _)| *id);
        ctx.state.pending_increment_bootstrap_after_scan = cursors.is_empty();
        let inc_corr = ctx.alloc_corr();
        out.push(crate::acl::to_effect::increment_http_trigger(
            ctx.api_base_url,
            &cursors,
            inc_corr,
            Some(connection_id),
        ));
        out.push(crate::acl::to_effect::emit_channels_loaded());
        tracing::info!("helix-im: hello handshake complete, increment HTTP triggered");

        Ok(())
    }
}

static HELLO_HANDLER: HelloHandler = HelloHandler;

#[distributed_slice(WS_HANDLERS)]
static HELLO_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: HELLO_ACTION,
    handler: &HELLO_HANDLER,
};
