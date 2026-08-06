//! `announcementChanged` WS 轻量版本通知。
//!
//! 帧只携 channelId/version；Helix 以每频道单调账本去重，并复用既有
//! `im_announcement_list` Announcement Core 发起一次权威全量重拉。

use helix_core::EffectSink;
use serde_json::json;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const ANNOUNCEMENT_CHANGED_ACTION: &str = "announcementChanged";

struct AnnouncementChangedHandler;

impl WsMessageHandler for AnnouncementChangedHandler {
    fn action(&self) -> &'static str {
        ANNOUNCEMENT_CHANGED_ACTION
    }

    /// 过滤旧/重复版本，并把更高版本转成同一 Announcement list HTTP 查询。
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
        let Some(version) = data
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .filter(|version| *version > 0)
        else {
            return Ok(());
        };
        let committed = ctx
            .state
            .announcement_versions
            .get(&channel_id)
            .copied()
            .unwrap_or_default();
        let pending = ctx
            .state
            .announcement_reload_versions
            .get(&channel_id)
            .copied()
            .unwrap_or_default();
        if version <= committed || version <= pending {
            return Ok(());
        }

        let corr = ctx.alloc_corr();
        let req_id = format!("announcement-refresh-{}", corr.raw());
        let payload = serde_json::to_vec(&json!({
            "channel_id": channel_id.as_str(),
            "req_id": req_id,
        }))
        .map_err(|error| ImError::Parse(format!("announcement refresh payload: {error}")))?;
        let effects = crate::commands::handle_outbound(
            "im_announcement_list",
            &payload,
            ctx.api_base_url,
            ctx.api_base_url,
            ctx.state.connection_id.as_deref(),
            corr,
        )?;
        for effect in effects {
            out.push(effect);
        }
        ctx.state
            .announcement_reload_versions
            .insert(channel_id, version);
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::OutboundReadReply {
                req_id,
                command: "im_announcement_list".to_string(),
                channel_id: Some(channel_id.as_str().to_string()),
            },
        );
        Ok(())
    }
}

static ANNOUNCEMENT_CHANGED_HANDLER: AnnouncementChangedHandler = AnnouncementChangedHandler;

inventory::submit! {
    WsHandlerRegistration {
        action: ANNOUNCEMENT_CHANGED_ACTION,
        handler: &ANNOUNCEMENT_CHANGED_HANDLER,
    }
}

#[cfg(target_arch = "wasm32")]
/// 在 wasm inventory 不可自动发现时保留静态 handler。
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&ANNOUNCEMENT_CHANGED_HANDLER);
}
