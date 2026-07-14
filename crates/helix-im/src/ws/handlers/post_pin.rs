//! `post_pin` action handler（置顶/取消置顶）。
//!
//! cses-im-server 当前真实帧为 `{postId, channelId, operation}`，HTTP 写入成功后广播
//! `post_pin`。本 handler 将其转成 message 行元数据 patch + render-ready
//! `im:post:updated`，前端只直绑 `data-pinned`，不做乐观合成。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;
use crate::sync_session::PostFields;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POST_PIN_ACTION: &str = "post_pin";

struct PostPinHandler;

impl WsMessageHandler for PostPinHandler {
    fn action(&self) -> &'static str {
        POST_PIN_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };
        ctx.state.invalidate_recent_message_coverage(channel_id);
        let Some(post_id) = data
            .get("postId")
            .or_else(|| data.get("post_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Ok(());
        };
        let pinned = data
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .map(|op| op != "remove")
            .unwrap_or(true);

        out.push(helix_core::Effect::PersistFire {
            ops: vec![crate::channel::pin_state_op(post_id, pinned)],
        });

        let fields = PostFields {
            id: post_id.to_string(),
            channel_id: channel_id.as_str().to_string(),
            props: serde_json::json!({ "pinned": pinned }).to_string(),
            ..PostFields::default()
        };
        out.push(crate::acl::to_effect::emit_post_updated(
            channel_id,
            frame.event_seq().map(|seq| seq.0).unwrap_or(0),
            post_id,
            &fields,
        ));
        Ok(())
    }
}

static POST_PIN_HANDLER: PostPinHandler = PostPinHandler;

inventory::submit! {
    WsHandlerRegistration {
        action: POST_PIN_ACTION,
        handler: &POST_PIN_HANDLER,
    }
}
