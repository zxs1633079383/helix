//! `update_channel_notice` action handler（群公告变更，WS 全量 19 事件补齐，P4）。
//!
//! 行为真源：cses-client `router.rs` 未单列此 arm → 落 `_ => vec![]` no-op（router.rs:46）。
//! 后端 `cses_channel.go:1190 publishChangeChannelNotice` 推 `WebsocketEventUpdateChannelNotice`，
//! wire `data` 含 `{channelId, content}`（见 full-map partials/5 §2.15）；群公告本地态实际经
//! `update_channel`（path2 PATCH，notice 列）刷新，本帧无独立落库副作用。
//!
//! 故 helix 对齐真源：graceful no-op（经 registry 路由不冒 `UnsupportedWsAction`，无业务 Effect）。
//! YAGNI：现网前端不单独消费此帧 → 不实现业务逻辑（CLAUDE.md §1.4）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const UPDATE_CHANNEL_NOTICE_ACTION: &str = "update_channel_notice";

struct UpdateChannelNoticeHandler;

impl WsMessageHandler for UpdateChannelNoticeHandler {
    fn action(&self) -> &'static str {
        UPDATE_CHANNEL_NOTICE_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 真源 default vec![]：no-op（公告态由 update_channel PATCH 刷新）。
        Ok(())
    }
}

static UPDATE_CHANNEL_NOTICE_HANDLER: UpdateChannelNoticeHandler = UpdateChannelNoticeHandler;

#[distributed_slice(WS_HANDLERS)]
static UPDATE_CHANNEL_NOTICE_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: UPDATE_CHANNEL_NOTICE_ACTION,
    handler: &UPDATE_CHANNEL_NOTICE_HANDLER,
};
