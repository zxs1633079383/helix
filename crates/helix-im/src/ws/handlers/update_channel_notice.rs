//! `update_channel_notice` action handler（群公告变更，WS 全量 19 事件补齐，P4）。
//!
//! 后端 `cses_channel.go:1190 publishChangeChannelNotice` 推 `WebsocketEventUpdateChannelNotice`，
//! wire `data` 含 `{channelId, content}`（见 full-map partials/5 §2.15）。
//!
//! 移动端契约要求该帧转为 `im:channel:update` thin 信号，通知 UI 重查 dialog projection。
//! 公告正文仍以后端 / SQLite 投影为准，本 handler 不伪造 channel 行，只发领域更新信号。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const UPDATE_CHANNEL_NOTICE_ACTION: &str = "update_channel_notice";

struct UpdateChannelNoticeHandler;

impl WsMessageHandler for UpdateChannelNoticeHandler {
    fn action(&self) -> &'static str {
        UPDATE_CHANNEL_NOTICE_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .or_else(|| data.get("id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        out.push(crate::acl::to_effect::emit_channel_update(channel_id));
        Ok(())
    }
}

static UPDATE_CHANNEL_NOTICE_HANDLER: UpdateChannelNoticeHandler = UpdateChannelNoticeHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&UPDATE_CHANNEL_NOTICE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: UPDATE_CHANNEL_NOTICE_ACTION,
        handler: &UPDATE_CHANNEL_NOTICE_HANDLER,
    }
}
