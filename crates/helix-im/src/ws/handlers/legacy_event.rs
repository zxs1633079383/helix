use helix_core::EffectSink;

use crate::error::ImError;

use super::super::{
    ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, LEGACY_EVENT_ACTION,
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
        match crate::ws::parser::parse_inbound_value(frame.root()) {
            Ok(ev) => {
                let (ingest_channel, ingest_seq) = (ev.channel_id, ev.seq);
                let mut timeline_readback_required = false;
                if let Some(ch) = ctx.state.channels.get_mut(&ev.channel_id) {
                    let recent_start = out.as_slice().len();
                    ch.ingest(ev, out, ctx.now_ms)?;
                    timeline_readback_required = super::gate::recent_effects_persist_message(
                        &out.as_slice()[recent_start..],
                    );
                    super::gate::schedule_reload_if_recent_too_long(
                        ctx,
                        ingest_channel,
                        recent_start,
                        out,
                    )?;
                }
                if timeline_readback_required {
                    ctx.request_attached_timeline_refresh(ingest_channel);
                }
                // 根治：legacy 内容帧大缺口同样立即 backfill 追平 cursor。
                super::gate::trigger_backfill_if_large_gap(ctx, ingest_channel, ingest_seq, out);
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
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&LEGACY_EVENT_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: LEGACY_EVENT_ACTION,
        handler: &LEGACY_EVENT_HANDLER,
    }
}
