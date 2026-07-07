//! F1 发消息 HTTP 200 兜底对账（从 `module.rs` 外提，守 module.rs ≤ baseline）。
//!
//! ## 根因（UC-1.1 阻断）
//!
//! 真 Go 对**发送者自身连接不推 `post` WS 回声**（self-omit，实测 4/4 会话 0 echo）→ 若只押
//! WS echo，乐观行永停 `Sending` 到 15s 超时 spinner。消息**真落库了**（HTTP 200
//! `{status:SUCCESS}`），故 `posts/create` HTTP 200 SUCCESS 即可兜底标 `Sent`。
//!
//! ## 幂等
//!
//! 与 WS echo 路径（`ws/handlers/post.rs::reconcile_post_echo`）共存——两者都按 `temporary_id`
//! ON CONFLICT 对账，后到者覆盖同值无害；先到者已清 pending → 后到 no-op。

use helix_core::{Correlation, EffectSink};

use crate::state::{CorrelationContext, ImState, ServerId, TemporaryId};

/// 注册 P1 `OptimisticSend` 路由（乐观落库回报）。
pub(crate) fn register_optimistic_send_corr(
    state: &mut ImState,
    p1_corr: Correlation,
    temporary_id: TemporaryId,
) {
    state
        .corr_map
        .insert(p1_corr, CorrelationContext::OptimisticSend { temporary_id });
}

/// 注册 H1 `OutboundSendHttp` 路由（posts/create HTTP 200 兜底对账）。
///
/// **F1 修复点**：H1 必须注册——漏注册时 posts/create HTTP 200 回报
/// `corr_map.remove(h1_corr)` 返 None → 响应静默丢弃 → 真 Go self-omit echo 下乐观行永停
/// `Sending` 到 15s spinner。rich/file 上传门控下，H1 只能在全部上传成功、真正发出
/// posts/create 时注册。
pub(crate) fn register_outbound_send_http_corr(
    state: &mut ImState,
    h1_corr: Correlation,
    channel_id: crate::state::ChannelId,
    temporary_id: TemporaryId,
) {
    state.corr_map.insert(
        h1_corr,
        CorrelationContext::OutboundSendHttp {
            channel_id,
            temporary_id,
        },
    );
}

