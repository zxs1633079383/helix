//! `post` 帧的 echo 对账 / 自端识别辅助（从 `post.rs` 拆出·零行为变更）。
//!
//! structure-gate 300 行硬顶（many-small-files）。这两个辅助是 `post::handle` 的
//! echo 路径配套：
//!   - `is_echo_frame`：path3 未读 +1 前的「本端 echo」判定（temporary_id 命中在途 send /
//!     user_id==self）；
//!   - `reconcile_post_echo`：temporary_id 命中在途 `PendingSend` 时覆写 send_status=sent +
//!     id=server_id + 撤 15s timer，并清 pending_sends/corr_map。
//!
//! 行为真源同 `post.rs` 头注（cses-client `router.rs`/`message_service.rs`）。

use helix_core::EffectSink;

use crate::state::{ServerId, TemporaryId};
use crate::sync_session::EventEnvelope;

use super::super::{ImWsContext, WsFrame};

/// 判定 `post` 帧是否为本端 echo（自己发的消息回流），用于 path3 未读 +1 前置闸门。
///
/// 两路识别（命中任一即 echo）——对齐现网「自己消息 +0」（message_service.rs:342 sender 豁免）：
///   1. `temporary_id` 命中在途 `PendingSend`：本端乐观发送的占位消息回流（最强信号，PK 对账键）；
///   2. `user_id == auth_user_id`（host 注入真身份且非空时）：sender 即自己。
///
/// 无身份（auth 空串）时仍可用路 1（temporary_id）识别本端 echo；命中 pending send 即可安全走
/// own-echo unread=0，避免前端再做 +1/-1 业务补偿。
/// 边界零信任：纯只读判定，不改状态、不 panic。
pub(super) fn is_echo_frame(
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
pub(super) fn reconcile_post_echo(
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
