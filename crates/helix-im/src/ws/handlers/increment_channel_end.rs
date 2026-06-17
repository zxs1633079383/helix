use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::{ChannelId, InflightSync};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const INCREMENT_CHANNEL_END_ACTION: &str = "increment_channel_end";

pub(crate) fn apply_increment_end(
    ctx: &mut ImWsContext<'_>,
    channel_id: Option<ChannelId>,
    out: &mut EffectSink,
) {
    match channel_id {
        Some(channel_id) => {
            out.push(crate::acl::to_effect::emit_channel_update(channel_id));
        }
        None => {
            let targets: Vec<ChannelId> = ctx
                .state
                .increment_fetched
                .iter()
                .filter(|id| !ctx.state.need_sync_skip.contains(*id))
                .copied()
                .collect();
            emit_proactive_resync_for(ctx, &targets, out);
            heal_incomplete_backfill_once(ctx, out);
        }
    }
}

pub(crate) fn emit_proactive_resync_for(
    ctx: &mut ImWsContext<'_>,
    targets: &[ChannelId],
    out: &mut EffectSink,
) {
    let conn_id = ctx.state.connection_id.clone();
    for &channel_id in targets {
        let from_seq = match ctx.state.channels.get(&channel_id) {
            Some(ch) if ch.inflight_sync.is_some() => continue,
            Some(ch) => ch.cursor.value(),
            None => continue,
        };
        let sync_corr = ctx.alloc_corr();
        ctx.state.corr_to_sync.insert(sync_corr, channel_id);
        if let Some(ch) = ctx.state.channels.get_mut(&channel_id) {
            ch.inflight_sync = Some(InflightSync(sync_corr));
        }
        out.push(crate::acl::to_effect::sync_notify(
            ctx.api_base_url,
            channel_id,
            from_seq,
            sync_corr,
            conn_id.as_deref(),
        ));
    }
}

fn heal_incomplete_backfill_once(ctx: &mut ImWsContext<'_>, out: &mut EffectSink) {
    if ctx.state.backfill_healed {
        return;
    }
    ctx.state.backfill_healed = true;

    let stuck: Vec<ChannelId> = ctx
        .state
        .increment_fetched
        .iter()
        .filter_map(|&id| {
            let target = ctx.state.increment_target.get(&id).copied()?;
            let cursor = ctx.state.channels.get(&id)?.cursor.value();
            (cursor < target).then_some(id)
        })
        .collect();

    if stuck.is_empty() {
        return;
    }
    tracing::info!(
        stuck_channels = stuck.len(),
        "helix-im B2: 冷启动自愈——对 local cursor 落后服务端 increment 水位的 channel 补发 sync"
    );
    emit_proactive_resync_for(ctx, &stuck, out);
}

fn parse_channel_id(frame: &WsFrame) -> Option<ChannelId> {
    // Preserve the existing classifier contract: missing or invalid channelId is
    // treated as a global increment end for this migration phase.
    frame
        .data()
        .and_then(|data| data.get("channelId"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
}

struct IncrementChannelEndHandler;

impl WsMessageHandler for IncrementChannelEndHandler {
    fn action(&self) -> &'static str {
        INCREMENT_CHANNEL_END_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        apply_increment_end(ctx, parse_channel_id(frame), out);
        Ok(())
    }
}

static INCREMENT_CHANNEL_END_HANDLER: IncrementChannelEndHandler = IncrementChannelEndHandler;

#[distributed_slice(WS_HANDLERS)]
static INCREMENT_CHANNEL_END_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: INCREMENT_CHANNEL_END_ACTION,
    handler: &INCREMENT_CHANNEL_END_HANDLER,
};
