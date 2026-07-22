//! `post` action handler（C1 sync-v2 真 Go 在线广播新消息 / echo 统一帧）。
//!
//! 行为真源（现网 source of truth）：
//!   - `cses-client/src-tauri/src/features/im/router.rs:40` `"post" => post::handle`；
//!   - `post.rs:42-114` 统一 `handle_new_post`，不在落库层分叉 echo/新消息；
//!   - `message_service.rs:64-69` `batch_upsert ON CONFLICT(temporary_id)`。
//!
//! 真 Go WS 在线新消息广播顶层 `action=="post"`（**不是** `posted`——Go 常量
//! `WebsocketEventPost="post"` 与 `WebsocketEventPosted="posted"` 是两个事件，cses 新消息用
//! `post`）。echo 与新消息**不在落库层分叉**：统一解析成 `EventEnvelope` 喂 channel gate
//! （`ch.ingest` → `event_to_upsert_op` ON CONFLICT temporary_id），echo（本端 temporary_id
//! 命中乐观行）靠 PK 自然对账，server `id` 覆写本地行。

use crate::error::ImError;
use crate::state::ServerId;
use helix_core::EffectSink;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};
use super::post_echo_settle::{is_echo_frame, reconcile_post_echo};

const POST_ACTION: &str = "post";

mod parse;
mod pin;
use parse::{emit_unsequenced_post, parse_post_data, parse_post_frame};
use pin::post_pin_effect_from_notice;

struct PostHandler;

impl WsMessageHandler for PostHandler {
    fn action(&self) -> &'static str {
        POST_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 解析失败（缺 channelId / event_seq）→ 若仍有 post/channelId，则发 render-only thin bus，
        // 不推进 cursor；否则 no-op。
        let Some(ev) = parse_post_frame(frame) else {
            if let Some(channel_id) = emit_unsequenced_post(frame, ctx.auth_user_id, out) {
                ctx.state.invalidate_recent_message_coverage(channel_id);
            }
            return Ok(());
        };
        ctx.state.invalidate_recent_message_coverage(ev.channel_id);

        // R4-echo-gate：echo 对账与 channel gate **解耦**——reconcile 只读 pending_sends/corr_map，
        // 从不读 channels，故必须**无条件**先做。否则冷启动 send-before-cursor-seed 乱序下，本端帧因
        // channel 未注册被整体跳过 → pending 残留 → 15s on_timeout 把已送达消息误标 UnSend。
        // 仅当 temporary_id 命中在途 send 时才实际对账（内部门控）。
        let echo_tmp_id = ev.fields.temporary_id.clone();
        let echo_server_id = ServerId::from_str(ev.fields.id.as_str());
        let is_echo = is_echo_frame(ctx, &echo_tmp_id, &ev, frame);
        reconcile_post_echo(ctx, ev.channel_id, &echo_tmp_id, echo_server_id, out);

        // S3 path3：未读 +1 决策（未读自增 SQL + lastPost 组）。**⑤ 修复（gate 单一发出点）**：
        // 决策在此用「原始帧 + auth + 完整 echo 判定」算定后 attach 到 envelope，由 gate 在
        // **apply 与 flush 两路**统一发出——**不再**在此直接 push、**不再**用 is_contiguous 门控
        // （apply 时机交给 gate）。修复偏差：乱序（seq>cursor+1）post 先进 buffer，flush 时同样
        // 补未读（原实现只在立即 apply 路径 +1，乱序经 buffer flush 丢未读）。
        //
        // BLOCKING-1：auth_user_id 由 host 经 ctx 注入真值。`post_updates` 据此做 ① sender 豁免
        // （自己消息 +0，真源 message_service.rs:342）② 可见性门控（viewers 不含自己 → +0）。
        // echo 帧（本端回流）显式跳过（temporary_id 命中在途 send 或 user_id==self），即便落库也绝不
        // bump（own echo 无 unread，temp_id 命中不依赖 auth）；此判定在 handler（有 ctx）做，对**乱序帧同样有效**，故
        // 「乱序自端 echo」无需依赖额外假设。重复/旧消息（seq<=cursor）由 gate dup-drop 不 apply →
        // 不发；GuardedBump 守卫（create_at > last_root_post_at）再兜底幂等。
        let unread_bump = parse_post_data(frame).map(|data| {
            let mut update = crate::channel_write::post_updates(
                ev.channel_id,
                data,
                ctx.auth_user_id,
                ev.fields.create_at,
            );
            if is_echo {
                update.unread_delta = 0;
                update.unread_post_id = None;
            }
            update
        });
        let forward_causation = frame.cses_track_id().and_then(|request_id| {
            ctx.state
                .pending_forward_deliveries
                .matching_request(request_id, ev.channel_id)
        });
        let ev = ev
            .with_unread_bump(unread_bump)
            .with_viewer_user_id(ctx.auth_user_id)
            .with_causation_id(forward_causation.clone());

