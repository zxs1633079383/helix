//! 共享 MessageV3 gate 的 gap backfill、超长 recent reload 与 resync 调度。
//!
//! ## 为什么必须走 gate（严格 +1），不能用 cursor MAX-jump
//!
//! 现网真源 `handlers/post.rs:457 on_ws_event` 是**所有**带 event_seq 的 WS 事件
//! （post / post_update / posts_update / post_read）的**唯一 gate 入口**：
//!   - seq == cursor+1 → apply（Case1）
//!   - seq <= cursor   → drop（Case2，幂等）
//!   - seq > cursor+1  → buffer + arm 1s gate（Case3，**不推 cursor**）
//!
//! 旧实现（`try_advance` MAX-jump）的致命缺陷：两条 edit/read 帧之间漏掉的 **post（新消息）**
//! 造成的 seq 空洞，会被后到的 edit/read 帧 MAX-jump 直接跨过 → cursor 越过缺口 → sync 永不
//! 回拉该缺失 post → **消息丢失且不可恢复**（违 spec §S6「WS 帧 flush_contiguous 严格 +1」）。
//!
//! 走 gate 后：缺口事件进 buffer + arm gate，1s 后触发 sync/notify 回拉缺口，缺口填上才连续
//! flush——与 `post` handler 的 `ch.ingest` 完全同口径，cursor 永不跨空洞。
//!
//! 具体业务事件的持久化与发布由各自 correlation owner 完成；本文件不再提供可绕过
//! matching PersistOk 的内容事件写入或提前 Emit。

use helix_core::effect::StorageOp;
use helix_core::{Effect, EffectSink};

use super::super::ImWsContext;
use crate::error::ImError;
use crate::state::{ChannelId, Seq};

/// 大缺口立即 backfill 的阈值（gap = seq − cursor）。
///
/// 超过此阈值 → 判定为「活动频道 cursor 真落后服务端 high-water、需 backfill」，立即触发
/// per-channel proactive resync（不等 1s gate timer）。≤ 阈值的小缺口仍只走 1s gate timer，
/// 给 WS 自然乱序重排留窗口，避免对瞬时相邻乱序触发无谓 HTTP sync（回归保护）。
///
/// 取 2：gap > 2 ⇒ seq ≥ cursor+4（至少 3 条中间事件缺失）才算「真缺口」。echo gate-gap 整类
/// 实证 gap 极大（cursor=2 / server seq=22 ⇒ gap=20）；常规相邻乱序 gap=1。
const GAP_IMMEDIATE_RESYNC_THRESHOLD: u64 = 2;

/// True only when the current WS handler has appended an authoritative local
/// message-table mutation. Consumers use this to schedule a typed timeline
/// readback after the handler returns, rather than projecting raw WS content.
pub(super) fn recent_effects_persist_message(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| {
        let Effect::PersistFire { ops } = effect else {
            return false;
        };
        ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(spec) => spec.table == "message",
            StorageOp::BatchUpdate(spec) => spec.table == "message",
            StorageOp::MonotonicUpsert(spec) => spec.table == "message",
            StorageOp::GuardedBump(spec) => spec.table == "message",
            StorageOp::ScopedGuardedBump(spec) => spec.table == "message",
            StorageOp::Get(spec) => spec.table == "message",
            StorageOp::ScopedGet(spec) => spec.table == "message",
            StorageOp::Scan(spec) => spec.table == "message",
            StorageOp::BatchDelete(spec) => spec.table == "message",
        })
    })
}

