//! `channel_close` action handler（关闭频道：delete_at + is_active 置位，spec §S1）。
//!
//! 行为真源：现网 `handlers/channel.rs:575 handle_close` → `Channel::update_fields(delete_at,
//! is_active=false)` → emit `im:channel:closed{channelId, deleteAt}`。
//!
//! wire（`CloseChannelData` types.rs:374）：`channelId` / `deleteAt`。helix：定点列 patch
//! （`channel_set_cols`：UPDATE channel SET delete_at=?, is_active=0 WHERE id=?）。本地无此行
//! → 0 行命中 no-op（幂等）。spec §S3.4 纯本地字段 is_remove 不在此路径动。

use helix_core::effect::SqlValue;
use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_CLOSE_ACTION: &str = "channel_close";

struct ChannelCloseHandler;

impl WsMessageHandler for ChannelCloseHandler {
    fn action(&self) -> &'static str {
        CHANNEL_CLOSE_ACTION
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

        let delete_at = data
            .get("deleteAt")
            .or_else(|| data.get("delete_at"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        // delete_at + is_active=0 定点 patch（对齐现网 update_fields）。
        out.push(crate::acl::to_effect_s1::channel_set_cols(
            channel_id,
            vec![
                ("delete_at", SqlValue::Integer(delete_at)),
                ("is_active", SqlValue::Integer(0)),
            ],
        ));
        if let Some(scope) = ctx.state.dialog_list_frame.scope().cloned() {
            if let Some(frame) = ctx
                .state
                .dialog_list_frame
                .patch_closed(scope, channel_id.as_str())
            {
                out.push(frame);
            }
        }
        out.push(crate::acl::to_effect_s1::emit_channel_closed(
            channel_id, delete_at,
        ));
        Ok(())
    }
}

static CHANNEL_CLOSE_HANDLER: ChannelCloseHandler = ChannelCloseHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_CLOSE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_CLOSE_ACTION,
        handler: &CHANNEL_CLOSE_HANDLER,
    }
}
