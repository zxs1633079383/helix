//! `handle_port_reply` 双 emit 读族臂的 emit helper（从 port_reply.rs 外提，收口 port_reply.rs 行数基线）。
//!
//! S7 byIds 成员快照仍保留双 emit；G12 回复族只把 authority body 规范化为 Helix 内部投影，
//! 随后由对应 Gate 在持久化读回后发布唯一 MessageV3 业务事件。

use crate::http_envelope::unwrap_sync_envelope;
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;

/// S7（issue #56）：byIds 成员快照读族回报 → 双 emit。
/// Ok → ① 透传 im:read:result{req_id, body}（UC-6.4 ② 冻结契约面）；
///       ② 解析 byIds body（map[channelId][]）逐 channel emit render-ready im:channel:members
///       （壳退纯绑定·载族自愈）。信封畸形 / Err → 仅回灌 read error（不丢 req_id，前端 reject）。
pub(crate) fn emit_members_by_ids(req_id: &str, outcome: &PortOutcome, out: &mut EffectSink) {
    match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                out.push(crate::read_relay::emit_read_result(
                    req_id,
                    raw_body.as_ref(),
                ));
                let body: serde_json::Value =
                    serde_json::from_slice(raw_body.as_ref()).unwrap_or(serde_json::Value::Null);
                for (cid, members) in crate::render_ready_members::members_from_byids_body(&body) {
                    out.push(crate::render_ready_members::emit_channel_members(
                        &cid,
                        members,
                        Vec::new(),
                        crate::render_ready_members::MemberProjectionMode::Snapshot,
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(req_id, error = ?e, "byIds members reply envelope decode failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "response envelope decode failed",
                ));
            }
        },
        PortOutcome::Err(e) => {
            tracing::warn!(req_id, error = ?e, "byIds members http failed");
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "http request failed",
            ));
        }
    }
}

pub(crate) fn emit_contact_candidates(req_id: &str, outcome: &PortOutcome, out: &mut EffectSink) {
    match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                let body: serde_json::Value =
                    serde_json::from_slice(raw_body.as_ref()).unwrap_or(serde_json::Value::Null);
                out.push(crate::render_ready_candidates::emit(req_id, &body));
            }
            Err(_) => out.push(crate::render_ready_candidates::emit_error(
                req_id,
                "response envelope decode failed",
            )),
        },
        PortOutcome::Err(_) => out.push(crate::render_ready_candidates::emit_error(
            req_id,
            "http request failed",
        )),
    }
}

/// 将 G12 HTTP authority 规范化为内部 reply projection；失败不产生 raw 渲染事件。
pub(crate) fn normalize_replies(
    request: &crate::render_ready_replies::ReplyProjectionRequest,
    outcome: &PortOutcome,
    revisions: &mut std::collections::HashMap<String, u64>,
    seen_ids: &mut std::collections::HashMap<String, std::collections::HashSet<String>>,
) -> Option<crate::render_ready_replies::FlatReplyProjection> {
    let req_id = request.req_id.as_str();
    match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                let body: serde_json::Value = match serde_json::from_slice(raw_body.as_ref()) {
                    Ok(body) => body,
                    Err(error) => {
                        tracing::warn!(req_id, error = ?error, "replies authority JSON decode failed");
                        return None;
                    }
                };
                let mut projection = crate::render_ready_replies::finalize_projection(
                    request,
                    crate::render_ready_replies::extract_flat_replies(
                        &body,
                        request.viewer_user_id.as_str(),
                    ),
                );
                if crate::render_ready_replies::accept_projection(
                    &mut projection,
                    revisions,
                    seen_ids,
                ) {
                    return Some(projection);
                }
            }
            Err(e) => {
                tracing::warn!(req_id, error = ?e, "replies reply envelope decode failed");
            }
        },
        PortOutcome::Err(e) => {
            tracing::warn!(req_id, error = ?e, "replies http failed");
        }
    }
    None
}

/// 将 createPosts admission 回包收敛为 MessageV3 batch result。
pub(crate) fn emit_create_posts_result(
    req_id: &str,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), crate::ImError> {
    let event = match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                let body =
                    serde_json::from_slice(raw_body.as_ref()).unwrap_or(serde_json::Value::Null);
                crate::event::post::batch_result_from_authority(req_id, &body)?
            }
            Err(_) => crate::event::post::batch_error(req_id, "response envelope decode failed")?,
        },
        PortOutcome::Err(_) => crate::event::post::batch_error(req_id, "http request failed")?,
    };
    out.push(event.into_effect());
    Ok(())
}

/// 回灌读取同步结果，并将每条回执分别发布为 canonical `im:post:readers` 事件。
pub(crate) fn emit_post_readers(req_id: &str, outcome: &PortOutcome, out: &mut EffectSink) {
    match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                out.push(crate::read_relay::emit_read_result(
                    req_id,
                    raw_body.as_ref(),
                ));
                let body =
                    serde_json::from_slice(raw_body.as_ref()).unwrap_or(serde_json::Value::Null);
                for event in crate::query::render_ready::receipts::emit(&body) {
                    out.push(event);
                }
            }
            Err(_) => {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "response envelope decode failed",
                ));
            }
        },
        PortOutcome::Err(_) => {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "http request failed",
            ));
        }
    }
}
