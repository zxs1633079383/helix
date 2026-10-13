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

use crate::state::{CorrelationContext, Seq, ServerId, TemporaryId};
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
/// 完成 cursor 推进 + 落库（ON CONFLICT temporary_id 覆盖乐观行）。`event_to_upsert_op`
/// 会为服务端确认的新行写 `send_status='sent'`，但冲突更新显式排除该本地权威列，也不会取消
/// 15s timer——这两件事是 helix 把现网前端的乐观态/超时下沉进 core 后新增的职责。本函数补齐：
///
/// - `temporary_id` 命中在途 `PendingSend` 时 → `ps.reconcile`（吐 Persist 覆写
///   `send_status='sent'` + `id=server_id` ON CONFLICT(temporary_id) + CancelTimer 撤 15s，
///   状态推进到 Sent）。
/// - 之后从 `pending_sends` + `corr_map`(OptimisticSend) 移除该记录（防泄漏）。
///
/// 普通自发 WS 回声在发送对账 PersistOk 后独立发出 `im:post:sent`，不等待频道缺号补齐。
/// 同一 `post` handler 仍按 gate 完成原子提交和 `im:post:received`，不额外请求 latest。
///
/// **不推进 cursor**：cursor 已由调用方 `ch.ingest` 推进（HX-C008 单调，避免双推）。
/// 非 echo（`temporary_id` 空 / 不在 pending_sends / `id` 非合法 26 字符 ServerId）→ no-op，
/// 与他人推送的新消息一致。边界零信任不 panic。
pub(super) fn reconcile_post_echo(
    ctx: &mut ImWsContext<'_>,
    temporary_id: &str,
    server_id: Option<ServerId>,
    out: &mut EffectSink,
) -> Result<(), crate::ImError> {
    // 空 temporary_id（他人推送的新消息）→ 非 echo，不动发送态。
    if temporary_id.is_empty() {
        return Ok(());
    }
    // server_id 必须是合法 26 字符 Id26（reconcile 覆写 message.id 必须用合法 server id；
    // 非法/缺失 → 不 reconcile，留待 sync 兜底覆盖）。
    let Some(server_id) = server_id else {
        return Ok(());
    };
    let tmp = TemporaryId(temporary_id.to_string());
    // 仅当命中在途 PendingSend 才对账（先分配 corr 避免双重可变借用）。
    if !ctx.state.pending_sends.contains_key(&tmp) {
        return Ok(());
    }
    // 编码先于 pending/timer 变更；P1 body 已持久化，sent 只补充权威 id 和发送终态。
    let terminal_event = ctx
        .state
        .pending_sends
        .get(&tmp)
        .and_then(|pending| pending.body.as_ref().map(|body| (pending, body)))
        .map(|(pending, body)| {
            crate::event::post::sent_from_local_body(
                pending.timeline_readback.causation_id.as_deref(),
                temporary_id,
                server_id.as_str(),
                ctx.auth_user_id,
                body,
            )
            .map(|event| event.into_bytes())
        })
        .transpose()?;
    let reconcile_corr = ctx.alloc_corr();
    // R4-retain：取出该 send 的 P1 / H1 corr（send 时回填），用于 O(1) 清路由。
    let (persist_corr, outbound_http_corr) = ctx
        .state
        .pending_sends
        .get(&tmp)
        .map(|ps| (ps.persist_corr, ps.outbound_http_corr))
        .unwrap_or_default();
    if let Some(ps) = ctx.state.pending_sends.get_mut(&tmp) {
        ps.reconcile(server_id, reconcile_corr, out);
    }
    // 对账落定后移除（防 PendingSend / corr_map 泄漏；late timer 已被 CancelTimer 撤销，
    // 即便竞态到达也因 status==Sent 幂等无害）。
    ctx.state.pending_sends.remove(&tmp);
    // 对账落定后移除该 send 的路由，迟到的 HTTP Err 不得覆盖权威成功。
    if let Some(corr) = persist_corr {
        ctx.state.corr_map.remove(&corr);
    }
    if let Some(corr) = outbound_http_corr {
        ctx.state.corr_map.remove(&corr);
    }
    // 发送终态等待自己的 PersistOk；频道 gate 的 received 仍独立等待连续提交。
    let continuation = match terminal_event {
        Some(terminal_event) => CorrelationContext::AuthoritativeSendTerminalPersist {
            temporary_id: tmp,
            terminal_event: bytes::Bytes::from(terminal_event),
        },
        None => CorrelationContext::AuthoritativeSendReconcilePersist { temporary_id: tmp },
    };
    ctx.state.corr_map.insert(reconcile_corr, continuation);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_self_echo_keeps_its_sequence_for_the_shared_gate() {
        let event = EventEnvelope::new(
            crate::state::test_channel_id(9),
            Seq(22),
            crate::sync_session::EventKind::PostUpsert,
            crate::sync_session::PostFields {
                id: "post-gap-echo".to_string(),
                msg_type: "VOTE".to_string(),
                props: r#"{"vote":{"state":0,"items":[],"options":[]}}"#.to_string(),
                ..Default::default()
            },
        )
        .with_msg_id(Some("post-gap-echo".to_string()))
        .with_viewer_user_id("viewer-1");

        assert_eq!(event.seq, Seq(22));
    }

    #[test]
    fn self_echo_terminal_persist_does_not_schedule_latest_timeline_refresh() {
        let temporary_id = TemporaryId("temporary-echo-1".to_string());
        let mut state = crate::state::ImState::new();
        let mut pending = crate::pending_send::PendingSend::new(
            temporary_id.clone(),
            helix_core::TimerId::from_raw(91),
            None,
        );
        pending.timeline_readback.causation_id = Some("send-request-echo".to_string());
        state.pending_sends.insert(temporary_id.clone(), pending);
        let mut next_corr = 700_u64;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut sink = EffectSink::new();

        {
            let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);
            reconcile_post_echo(
                &mut ctx,
                temporary_id.0.as_str(),
                Some(crate::state::test_server_id(41)),
                &mut sink,
            )
            .expect("echo reconciliation");
        }

        let terminal_corr = sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                helix_core::Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("self echo writes the terminal sent fact");
        assert_eq!(
            state.corr_map.get(&terminal_corr),
            Some(&CorrelationContext::AuthoritativeSendReconcilePersist {
                temporary_id: temporary_id.clone(),
            }),
            "普通 WS echo 只完成发送对账，timeline 由 post 权威投影更新"
        );
    }

    #[test]
    // 回归锚: G03a（WS echo 不因 retry window 触发第二次 latest 查询）
    fn retry_self_echo_does_not_schedule_a_second_latest_query() {
        let temporary_id = TemporaryId("temporary-retry-echo".to_string());
        let mut state = crate::state::ImState::new();
        let mut pending = crate::pending_send::PendingSend::new(
            temporary_id.clone(),
            helix_core::TimerId::from_raw(92),
            None,
        );
        pending.timeline_readback = crate::pending_send::TimelineReadbackContext {
            window_token: Some("retry-echo-window".to_string()),
            causation_id: Some("retry-echo-request".to_string()),
        };
        state.pending_sends.insert(temporary_id.clone(), pending);

        let mut next_corr = 210;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(next_corr);
            next_corr += 1;
            corr
        };
        let mut sink = EffectSink::new();
        {
            let mut ctx = ImWsContext::new(&mut state, 1_000, "", "", &mut alloc_corr);
            reconcile_post_echo(
                &mut ctx,
                temporary_id.0.as_str(),
                Some(crate::state::test_server_id(42)),
                &mut sink,
            )
            .expect("echo reconciliation");
        }

        let terminal_corr = sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                helix_core::Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("retry self echo writes the terminal sent fact");
        assert_eq!(
            state.corr_map.get(&terminal_corr),
            Some(&CorrelationContext::AuthoritativeSendReconcilePersist {
                temporary_id: temporary_id.clone(),
            })
        );
    }
}
