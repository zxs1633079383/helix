//! `channel_member_role_updated` action handler（更新群成员角色，WS 全量 19 事件补齐，P4）。
//!
//! 后端 `cses_channel.go:1448` 推 `WebsocketEventChannelMemberRoleUpdated`，wire `data` =
//! `{channelId, userIds:[], role}`（见 full-map partials/5 §2.12）。
//!
//! 移动端契约要求该帧转为 `im:channel:member-updated`。本 handler 不伪造成员表写入，
//! 只透传 raw role update 作为 thin bus 信号；权威成员表仍由 byIds / member_update 投影兑现。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const ROLE_UPDATED_ACTION: &str = "channel_member_role_updated";

struct RoleUpdatedHandler;

impl WsMessageHandler for RoleUpdatedHandler {
    fn action(&self) -> &'static str {
        ROLE_UPDATED_ACTION
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

        crate::channel_capabilities::remember_role_update(
            ctx.state,
            channel_id,
            data,
            ctx.auth_user_id,
        );

        let raw = serde_json::to_vec(data).unwrap_or_default();
        out.push(crate::acl::to_effect_s1::emit_channel_member_updated(
            channel_id, &raw,
        ));
        Ok(())
    }
}

static ROLE_UPDATED_HANDLER: RoleUpdatedHandler = RoleUpdatedHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&ROLE_UPDATED_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: ROLE_UPDATED_ACTION,
        handler: &ROLE_UPDATED_HANDLER,
    }
}
