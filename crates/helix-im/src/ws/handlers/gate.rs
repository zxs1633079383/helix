//! 共享：把带 `event_seq` 的内容类 WS 事件（type2 编辑 / type3 撤回 / type6 已读）
//! 经 **channel gate** 推进 cursor（BLOCKING-2）。
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
//! gate apply/flush 已 kind-aware（`channel::event_to_storage_op` / `emit_for_kind`）：
//! type2 内容 patch（保留本地 read_bits）/ type3 撤回 / type6 已读位覆盖，各按语义落库 + emit，
//! 故落库 + emit 全由 gate 完成——调用方**不再**自行 push 内容 op / emit / advance_cursor。

use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::sync_session::{EventEnvelope, EventKind, PostFields};

use super::super::{ImWsContext, WsFrame};

/// 大缺口立即 backfill 的阈值（gap = seq − cursor）。
///
/// 超过此阈值 → 判定为「活动频道 cursor 真落后服务端 high-water、需 backfill」，立即触发
/// per-channel proactive resync（不等 1s gate timer）。≤ 阈值的小缺口仍只走 1s gate timer，
/// 给 WS 自然乱序重排留窗口，避免对瞬时相邻乱序触发无谓 HTTP sync（回归保护）。
///
/// 取 2：gap > 2 ⇒ seq ≥ cursor+4（至少 3 条中间事件缺失）才算「真缺口」。echo gate-gap 整类
/// 实证 gap 极大（cursor=2 / server seq=22 ⇒ gap=20）；常规相邻乱序 gap=1。
const GAP_IMMEDIATE_RESYNC_THRESHOLD: u64 = 2;

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

