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

/// 注册一条 send 的两条 corr 路由：P1 `OptimisticSend`（乐观落库回报）+ H1 `OutboundSendHttp`
/// （posts/create HTTP 200 兜底对账）。
///
/// **F1 修复点**：两条都注册——H1 漏注册时 posts/create HTTP 200 回报 `corr_map.remove(h1_corr)`
/// 返 None → 响应静默丢弃 → 真 Go self-omit echo 下乐观行永停 `Sending` 到 15s spinner。
pub(crate) fn register_send_corrs(
    state: &mut ImState,
    p1_corr: Correlation,
    h1_corr: Correlation,
    temporary_id: String,
) {
    state.corr_map.insert(
        p1_corr,
        CorrelationContext::OptimisticSend {
            temporary_id: TemporaryId(temporary_id.clone()),
        },
    );
    state.corr_map.insert(
        h1_corr,
        CorrelationContext::OutboundSendHttp {
            temporary_id: TemporaryId(temporary_id),
        },
    );
}

/// `posts/create` HTTP 200 兜底对账：解 ADR-007 信封 → 裸 Go body `{status, data?}`。
///
/// - `status == "SUCCESS"` + 含 server post id（防御取 `data.id` / `data.post.id` / `id`）
///   → `pending_send.reconcile(server_id)`（吐 Persist `id=serverId,status=sent` + CancelTimer，
///   推进 `Sent`）。
/// - `status == "SUCCESS"` 但无可解 serverId → `mark_sent_pending_server_id`（仅 `status=sent`
///   + CancelTimer + `Sent`；serverId 后续 history/WS echo 按 temporary_id 幂等补）。
/// - `status != "SUCCESS"`（含 4xx body 误判）/ 信封畸形 → 不标 Sent（边界零信任不 panic，
///   留 15s timer 兜底标 UnSend）。
///
/// `reconcile_corr` 由调用方分配（单调分配器，HX-C005）。对账落定后清理 pending_sends + corr_map
/// （防泄漏，对齐 echo 路径）。
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

    // R4-retain：取出该 send 的 P1 Persist corr（清 corr_map 用）。
    let persist_corr = state
        .pending_sends
        .get(temporary_id)
        .and_then(|ps| ps.persist_corr);

    if let Some(server_id) = extract_post_server_id(&body) {
        // 含 serverId → reconcile：Persist(id=serverId,status=sent) + CancelTimer + Sent。
        if let Some(ps) = state.pending_sends.get_mut(temporary_id) {
            ps.reconcile(server_id, reconcile_corr, out);
        }
    } else if let Some(ps) = state.pending_sends.get_mut(temporary_id) {
        // SUCCESS 但无 serverId → 仅标 Sent + persist 乐观行 status=sent（serverId 后补）+ 撤 timer。
        ps.mark_sent_pending_server_id(reconcile_corr, out);
    }

    // 对账落定 → 清理 pending_sends + corr_map（防泄漏，对齐 echo 路径；late timer 已 CancelTimer）。
    state.pending_sends.remove(temporary_id);
    if let Some(corr) = persist_corr {
        state.corr_map.remove(&corr);
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
