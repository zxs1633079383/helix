use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::ChannelId;

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
            // NS-1：increment_fetched 是 HashSet，迭代序逐进程随机。resync 逐 channel 单调
            // alloc_corr + push 有序 Http Effect，必须确定性排序锚定可回放（GOAL §1，ChannelId: Ord）。
            let mut targets: Vec<ChannelId> = ctx
                .state
                .increment_fetched
                .iter()
                .filter(|id| !ctx.state.need_sync_skip.contains(*id))
                .copied()
                .collect();
            targets.sort_unstable();
            emit_proactive_resync_for(ctx, &targets, out);
            heal_incomplete_backfill_once(ctx, out);
        }
    }
}

/// B4：proactive resync 入口——不再逐 channel 直发，改为入 SyncScheduler 全局并发窗口
/// （`pending_sync` VecDeque + 在途上限 K），解 257 channel 重连风暴。targets 由调用方确定性
/// 排序锚定（HX-C010）；超 K 的留队列，等 sync 回报释放窗口续发（见 module.rs SyncPull 回报 drain）。
pub(crate) fn emit_proactive_resync_for(
    ctx: &mut ImWsContext<'_>,
    targets: &[ChannelId],
    out: &mut EffectSink,
) {
    // alloc_corr 经 ctx 拆借透传（与 corr_map 单调分配器不变量同源，state.rs CorrelationContext）。
    let api_base_url = ctx.api_base_url;
    let (state, alloc) = ctx.split_state_alloc();
    crate::sync_scheduler::enqueue_and_drain(state, api_base_url, targets, alloc, out);
}

fn heal_incomplete_backfill_once(ctx: &mut ImWsContext<'_>, out: &mut EffectSink) {
    if ctx.state.backfill_healed {
        return;
    }
    ctx.state.backfill_healed = true;

    let mut stuck: Vec<ChannelId> = ctx
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
    // HX-C010：stuck 由 increment_fetched（HashSet）迭代收集 → SipHash 逐进程随机序。
    // emit_proactive_resync_for 逐 channel alloc 单调 corr + 插 corr_map（corr 是 load-bearing：
    // 路由 + inflight 守卫 + 回放断言），≥2 stuck channel 须确定性有序锚定可回放（ChannelId: Ord）。
    stuck.sort_unstable();
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
