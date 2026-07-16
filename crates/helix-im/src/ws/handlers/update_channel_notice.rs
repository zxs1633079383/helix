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
            .or_else(|| data.get("id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        out.push(crate::acl::to_effect::emit_channel_update(channel_id));
        let settings = data
            .get("settings")
            .filter(|value| value.is_object())
            .unwrap_or(data);
        if let Some(setting_version) = data
            .get("settingVersion")
            .or_else(|| data.get("setting_version"))
            .and_then(serde_json::Value::as_u64)
        {
            let role = ctx
                .state
                .viewer_channel_roles
                .get(&channel_id)
                .map(String::as_str);
            let capabilities =
                crate::channel_capabilities::for_viewer(data.get("capabilitiesByRole"), role);
            if let Some(effect) = crate::acl::to_effect_s1::emit_channel_settings_updated(
                channel_id,
                settings,
                setting_version,
                capabilities,
            ) {
                out.push(effect);
            }
        }
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
