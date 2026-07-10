//! `handle_port_reply` 双 emit 读族臂的 emit helper（从 port_reply.rs 外提，收口 port_reply.rs 行数基线）。
//!
//! S7 byIds 成员快照（issue #56）/ S8 回复族（issue #57）两条读族回报臂同构：剥 ADR-007 信封 →
//! ① 透传 `im:read:result{req_id, body}`（冻结 verbatim ② 契约面）② 解析 body emit render-ready 投影。
//! 信封畸形 / Err → 回灌 read error（不丢 req_id·前端 reject）+（回复族）补发空 replyIds 清抽屉。
//! 纯搬运·零行为变更（逻辑逐字对应原 port_reply.rs 两臂）。

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

/// S8 回复族（UC-2.4·issue #57·终局切）：getReplies/getReplyBranch HTTP 回报 → 双 emit。
/// Ok → 剥信封得裸响应体 → emit `im:read:result{req_id, body}`（冻结 verbatim ② 契约面照旧）
///   + emit `im:channel:replies{reqId, replyIds}`（render-ready·helix 抽好 postId·壳纯绑定）。
/// 信封畸形 / Err → emit `im:read:result{req_id, error}` + emit 空 replyIds（前端 reject·清抽屉）。
pub(crate) fn emit_replies(req_id: &str, outcome: &PortOutcome, out: &mut EffectSink) {
    match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                out.push(crate::read_relay::emit_read_result(
                    req_id,
                    raw_body.as_ref(),
                ));
                let body: serde_json::Value =
                    serde_json::from_slice(raw_body.as_ref()).unwrap_or(serde_json::Value::Null);
                let ids = crate::render_ready_replies::extract_reply_ids(&body);
                out.push(crate::render_ready_replies::emit_channel_replies(
                    req_id, ids,
                ));
            }
            Err(e) => {
                tracing::warn!(req_id, error = ?e, "replies reply envelope decode failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "response envelope decode failed",
                ));
                out.push(crate::render_ready_replies::emit_channel_replies(
                    req_id,
                    Vec::new(),
                ));
            }
        },
        PortOutcome::Err(e) => {
            tracing::warn!(req_id, error = ?e, "replies http failed");
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "http request failed",
            ));
            out.push(crate::render_ready_replies::emit_channel_replies(
                req_id,
                Vec::new(),
            ));
        }
    }
}

/// G-11 单消息定位读回：保留通用 read result，并额外输出 Helix 已整形的 highlighted 行。
pub(crate) fn emit_located_post(req_id: &str, outcome: &PortOutcome, out: &mut EffectSink) {
    match outcome {
        PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw_body) => {
                out.push(crate::read_relay::emit_read_result(
                    req_id,
                    raw_body.as_ref(),
                ));
                if let Some(effect) =
                    crate::render_ready_locate::emit_located_post(raw_body.as_ref())
                {
                    out.push(effect);
                }
            }
            Err(e) => {
                tracing::warn!(req_id, error = ?e, "locate post reply envelope decode failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "response envelope decode failed",
                ));
            }
        },
        PortOutcome::Err(e) => {
            tracing::warn!(req_id, error = ?e, "locate post http failed");
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "http request failed",
            ));
        }
    }
}
