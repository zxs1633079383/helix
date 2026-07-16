use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

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
            // NS-1 + G-S1：输入帧序是可回放事实，且 Go 以最近活跃优先交付。
            // 用 Vec 保序、HashSet O(1) 去重，避免重新按 channelId 排序后把新频道推到历史洪峰末尾。
            // 直接构造 state 的旧测试/恢复态可能没有 increment_order；剩余 fetched 以 ID 升序补齐，
            // 仍不读取 HashSet 的随机迭代序作为输出顺序。
            let mut targets = Vec::with_capacity(ctx.state.increment_fetched.len());
            let mut seen =
                std::collections::HashSet::with_capacity(ctx.state.increment_fetched.len());
            for &channel_id in &ctx.state.increment_order {
                if ctx.state.increment_fetched.contains(&channel_id)
                    && !ctx.state.need_sync_skip.contains(&channel_id)
                    && seen.insert(channel_id)
                {
                    targets.push(channel_id);
                }
            }
            let mut unordered_tail: Vec<ChannelId> = ctx
                .state
                .increment_fetched
                .iter()
                .filter(|id| !ctx.state.need_sync_skip.contains(*id) && !seen.contains(*id))
                .copied()
                .collect();
            unordered_tail.sort_unstable();
            for channel_id in unordered_tail {
                if seen.insert(channel_id) {
                    targets.push(channel_id);
                }
            }
            for channel_id in collect_incomplete_backfill_targets(ctx) {
                if seen.insert(channel_id) {
                    targets.push(channel_id);
                }
            }
            emit_proactive_resync_for(ctx, &targets, out);

            // UC-10 与既有时序一致：global end 触发查询；其业务完成仍由 TodoQuery HTTP 回执决定。
            let todo_post_ids = std::mem::take(&mut ctx.state.about_me_post_ids);
            trigger_todo_query(ctx, &todo_post_ids, out);

            // G-S1：global end 只封口输入批次，不能自己晋升 ready。把全部 channel/member 写与
            // 当前已确认 cursor 合成一个相关事务；只有 PortReply Ok 才发布 increment/loaded。
            // sync/todo 自有 HTTP 回执状态机，不等价于最终投影完成。
            let mut ops = std::mem::take(&mut ctx.state.pending_increment_ops);
            let projections = std::mem::take(&mut ctx.state.pending_increment_projections);
            let mut cursor_channels: Vec<ChannelId> =
                ctx.state.increment_fetched.iter().copied().collect();
            cursor_channels.sort_unstable();
            for channel_id in cursor_channels {
                if let Some(channel) = ctx.state.channels.get(&channel_id) {
                    ops.push(crate::acl::to_effect::advance_cursor_op(
                        channel_id,
                        channel.cursor.value(),
                    ));
                }
            }
            if ops.is_empty() && projections.is_empty() {
                return;
            }
            let corr = ctx.alloc_corr();
            out.push(Effect::Persist { corr, ops });
            ctx.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::IncrementBatchPersist { projections },
            );
        }
    }
}

/// UC-10 待办触发（global increment-end 收尾）：缓冲非空 → build `posts/queryTodoList {postIds}`
/// HTTP Effect + 注册 `CorrelationContext::TodoQuery`（回报装配 emit），随后**清空缓冲**。
///
/// 缓冲空 → no-op（无待办可拉）。HTTP body/headers 与 outbound 写命令同构（Content-Type +
/// connectionId + session auth intent）；身份头复用 `session_auth_headers`（DRY，HX-C006）。
fn trigger_todo_query(ctx: &mut ImWsContext<'_>, post_ids: &[String], out: &mut EffectSink) {
    if post_ids.is_empty() {
        return;
    }
    let body = crate::todo::query_todo_body(post_ids);

    let corr = ctx.alloc_corr();
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::sync_http_effects::session_auth_headers(
        ctx.state.connection_id.as_deref(),
    ));
    out.push(helix_core::effect::Effect::Http {
        corr,
        req: helix_core::effect::HttpRequest {
            method: "POST".to_string(),
            url: format!("{}/posts/queryTodoList", ctx.api_base_url),
            headers,
            body: Some(bytes::Bytes::from(
                serde_json::to_vec(&body).unwrap_or_default(),
            )),
        },
    });
    // 注册回报上下文：HTTP reply → 装配 emit im:todo:updated（module.rs handle_port_reply）。
    ctx.state
        .corr_map
        .insert(corr, crate::state::CorrelationContext::TodoQuery);
    tracing::info!("helix-im UC-10: hello 收尾触发 queryTodoList（待办内容拉取）");
}

/// G-S1 事务成功后的续步：发布最终投影。调用者只能是
/// `IncrementBatchPersist` 的成功回执分支。
pub(crate) fn after_increment_batch_persist(
    projections: Vec<(ChannelId, Vec<u8>)>,
    out: &mut EffectSink,
) {
    for (channel_id, raw) in projections {
        out.push(crate::acl::to_effect::emit_channel_increment(
            channel_id,
            raw.as_ref(),
        ));
    }
    out.push(crate::acl::to_effect::emit_channels_loaded());
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

fn collect_incomplete_backfill_targets(ctx: &mut ImWsContext<'_>) -> Vec<ChannelId> {
    if ctx.state.backfill_healed {
        return Vec::new();
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
        return stuck;
    }
    // HX-C010：stuck 由 increment_fetched（HashSet）迭代收集 → SipHash 逐进程随机序。
    // emit_proactive_resync_for 逐 channel alloc 单调 corr + 插 corr_map（corr 是 load-bearing：
    // 路由 + inflight 守卫 + 回放断言），≥2 stuck channel 须确定性有序锚定可回放（ChannelId: Ord）。
    stuck.sort_unstable();
    tracing::info!(
        stuck_channels = stuck.len(),
        "helix-im B2: 冷启动自愈——对 local cursor 落后服务端 increment 水位的 channel 补发 sync"
    );
    stuck
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
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&INCREMENT_CHANNEL_END_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: INCREMENT_CHANNEL_END_ACTION,
        handler: &INCREMENT_CHANNEL_END_HANDLER,
    }
}
