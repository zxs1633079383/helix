//! `post_pin` action handler（置顶/取消置顶，WS 全量 19 事件补齐，P4）。
//!
//! 行为真源（现网 source of truth）：cses-client `router.rs` **未**单列 `post_pin` arm
//! → 落 `_ => { log unknown_action; vec![] }`（router.rs:46-57）。置顶态变更实际经
//! `im:channel:update`（`update_channel` 帧 payload.channel 带 pinned）刷新前端
//! （`message-v3/docs/case/12-顶部置顶.md:37`），`post_pin` 帧本身无独立落库/emit 副作用。
//!
//! 故 helix 对齐真源：注册为 **graceful no-op**——帧经 registry 正常路由（不冒
//! `UnsupportedWsAction` 硬错，对齐 module.rs dispatch 契约），无业务 Effect。
//! 后端 `cses_channel.go:1840` 确有 `WebsocketEventPostPin` 推送，但现网前端不消费 →
//! 不实现业务逻辑（YAGNI；第二个真实消费方出现前不抽，CLAUDE.md §1.4）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const POST_PIN_ACTION: &str = "post_pin";

struct PostPinHandler;

impl WsMessageHandler for PostPinHandler {
    fn action(&self) -> &'static str {
        POST_PIN_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 真源 default vec![]：no-op（pin 态走 update_channel 刷新，本帧无独立副作用）。
        Ok(())
    }
}

static POST_PIN_HANDLER: PostPinHandler = PostPinHandler;

#[distributed_slice(WS_HANDLERS)]
static POST_PIN_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: POST_PIN_ACTION,
    handler: &POST_PIN_HANDLER,
};
