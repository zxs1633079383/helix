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
        reconcile_post_echo(ctx, &echo_tmp_id, echo_server_id, out)?;

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
                .request_for_command(request_id, ev.channel_id)
        });
        let ev = ev
            .with_unread_bump(unread_bump)
            .with_viewer_user_id(ctx.auth_user_id)
            .with_causation_id(forward_causation);
        if ctx.auth_user_id.is_empty() || i64::try_from(ev.seq.0).is_err() {
            return Ok(());
        }

        // MessageV3 G-01：陌生 channel 仍以本地 cursor=0 惰性注册；gate 只决定
        // apply/buffer/drop，真正的 message/channel/member/cursor 写入由 correlated
        // PersistAtomic 完成，matching PersistOk 后再读当前 viewer 复合键绝对态。
        let recent_start = out.as_slice().len();
        let post_visible = ev
            .unread_bump
            .as_ref()
            .map(|update| update.visible)
            .unwrap_or(true);
        let pin_effect = post_pin_effect_from_notice(&ev);
        let (ingest_channel, ingest_seq) = (ev.channel_id, ev.seq);
        let engagement_type = ev.fields.msg_type.clone();
        let gate_expected_seq = {
            let ch = ctx
                .state
                .channels
                .entry(ev.channel_id)
                .or_insert_with(|| crate::channel::Channel::new(ev.channel_id, 0));
            let expected = crate::state::Seq(ch.cursor.value().0.saturating_add(1));
            if post_visible {
                if let Some(event) = ch.admit_message_v3_post(ev, out)? {
                    let corr = ctx.alloc_corr();
                    crate::port_reply::message_v3_post::queue_commit(
                        ctx.state,
                        ctx.auth_user_id,
                        corr,
                        event,
                        out,
                    )?;
                }
            } else {
                let mut ignored_channel_updates = Vec::new();
                ch.ingest_collecting_channel_updates(
                    ev,
                    out,
                    ctx.now_ms,
                    &mut ignored_channel_updates,
                )?;
            }
            expected.0
        };
        // 只记录脱敏 gate 形状，定位投票/平均分 echo 是否因身份、可见性或序号被拦截。
        if matches!(engagement_type.as_str(), "VOTE" | "AVERAGE_SCORE") {
            tracing::info!(
                target: "helix_im::engagement_projection",
                hop = "post.echo_gate",
                event_seq = ingest_seq.0,
                expected_seq = gate_expected_seq,
                is_echo,
                post_visible,
                auth_present = !ctx.auth_user_id.is_empty(),
                temporary_id_present = !echo_tmp_id.is_empty(),
                "engagement post echo gate decision"
            );
        }
        super::gate::schedule_reload_if_recent_too_long(ctx, ingest_channel, recent_start, out)?;
        if let Some((persist, emit)) = pin_effect {
            // Pin notice 属独立 post-update Gate；G-01 普通 TEXT 不会进入此分支。
            out.push(persist);
            if let Some(effect) = ctx
                .state
                .invalidate_pinned_projection(ctx.auth_user_id, ingest_channel)
            {
                out.push(effect);
            }
            out.push(emit);
        }

        // 大缺口仍走既有主动 backfill，buffer 中事件只有在前置 seq 完成原子提交后才会继续。
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
/// 在 wasm inventory 不可自动发现时保留静态 handler。
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POST_HANDLER);
}
