//! `post_pin` action handler（置顶/取消置顶）。
//!
//! cses-im-server 当前真实帧为 `{postId, channelId, operation}`，HTTP 写入成功后广播
//! `post_pin`。该帧没有完整 PostFields，因此本 handler 只持久化 pinned patch；完整的
//! render-ready `im:post:updated` 必须由同时产生的 NOTICE `props.content` 投影，不能用
//! `PostFields::default()` 伪造 `createAt` 等时间线字段。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

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
        let pinned_projection_invalidation = ctx
            .state
            .invalidate_pinned_projection(ctx.auth_user_id, channel_id);
        let Some(post_id) = data
            .get("postId")
            .or_else(|| data.get("post_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Ok(());
        };
        let Some(pinned) = data
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .and_then(|operation| match operation {
                "add" => Some(true),
                "remove" => Some(false),
                _ => None,
            })
        else {
            // Go 只发布 add/remove；未知操作不能默认为 add，避免错误帧篡改 pinned 事实。
            return Ok(());
        };

        // 裸 post_pin 没有正文与时间戳，只写 pinned；完整 fat projection 由 NOTICE 负责。
        out.push(helix_core::Effect::PersistFire {
            ops: vec![crate::channel::pin_state_op(post_id, pinned)],
        });
        if let Some(effect) = pinned_projection_invalidation {
            out.push(effect);
        }
        Ok(())
    }
}

static POST_PIN_HANDLER: PostPinHandler = PostPinHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POST_PIN_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: POST_PIN_ACTION,
        handler: &POST_PIN_HANDLER,
    }
}