/// `posts/create` HTTP 200 兜底对账：解 ADR-007 信封 → 裸 Go body `{status, data?}`。
///
/// - `status == "SUCCESS"` + 含 server post id（防御取 `data.id` / `data.post.id` / `id`）
///   → `pending_send.reconcile(server_id)`（吐 Persist `id=serverId,status=sent` + CancelTimer，
///   推进 `Sent`）。
/// - `status == "SUCCESS"` 但无可解 serverId → `mark_sent_pending_server_id`（仅 `status=sent`
///   + CancelTimer + `Sent`），**保留 pending_sends 不清栈**：serverId 由随后 WS echo 按
///   temporary_id 幂等补写 id 列并在 `reconcile_post_echo` 清栈（修 9 个 send spec 红根因）。
/// - `status != "SUCCESS"`（含 4xx body 误判）/ 信封畸形 → 不标 Sent（边界零信任不 panic，
///   留 15s timer 兜底标 UnSend）。
///
/// `reconcile_corr` 由调用方分配（单调分配器，HX-C005）。
/// **清栈口径**：仅「HTTP 响应已含 serverId」分支在此清 pending_sends + corr_map；「SUCCESS 无
/// serverId」分支不清（留待 WS echo 路径清，避免 echo 到达时 contains_key=false 提前 return）。
pub(crate) fn reconcile_send_http(
    state: &mut ImState,
    temporary_id: &TemporaryId,
    reply: &[u8],
    reconcile_corr: Correlation,
    out: &mut EffectSink,
) {
    // 仅当仍有在途 PendingSend 才处理（WS echo 先到已清理 → no-op，幂等）。
    if !state.pending_sends.contains_key(temporary_id) {
        return;
    }

    // 解信封 → 裸 Go body；信封/base64 畸形 → 不标 Sent（不 panic）。
    let Ok(raw_body) = crate::http_envelope::unwrap_sync_envelope(reply) else {
        tracing::warn!(tmp_id = ?temporary_id, "posts/create reply envelope decode failed, not advancing");
        return;
    };
    // 解 JSON body；坏 JSON → 不标 Sent。
    let Ok(body) = serde_json::from_slice::<serde_json::Value>(raw_body.as_ref()) else {
        tracing::warn!(tmp_id = ?temporary_id, "posts/create reply body JSON parse failed, not advancing");
        return;
    };
    // status 必须严格 == "SUCCESS"（防 4xx body 误判，边界零信任）。
    if body.get("status").and_then(serde_json::Value::as_str) != Some("SUCCESS") {
        tracing::warn!(tmp_id = ?temporary_id, "posts/create reply status != SUCCESS, not advancing (will timeout)");
        return;
    }

    if let Some(server_id) = extract_post_server_id(&body) {
        // 含 serverId → reconcile：Persist(id=serverId,status=sent) + CancelTimer + Sent。
        // serverId 已在 HTTP 响应落定 → 无需再等 WS echo，**此处即清栈**（pending_sends + corr_map），
        // 与 echo 路径终态一致（后到 echo 因 contains_key=false no-op，幂等无害）。
        // R4-retain：取出该 send 的 P1 Persist corr（清 corr_map 用）。
        let persist_corr = state
            .pending_sends
            .get(temporary_id)
            .and_then(|ps| ps.persist_corr);
        if let Some(ps) = state.pending_sends.get_mut(temporary_id) {
            ps.reconcile(server_id, reconcile_corr, out);
        }
        state.pending_sends.remove(temporary_id);
        if let Some(corr) = persist_corr {
            state.corr_map.remove(&corr);
        }
    } else if let Some(ps) = state.pending_sends.get_mut(temporary_id) {
        // SUCCESS 但无 serverId（cses post 经 Pulsar 异步建，posts/create HTTP 200 体仅 `{status}`）
        // → 仅标 Sent + persist 乐观行 send_status=sent + 撤 15s timer（停 spinner）。
        //
        // **根因修复（fix/im-send-echo-reconcile）**：此分支**绝不**清 pending_sends / corr_map。
        // serverId 由随后携带 server id 的 WS `post` echo 经
        // `ws/handlers/post.rs::reconcile_post_echo` 按 temporary_id 幂等补写
        // （ON CONFLICT(temporary_id) 覆盖 id 列 → 乐观行 id: temporary_id→server_id），
        // **并在那里清栈**（pending_sends + corr_map[persist_corr]，见 reconcile_post_echo 尾部）。
        //
        // 旧实现在此无条件 `pending_sends.remove` → echo 到达时 contains_key=false →
        // reconcile_post_echo 提前 return → server_id 永不落乐观行 id 列（9 个 send 类 spec 红的根因）。
        //
        // 泄漏边界：timer 已被 mark_sent_pending_server_id 内 CancelTimer 撤销（不会误标 UnSend），
        // 行已 Sent（UI 正确）。echo 永不到达的极端边界下残留 1 条 pending_send；这与既有失败路
        // （status != SUCCESS 早返 + on_timeout 不清栈）同档，非本次新增的不可控泄漏。常态下 WS echo
        // 必达（cses-im-server 实测真广播 echo 给发送者）→ reconcile_post_echo 即时清栈，无泄漏。
        ps.mark_sent_pending_server_id(reconcile_corr, out);
    }
}

/// 从 `posts/create` 裸 Go 响应 body 防御性抠 server post id。
///
/// 真 Go 响应 id 落点不固定，依次尝试 `data.id` → `data.post.id` → 顶层 `id`；取第一个合法 26 字符
/// Id26 即用。全 miss / 非法 → `None`（调用方仍按 temporary_id 标 Sent，serverId 后补）。
/// 边界零信任：非字符串 / 非法 Id26 不 panic，降级 `None`。
fn extract_post_server_id(body: &serde_json::Value) -> Option<ServerId> {
    let candidate = body
        .pointer("/data/id")
        .or_else(|| body.pointer("/data/post/id"))
        .or_else(|| body.get("id"))
        .and_then(serde_json::Value::as_str)?;
    ServerId::from_str(candidate)
}