/// 根治 echo gate-gap 整类：检测到**大缺口**即对该 channel **立即**触发 proactive resync。
///
/// ## 根因（HOP③ 逐跳实证）
///
/// 活动频道本地 cursor 落后服务端 high-water（如 cursor=2 / server seq=22），收到 echo
/// （seq≫cursor+1）→ `Channel::ingest` buffer + arm **1s** gate timer，cursor 不推进、buffered
/// echo 不 emit。cursor 仅在 1s timer fire 后发出的 `/channel/sync/notify` 回报 commit 时才追平
/// ——慢于 e2e 静默/认领窗口 → buffered echo 永不在窗口内 flush（UC-1.7 转发他频道 / 5.6w 公告写
/// / 他人消息红根因）。创可贴（`post.rs` echo_settle / 本文件 edit_settle）只对 self-echo +
/// PostEdit 立即补发投影且**不推 cursor**，覆盖不到转发他频道 / 他人消息。
///
/// ## 根治（非创可贴·非绕 gate）
///
/// 缺口够大即**立即**对该 channel 走 `emit_proactive_resync_for`（对齐 UC-4.x 的
/// `emit_proactive_resync` 追平机制）：sync 回拉缺口事件 + cursor 经 `on_persist_ok` commit 追平
/// 到 high-water → buffered echo 自然 flush emit。守 HX-C008：cursor 仍由 sync commit **严格推进**，
/// 不放行绕 gate、不破排序——是把缺口**真填上**让 cursor 推进。
///
/// ## 防风暴
///
/// ① per-channel `inflight_sync` 守卫：已在途 sync 的 channel 跳过（`emit_proactive_resync_for`
///    → `enqueue_and_drain` 内部 `inflight_sync.is_some() → continue`），突发多帧只触发一次；
/// ② 全局 `SyncScheduler` 窗口 K（≤ MAX_INFLIGHT_SYNC）；
/// ③ 小缺口（≤ 阈值）不在此触发——留 1s gate timer 兜底，给 WS 自然乱序重排留窗口。
pub(super) fn trigger_backfill_if_large_gap(
    ctx: &mut ImWsContext<'_>,
    channel_id: ChannelId,
    seq: Seq,
    out: &mut EffectSink,
) {
    let should_resync = match ctx.state.channels.get(&channel_id) {
        Some(ch) => {
            let cursor = ch.cursor.value().0;
            // 仅当：① 仍有缺口未平（gate armed）② 该 channel 无在途 sync（防风暴）
            //      ③ 缺口够大（真 backfill·非瞬时乱序）
            ch.gate.is_some()
                && ch.inflight_sync.is_none()
                && seq.0 > cursor.saturating_add(GAP_IMMEDIATE_RESYNC_THRESHOLD)
        }
        None => false,
    };
    if should_resync {
        super::increment_channel_end::emit_proactive_resync_for(ctx, &[channel_id], out);
    }
}

fn recent_effects_emitted_sync_too_long(effects: &[Effect], channel_id: ChannelId) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Emit { event } if {
            let s = String::from_utf8_lossy(event.0.as_ref());
            s.contains("\"event\":\"im:sync:too_long\"")
                && s.contains("\"mode\":\"drop_and_reload\"")
                && s.contains(channel_id.as_str())
        })
    })
}

fn recent_effects_already_request_latest_post(effects: &[Effect], channel_id: ChannelId) -> bool {
    effects.iter().any(|effect| {
        let Effect::Http { req, .. } = effect else {
            return false;
        };
        if !req.url.contains("posts/getLatestPost") {
            return false;
        }
        let Some(body) = req.body.as_ref() else {
            return false;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(body.as_ref()) else {
            return false;
        };
        value["channelId"] == channel_id.as_str()
    })
}

pub(super) fn schedule_reload_if_recent_too_long(
    ctx: &mut ImWsContext<'_>,
    channel_id: ChannelId,
    recent_start: usize,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let Some(recent) = out.as_slice().get(recent_start..) else {
        return Ok(());
    };
    if !recent_effects_emitted_sync_too_long(recent, channel_id)
        || recent_effects_already_request_latest_post(recent, channel_id)
    {
        return Ok(());
    }

    let Some(reset_to) = ctx
        .state
        .channels
        .get(&channel_id)
        .map(|ch| Seq(ch.cursor.value().0 + 1))
    else {
        return Ok(());
    };
    let connection_id = ctx.state.connection_id.clone();
    let corr = ctx.alloc_corr();
    let payload = serde_json::json!({
        "channel_id": channel_id.as_str(),
        "timestamp": 0,
    });
    let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
    let effects = crate::commands::handle_outbound(
        "im_get_latest_post",
        payload_bytes.as_ref(),
        ctx.api_base_url,
        ctx.api_base_url,
        connection_id.as_deref(),
        corr,
    )?;
    ctx.state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::TooLongReload {
            channel_id,
            reset_to,
        },
    );
    for effect in effects {
        out.push(effect);
    }
    Ok(())
}