/// 把内容类事件经 channel gate ingest（严格 +1 / drop / buffer+gate）。
///
/// - `channel_id`：cursor 推进锚（已 parse 校验的 Id26）；
/// - `kind`：PostEdit / PostRevoke / PostRead（决定 gate 落库 + emit 语义）；
/// - `fields`：落库所需 owned 视图（read_bits / message / props / id 等，调用方按 kind 填）；
/// - `msg_id`：wire 权威 msgId（patch WHERE 键；缺 → fields 派生）。
///
/// 无 event_seq → 无 cursor 锚，gate 无从判定连续性 → 仅落库 best-effort？不——真源 on_ws_event
/// 需要 k=event_seq，无 seq 的内容帧不进 gate。这里保守：无 seq 直接返回（cursor 不动、不落库），
/// 与缺 channelId 同口径（边界零信任，helix-im 不变量 4）。未注册 channel（driver 未种 cursor）
/// 静默忽略（与 post handler ingest 门控同口径）。
pub(super) fn gate_ingest_content_event(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    channel_id: ChannelId,
    kind: EventKind,
    fields: PostFields,
    msg_id: Option<String>,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let Some(seq) = frame.event_seq() else {
        return Ok(());
    };
    let ev = EventEnvelope::new(channel_id, seq, kind, fields)
        .with_msg_id(msg_id)
        .with_viewer_user_id(ctx.auth_user_id);
    if let Some(ch) = ctx.state.channels.get_mut(&channel_id) {
        let recent_start = out.as_slice().len();
        // —— edit echo 在 cursor gap 下兜底 emit（对齐 post.rs echo_settle·全链 echo 修复）——————
        // 真源对偶（post.rs:101-134）：post（type1）echo 在 seq≫cursor+1（冷启动 cursor 落后）下被
        // gate buffer → 投影不发 → 乐观行/DOM 永停。post handler 已加 self-echo 兜底 emit；但
        // **post_update（type2 编辑：模板已收到 / 快捷回复 / 公告写）echo 走本 gate 入口未加兜底**
        // → 同款 cursor-gate buffer bug：收到 post_update 帧但 seq≠cursor+1 → buffer → 永不 emit
        // im:post:updated → DOM data-template-received / data-reactions 永不更新（UC-3.3/1.8/5.6w 红根因）。
        //
        // 修复：编辑 echo 是「对既有行的就地更新」，其投影与 channel 事件**排序无关**（壳按 msg_id
        // 幂等覆盖）。当 gate 不会自然 apply-emit（seq≠cursor+1）时，用在手 ev.fields 兜底 emit 一次
        // im:post:updated 结算 DOM。守 HX-C008：**不推 cursor**（cursor 仍由 ch.ingest 严格 +1 推进，
        // gap 经 buffer/sync 回补）；buffer 后续 flush 时会再 emit 同投影（壳幂等·无害）。
        // 仅 PostEdit（type2）兜底——撤回（type3）/已读（type6）语义不同，不在此兜底。
        let edit_settle =
            if matches!(ev.kind, EventKind::PostEdit) && ev.seq.0 != ch.cursor.value().0 + 1 {
                let msg_id_ref = ev
                    .msg_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(ev.fields.id.as_str());
                // 两面兜底（对齐 post.rs：echo_settle 发投影 + reconcile_post_echo 落库都不过 gate）：
                //   ④ 落库：edit_content_op（UPDATE WHERE id·幂等·保留本地 read_bits）让 storage 面成立；
                //   ② 投影：im:post:updated 让 projection + DOM 面成立。
                // 二者均**不推 cursor**（HX-C008：cursor 仍由 ch.ingest 严格 +1·gap 经 buffer/sync 回补；
                // buffer flush 时会再落同 op + 发同投影·均幂等无害）。
                Some((
                    helix_core::Effect::PersistFire {
                        ops: vec![crate::channel::event_to_storage_op(&ev)],
                    },
                    crate::acl::to_effect::emit_post_updated_for_viewer(
                        channel_id,
                        ev.seq.0,
                        msg_id_ref,
                        &ev.fields,
                        ev.viewer_user_id.as_str(),
                    ),
                ))
            } else {
                None
            };
        ch.ingest(ev, out, ctx.now_ms)?;
        schedule_reload_if_recent_too_long(ctx, channel_id, recent_start, out)?;
        // gate buffer/drop 下编辑 echo 的落库 + DOM 兜底结算（apply 路径已自然落库+emit → None）。
        if let Some((persist, emit)) = edit_settle {
            out.push(persist);
            out.push(emit);
        }
    }
    // 根治：大缺口立即 backfill resync（追平 cursor → flush buffered echo）。&mut ctx.state.channels
    // 借用已在上面的 if-let 作用域结束时释放，此处可安全重借 ctx。
    trigger_backfill_if_large_gap(ctx, channel_id, seq, out);
    Ok(())
}

/// 仅推进 cursor 的 gate 入口（`posts_update` 批量帧用，BLOCKING-2）。
///
/// `posts_update` 是**单 cursor 事件**（顶层一个 event_seq）携 N 条 post 编辑/撤回：N 条内容
/// patch 已由调用方**立即**幂等落库（`BatchUpdate WHERE id`），cursor 由顶层 event_seq 推进一次。
/// 走 `Channel::ingest_cursor_advance`（严格 +1 / drop / 缺口仅 arm 1s gate，绝不 MAX-jump 跨空洞，
/// 守 spec §S6）——与 `gate_ingest_content_event` 同口径：无 event_seq → no-op（无 cursor 锚），
/// 未注册 channel（driver 未种 cursor）静默忽略（边界零信任，helix-im 不变量 4）。
pub(super) fn gate_advance_cursor_only(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    channel_id: ChannelId,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let Some(seq) = frame.event_seq() else {
        return Ok(());
    };
    if let Some(ch) = ctx.state.channels.get_mut(&channel_id) {
        ch.ingest_cursor_advance(seq, out)?;
    }
    // 根治：posts_update 批量帧的顶层 event_seq 缺口同样立即 backfill（cursor 卡住时追平）。
    trigger_backfill_if_large_gap(ctx, channel_id, seq, out);
    Ok(())
}
