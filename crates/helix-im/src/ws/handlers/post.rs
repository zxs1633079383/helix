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

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::{ChannelId, Seq, ServerId};
use crate::sync_session::{EventEnvelope, EventKind, PostFields};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};
use super::post_echo_settle::{is_echo_frame, reconcile_post_echo};

const POST_ACTION: &str = "post";

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
        reconcile_post_echo(ctx, &echo_tmp_id, echo_server_id, out);

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
        let ev = ev
            .with_unread_bump(unread_bump)
            .with_viewer_user_id(ctx.auth_user_id);

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
        super::gate::schedule_reload_if_recent_too_long(ctx, ingest_channel, recent_start, out)?;

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

/// 解析真 Go `action=="post"` 在线广播帧 → `EventEnvelope`（喂 channel gate）。
///
/// 硬 gate（缺则 None → no-op，边界零信任不 panic，helix-im 不变量 4）：
///   1. `channel_id`：`data.channelId` ?? `data.channel_id`（26 字符 Id26）——cursor 推进锚；
///   2. `seq`：event_seq 三级 fallback（**data.event_seq 优先**，与 `parse_inbound`/§1.5 同源）：
///      ① `data.event_seq`（权威 int64，post_core.go:424 注入顶层 data）
///      → ② per-post `data.props.channel_event_seq`（单帧冗余兜底，与顶层同值）
///      → ③ 顶层 `channel_event_seq`（legacy 键名）。零信任只取 `as_u64`。
///
/// 宽松字段（**不**硬校验 26 字符 id——现网 handle_new_post 对 id 不卡
/// `message_service.rs:55 unwrap_or("unknown")`）：
///   - `kind`：恒 `PostUpsert`（type1；post 帧恒为新消息 upsert）；
///   - `fields`：`extract_post_fields(data)`（复用既有提取，全程安全回退）；
///   - `msg_id`：宽松取 `data.id` / `data.post.id`（缺省 None，with_msg_id 内部回退 fields 派生）。
fn parse_post_frame(frame: &WsFrame) -> Option<EventEnvelope> {
    let data = frame.data()?;
    // post 字段可能嵌在 data 顶层或 data.post 子对象下（与 extract_post_fields 同源探查）。
    let post = data.get("post").unwrap_or(data);

    // channel_id 硬 gate（cursor 推进锚）：兼容 camel/snake；缺/非 26 字符 → None。
    let channel_id = data
        .get("channelId")
        .or_else(|| data.get("channel_id"))
        .or_else(|| post.get("channelId"))
        .or_else(|| post.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)?;

    // event_seq 三级 fallback（§1.5，data.event_seq 优先，与 parse_inbound 同源）：
    //   ① data.event_seq（权威 int64）→ ② per-post props.channel_event_seq → ③ 顶层 channel_event_seq。
    let seq = Seq(data
        .get("event_seq")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            post.get("props")
                .and_then(|props| props.get("channel_event_seq"))
                .and_then(serde_json::Value::as_u64)
        })
        .or_else(|| {
            frame
                .root()
                .get("channel_event_seq")
                .and_then(serde_json::Value::as_u64)
        })?);

    // post 帧恒为新消息 upsert（type1）；字段复用既有提取（宽松回退，含 temporary_id/id/message/props/readBits）。
    let fields = crate::ws::parser::extract_post_fields(data);

    // msg_id 宽松取（缺省 None → with_msg_id 内部回退 fields 派生的 id/temporary_id）。
    // **不**硬校验 26 字符（现网 handle_new_post 对 id 不 gate）。
    let msg_id = post
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    Some(EventEnvelope::new(channel_id, seq, EventKind::PostUpsert, fields).with_msg_id(msg_id))
}

/// 取 `post` 帧的 post 数据子树（path3 谓词用：type/viewers/isSchedule/userId/postId）。
///
/// 与 `parse_post_frame` 同源探查：post 字段可能在 `data.post` 子对象或 `data` 顶层。
/// 缺 data → None（边界零信任，path3 写跳过；message 落库仍走 ingest）。
fn parse_post_data(frame: &WsFrame) -> Option<&serde_json::Value> {
    let data = frame.data()?;
    Some(data.get("post").unwrap_or(data))
}

/// 真实 Go 的部分 post-like 回声（如 createPosts/relay）可能不携带 event_seq。移动端仍需要
/// `im:post:received` 入栈信号，但缺序帧不能推进 channel cursor。这里仅发 render-only bus。
fn emit_unsequenced_post(
    frame: &WsFrame,
    viewer_user_id: &str,
    out: &mut EffectSink,
) -> Option<ChannelId> {
    let data = frame.data()?;
    let post = data.get("post").unwrap_or(data);
    let channel_id = post
        .get("channelId")
        .or_else(|| post.get("channel_id"))
        .or_else(|| data.get("channelId"))
        .or_else(|| data.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)?;
    let fields = crate::ws::parser::extract_post_fields(post);
    let msg_id = post
        .get("id")
        .or_else(|| post.get("postId"))
        .or_else(|| post.get("post_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fields.id.as_str());
    out.push(crate::acl::to_effect::emit_post_received_for_viewer(
        channel_id,
        0,
        msg_id,
        &fields,
        viewer_user_id,
    ));
    Some(channel_id)
}

/// Go 真实 post pin 写族当前回声形态是 `action=post` 系统通知：
/// `type=NOTICE + props.type=addPostPin/removePostPin + props.content=<被操作 post JSON>`。
/// 这里在 helix 归一为目标消息 pinned patch，保持前端纯绑定 `data-pinned`。
fn post_pin_effect_from_notice(
    ev: &EventEnvelope,
) -> Option<(helix_core::Effect, helix_core::Effect)> {
    if !ev.fields.msg_type.eq_ignore_ascii_case("NOTICE") || ev.fields.props.is_empty() {
        return None;
    }
    let props: serde_json::Value = serde_json::from_str(&ev.fields.props).ok()?;
    let prop_type = props.get("type").and_then(serde_json::Value::as_str)?;
    let pinned = match prop_type {
        "addPostPin" => true,
        "removePostPin" => false,
        _ => return None,
    };
    let target_post: serde_json::Value =
        serde_json::from_str(props.get("content").and_then(serde_json::Value::as_str)?).ok()?;
    let post_id = target_post
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())?;
    let channel_id = target_post
        .get("channelId")
        .or_else(|| target_post.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .unwrap_or(ev.channel_id);

    let fields = PostFields {
        id: post_id.to_string(),
        channel_id: channel_id.as_str().to_string(),
        props: serde_json::json!({ "pinned": pinned }).to_string(),
        ..PostFields::default()
    };
    Some((
        helix_core::Effect::PersistFire {
            ops: vec![crate::channel::pin_state_op(post_id, pinned)],
        },
        crate::acl::to_effect::emit_post_updated(channel_id, ev.seq.0, post_id, &fields),
    ))
}

static POST_HANDLER: PostHandler = PostHandler;

#[distributed_slice(WS_HANDLERS)]
static POST_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POST_ACTION,
    handler: &POST_HANDLER,
};
