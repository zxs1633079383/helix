//! `update_channel` action handler（S3 path2：channel PATCH）。
//!
//! 行为真源：现网 `channel_service.rs:100 apply_update` → `collect_present_fields`（只收 WS
//! `Some` 字段）→ `update_partial`（参数化 UPDATE + 白名单）。helix：`channel_write::collect_present`
//! 产白名单列集 → `to_effect::update_channel_partial`（UPDATE … WHERE id=?）。
//!
//! `None`/缺省字段不进 SET（DB 既有值保留）；空列集 → 无写意图（None，no-op）。
//! 成员 / owner / admin_users / boss 不走此路径（独立成员表，对齐现网 update_partial filter）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const UPDATE_CHANNEL_ACTION: &str = "update_channel";

struct UpdateChannelHandler;

impl WsMessageHandler for UpdateChannelHandler {
    fn action(&self) -> &'static str {
        UPDATE_CHANNEL_ACTION
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
        // channel id 硬 gate（缺 / 非 26 字符 → no-op，边界零信任，helix-im 不变量 4）。
        let Some(channel_id) = data
            .get("id")
            .or_else(|| data.get("channelId"))
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        // 收 Some 字段 → PATCH（空集 = 无可写字段，update_channel_partial 返回 None → 不 push）。
        let cols = crate::channel_write::collect_present(data);
        if let Some(eff) = crate::acl::to_effect::update_channel_partial(channel_id, cols) {
            out.push(eff);
        }
        Ok(())
    }
}

static UPDATE_CHANNEL_HANDLER: UpdateChannelHandler = UpdateChannelHandler;

#[distributed_slice(WS_HANDLERS)]
static UPDATE_CHANNEL_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: UPDATE_CHANNEL_ACTION,
    handler: &UPDATE_CHANNEL_HANDLER,
};