        // ingest（落库 + cursor 推进 + ⑤ 未读 bump 由 gate 发出）。**陌生 channel 自动注册**
        // （Round-3 UC-4.5 偏差修复，对齐真源 `on_ws_event` 的 `gate_states.entry().or_insert_with`
        // + `handle_new_post` **无条件**落库）：首次见到的新群 / 被拉入群的首条在线 push 在 increment
        // 种 cursor 之前到达时，**不再整帧静默丢弃**——以 cursor=0 惰性注册（与 `increment_channel`
        // 真新 channel 同口径：cursor=本地未确认，**不**误种服务端水位），随后由 gate 处理：
        //   - event_seq==1（==cursor+1）→ 立即 apply：batch_upsert message + emit im:post:received + cursor→1；
        //   - event_seq>1（缺口）→ buffer + arm 1s gate → sync 回拉补齐前置消息，首条不丢。
        // on_start Scan 已把所有持久化 cursor 灌入 channels（handle_scan_reply），故「不在 channels」
        // 严格等价「DB 无 cursor 的真新 channel」，seed=0 正确（HX-C008 单调，gate 缺口回拉兜底）。
        let ch = ctx
            .state
            .channels
            .entry(ev.channel_id)
            .or_insert_with(|| crate::channel::Channel::new(ev.channel_id, 0));

        // —— gap DOM 兜底结算（fix/im-send-echo-reconcile·路 2 DOM 面）————————————
        // 乐观行 tmp→server 覆写 + status=sent 的 DOM 渲染由 `im:post:received` 投影驱动
        // （壳 im-store 按 temporaryId upsert）。该投影**仅在 gate apply（seq==cursor+1）时**由
        // ch.ingest emit；本端 echo 在 cursor 落后（gap，seq≫cursor+1）下被 buffer → 投影不发 →
        // 乐观行永停 sending（DB 的 id 已由 reconcile_post_echo 落定，但 DOM 是纯渲染壳，必须靠投影）。
        //
        // 修复：self-echo 是「发送者自己消息回流」，转发 / 他人消息则是移动端入栈真消息。二者的
        // render 投影与 channel cursor 推进解耦：当 gate 不会自然 apply-emit（seq≠cursor+1）时，
        // 用在手 ev.fields 兜底 emit 一次 `im:post:received` 结算渲染面。
        // 守不变量：**不推 cursor**（HX-C008——cursor 仍由 ch.ingest 的 gate 严格 +1 推进，gap 经
        // buffer/sync 回补；本兜底只发投影）。buffer 后续 flush 时会再 emit 同投影（壳按 msgId
        // 幂等 upsert·无害）。字段集走冻结 message_item_data（C004 不改契约）。
        let post_visible = ev
            .unread_bump
            .as_ref()
            .map(|update| update.visible)
            .unwrap_or(true);
        let post_settle = if post_visible && ev.seq.0 != ch.cursor.value().0 + 1 {
            let msg_id = ev
                .msg_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(ev.fields.id.as_str());
            Some(crate::acl::to_effect::emit_post_received_for_viewer(
                ev.channel_id,
                ev.seq.0,
                msg_id,
                &ev.fields,
                ctx.auth_user_id,
            ))
        } else {
            None
        };

        // 捕获 backfill 触发所需键（ev 即将 move 进 ch.ingest）。
        let pin_effect = post_pin_effect_from_notice(&ev);
        let (ingest_channel, ingest_seq) = (ev.channel_id, ev.seq);

        let recent_start = out.as_slice().len();
        let mut dialog_patches = Vec::new();
        ch.ingest_collecting_dialog_patches(ev, out, ctx.now_ms, &mut dialog_patches)?;
        // Only a contiguous gate apply (possibly flushing a buffered run) has
        // actually appended an authoritative message write. Buffer/drop paths
        // must not fabricate a timeline Frame from the raw WS payload. Do not
        // use the handler's unread visibility hint here: it has narrower
        // semantics than the gate's actual persisted-message decision.
        let timeline_readback_required =
            super::gate::recent_effects_persist_message(&out.as_slice()[recent_start..]);
        super::gate::schedule_reload_if_recent_too_long(ctx, ingest_channel, recent_start, out)?;

        if timeline_readback_required {
            ctx.request_attached_timeline_refresh_with_causation(ingest_channel, forward_causation);
        }

        // gate buffer/drop 下 post 的 DOM 结算（gate apply 路径已自然 emit → post_settle=None）。
        if let Some(eff) = post_settle {
            out.push(eff);
        }
        for pending in dialog_patches {
            let corr = ctx.alloc_corr();
            out.push(helix_core::Effect::Persist {
                corr,
                ops: pending.storage_ops(),
            });
            ctx.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::DialogPatchByPost { pending },
            );
        }
        if let Some((persist, emit)) = pin_effect {
            out.push(persist);
            out.push(emit);
        }

        // 根治 echo gate-gap 整类：post（type1·含转发他频道 / 他人消息）在大缺口下被 gate buffer
        // 时，立即触发该 channel proactive resync 追平 cursor → buffered post 自然 flush emit。
        // 创可贴 echo_settle 只救 self-echo；本路覆盖转发他频道 / 他人消息（守 HX-C008·不绕 gate）。
        super::gate::trigger_backfill_if_large_gap(ctx, ingest_channel, ingest_seq, out);
        Ok(())
    }
}

static POST_HANDLER: PostHandler = PostHandler;

inventory::submit! {
    WsHandlerRegistration {
        action: POST_ACTION,
        handler: &POST_HANDLER,
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POST_HANDLER);
}
