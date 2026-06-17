use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::{ChannelId, Seq, ServerId, TemporaryId};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const POSTED_ACTION: &str = "posted";

struct PostedHandler;

impl WsMessageHandler for PostedHandler {
    fn action(&self) -> &'static str {
        POSTED_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let echo = parse_echo(frame)?;
        reconcile_echo(ctx, echo, out);
        Ok(())
    }
}

struct PostedEcho {
    channel_id: ChannelId,
    seq: Seq,
    temporary_id: String,
    server_id: ServerId,
}

fn parse_echo(frame: &WsFrame) -> Result<PostedEcho, ImError> {
    let data = frame.data_required()?;
    let post = data.get("post").unwrap_or(data);

    let Some(temporary_id) = post
        .get("temporary_id")
        .or_else(|| post.get("temporaryId"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return Err(ImError::UnsupportedWsAction("posted(non-echo)".to_string()));
    };

    let server_id = post
        .get("id")
        .and_then(serde_json::Value::as_str)
        .and_then(ServerId::from_str)
        .ok_or_else(invalid_posted_echo)?;

    let channel_id = post
        .get("channel_id")
        .or_else(|| post.get("channelId"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(invalid_posted_echo)?;

    let seq = data
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
        })
        .map(Seq)
        .ok_or_else(invalid_posted_echo)?;

    Ok(PostedEcho {
        channel_id,
        seq,
        temporary_id,
        server_id,
    })
}

fn invalid_posted_echo() -> ImError {
    ImError::InvalidWsFrame("invalid posted echo required fields".to_string())
}

fn reconcile_echo(ctx: &mut ImWsContext<'_>, echo: PostedEcho, out: &mut EffectSink) {
    let tmp_id = TemporaryId(echo.temporary_id);
    let reconcile_corr = ctx.alloc_corr();
    // R4-retain：取出该 send 的 P1 Persist corr（send 时回填），用于对账后 O(1) 清路由。
    let persist_corr = ctx
        .state
        .pending_sends
        .get(&tmp_id)
        .and_then(|ps| ps.persist_corr);
    if let Some(ps) = ctx.state.pending_sends.get_mut(&tmp_id) {
        ps.reconcile(echo.server_id, reconcile_corr, out);
        // R4-retain：对账落定后 O(1) 移除该 send 的 OptimisticSend 路由（防 corr_map 泄漏）。
        // PendingSend 本身**保留**（status=Sent 可查询，对齐 `posted` 路径既有契约：
        // 迟到 15s timer 凭 status==Sent 幂等无害）。
        if let Some(corr) = persist_corr {
            ctx.state.corr_map.remove(&corr);
        }
    }
    // cursor 独立推进（与 pending_sends 借用解耦，HX-C008）。
    if let Some(ch) = ctx.state.channels.get_mut(&echo.channel_id) {
        let _ = ch.cursor.try_advance(echo.seq);
        out.push(crate::acl::to_effect::advance_cursor(
            echo.channel_id,
            echo.seq,
        ));
    }
}

static POSTED_HANDLER: PostedHandler = PostedHandler;

#[distributed_slice(WS_HANDLERS)]
static POSTED_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POSTED_ACTION,
    handler: &POSTED_HANDLER,
};
