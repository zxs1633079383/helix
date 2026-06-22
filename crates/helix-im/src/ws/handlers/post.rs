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
use crate::state::{ChannelId, Seq, ServerId, TemporaryId};
use crate::sync_session::{EventEnvelope, EventKind};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

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
        // 解析失败（缺 channelId / event_seq）→ no-op（边界零信任，不误报；与他人推送同口径）。
        let Some(ev) = parse_post_frame(frame) else {
            return Ok(());
        };

        // R4-echo-gate：echo 对账与 channel gate **解耦**——reconcile 只读 pending_sends/corr_map，
        // 从不读 channels，故必须**无条件**先做。否则冷启动 send-before-cursor-seed 乱序下，本端帧因
        // channel 未注册被整体跳过 → pending 残留 → 15s on_timeout 把已送达消息误标 UnSend。
        // 仅当 temporary_id 命中在途 send 时才实际对账（内部门控）。
        let echo_tmp_id = ev.fields.temporary_id.clone();
        let echo_server_id = ServerId::from_str(ev.fields.id.as_str());
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
        // bump（§5 验收：echo 无 unread）；此判定在 handler（有 ctx）做，对**乱序帧同样有效**，故
        // 「乱序自端 echo」无需依赖额外假设。重复/旧消息（seq<=cursor）由 gate dup-drop 不 apply →
        // 不发；GuardedBump 守卫（create_at > last_root_post_at）再兜底幂等。
        let is_echo = is_echo_frame(ctx, &echo_tmp_id, &ev, frame);
        let unread_bump = if is_echo {
            None
        } else {
            parse_post_data(frame).map(|data| {
                crate::channel_write::post_updates(
                    ev.channel_id,
                    data,
                    ctx.auth_user_id,
                    ev.fields.create_at,
                )
            })
        };
        let ev = ev.with_unread_bump(unread_bump);

        // ingest（落库 + cursor 推进 + ⑤ 未读 bump 由 gate 发出）门控 channel 注册：未注册 channel
        // （driver 未通过 increment 种 cursor）静默忽略，与 legacy event 路径同口径（driver 路由问题）。
        if let Some(ch) = ctx.state.channels.get_mut(&ev.channel_id) {
            ch.ingest(ev, out, ctx.now_ms)?;
        }
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
    let fields = crate::parser::extract_post_fields(data);

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

/// 判定 `post` 帧是否为本端 echo（自己发的消息回流），用于 path3 未读 +1 前置闸门。
///
/// 两路识别（命中任一即 echo）——对齐现网「自己消息 +0」（message_service.rs:342 sender 豁免）：
///   1. `temporary_id` 命中在途 `PendingSend`：本端乐观发送的占位消息回流（最强信号，PK 对账键）；
///   2. `user_id == auth_user_id`（host 注入真身份且非空时）：sender 即自己。
///
/// 无身份（auth 空串）时仅靠路 1（temporary_id）——保守不漏未读（与 `post_updates` 退化同策略）。
/// 边界零信任：纯只读判定，不改状态、不 panic。
fn is_echo_frame(
    ctx: &ImWsContext<'_>,
    temporary_id: &str,
    ev: &EventEnvelope,
    frame: &WsFrame,
) -> bool {
    // 路 1：temporary_id 命中在途 send（本端乐观占位回流）。
    if !temporary_id.is_empty() {
        let tmp = TemporaryId(temporary_id.to_string());
        if ctx.state.pending_sends.contains_key(&tmp) {
            return true;
        }
    }
    // 路 2：发送者 == 当前登录用户（仅当 host 注入了非空 auth 身份才可判）。
    if !ctx.auth_user_id.is_empty() {
        // sender 优先 ev.fields.user_id（parser 已提取）；回退 wire data.userId（含 userSnapshot）。
        let sender = if !ev.fields.user_id.is_empty() {
            ev.fields.user_id.as_str()
        } else {
            frame
                .data()
                .and_then(|d| {
                    d.get("userSnapshot")
                        .and_then(|s| s.get("userId"))
                        .or_else(|| d.get("userId"))
                })
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        };
        if !sender.is_empty() && sender == ctx.auth_user_id {
            return true;
        }
    }
    false
}

/// `post` 帧的发送对账（BLOCKING 修复）。
///
/// 真 Go 在线广播（含本端 echo）统一发 `action=="post"`，由 `parse_post_frame` 喂 `ch.ingest`
/// 完成 cursor 推进 + 落库（ON CONFLICT temporary_id 覆盖乐观行）。但 `event_to_upsert_op`
/// **不写 send_status 列**、**不取消 15s timer**——这两件事是 helix 把现网前端的乐观态/超时
/// 下沉进 core 后新增的职责。本函数补齐：
///
/// - `temporary_id` 命中在途 `PendingSend` 时 → `ps.reconcile`（吐 Persist 覆写
///   `send_status='sent'` + `id=server_id` ON CONFLICT(temporary_id) + CancelTimer 撤 15s，
///   状态推进到 Sent）。
/// - 之后从 `pending_sends` + `corr_map`(OptimisticSend) 移除该记录（防泄漏）。
///
/// **不推进 cursor**：cursor 已由调用方 `ch.ingest` 推进（HX-C008 单调，避免双推）。
/// 非 echo（`temporary_id` 空 / 不在 pending_sends / `id` 非合法 26 字符 ServerId）→ no-op，
/// 与他人推送的新消息一致。边界零信任不 panic。
fn reconcile_post_echo(
    ctx: &mut ImWsContext<'_>,
    temporary_id: &str,
    server_id: Option<ServerId>,
    out: &mut EffectSink,
) {
    // 空 temporary_id（他人推送的新消息）→ 非 echo，不动发送态。
    if temporary_id.is_empty() {
        return;
    }
    // server_id 必须是合法 26 字符 Id26（reconcile 覆写 message.id 必须用合法 server id；
    // 非法/缺失 → 不 reconcile，留待 sync 兜底覆盖）。
    let Some(server_id) = server_id else {
        return;
    };
    let tmp = TemporaryId(temporary_id.to_string());
    // 仅当命中在途 PendingSend 才对账（先分配 corr 避免双重可变借用）。
    if !ctx.state.pending_sends.contains_key(&tmp) {
        return;
    }
    let reconcile_corr = ctx.alloc_corr();
    // R4-retain：取出该 send 的 P1 Persist corr（send 时回填），用于 O(1) 清路由。
    let persist_corr = ctx
        .state
        .pending_sends
        .get(&tmp)
        .and_then(|ps| ps.persist_corr);
    if let Some(ps) = ctx.state.pending_sends.get_mut(&tmp) {
        ps.reconcile(server_id, reconcile_corr, out);
    }
    // 对账落定后移除（防 PendingSend / corr_map 泄漏；late timer 已被 CancelTimer 撤销，
    // 即便竞态到达也因 status==Sent 幂等无害）。
    ctx.state.pending_sends.remove(&tmp);
    // 对账落定后移除该 send 的 OptimisticSend 路由（防 corr_map 泄漏）。
    if let Some(corr) = persist_corr {
        ctx.state.corr_map.remove(&corr);
    }
}

static POST_HANDLER: PostHandler = PostHandler;

#[distributed_slice(WS_HANDLERS)]
static POST_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POST_ACTION,
    handler: &POST_HANDLER,
};
