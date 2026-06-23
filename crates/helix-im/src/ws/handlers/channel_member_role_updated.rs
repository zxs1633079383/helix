//! `channel_member_role_updated` action handler（更新群成员角色，WS 全量 19 事件补齐，P4）。
//!
//! 行为真源：cses-client `router.rs` 未单列此 arm → 落 `_ => vec![]` no-op（router.rs:46）。
//! 后端 `cses_channel.go:1448` 推 `WebsocketEventChannelMemberRoleUpdated`，wire `data` =
//! `{channelId, userIds:[], role}`（见 full-map partials/5 §2.12）；角色变更的本地态实际由
//! 后续 `channel_member_update`（全量 path1 upsert，含 role 列）覆盖，本帧无独立落库副作用。
//!
//! 故 helix 对齐真源：graceful no-op（经 registry 路由不冒 `UnsupportedWsAction`，无业务 Effect）。
//! YAGNI：现网前端不单独消费此帧 → 不实现业务逻辑（CLAUDE.md §1.4）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const ROLE_UPDATED_ACTION: &str = "channel_member_role_updated";

struct RoleUpdatedHandler;

impl WsMessageHandler for RoleUpdatedHandler {
    fn action(&self) -> &'static str {
        ROLE_UPDATED_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 真源 default vec![]：no-op（role 态由 channel_member_update 全量 upsert 覆盖）。
        Ok(())
    }
}

static ROLE_UPDATED_HANDLER: RoleUpdatedHandler = RoleUpdatedHandler;

#[distributed_slice(WS_HANDLERS)]
static ROLE_UPDATED_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: ROLE_UPDATED_ACTION,
    handler: &ROLE_UPDATED_HANDLER,
};
