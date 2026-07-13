//! `change_channel_approval` action handler（开/关发言审批，WS 全量 19 事件补齐，P4）。
//!
//! ⚠️ wire action 值 = `change_channel_approval`（**无 Status 后缀**）——后端常量名
//! `WebsocketEventChangeChannelApprovalStatus` 末尾带 `Status`，但 `NewWebSocketEvent` 注入的
//! action 字符串值无后缀（full-map partials/5 §2.17 注实证：`cses_channel.go:1073`
//! `publishChangeApprovalStatusMessage`）。注册 action 必须照 wire 值，不照常量名。
//!
//! 行为真源：cses-client `router.rs` 未单列此 arm → 落 `_ => vec![]` no-op（router.rs:46）。
//! wire `data` = `{channelId, approvalStatus:bool, message}`；审批态本地实际经 `update_channel`
//! （approval 列）刷新，本帧无独立落库副作用。
//!
//! 故 helix 对齐真源：graceful no-op（经 registry 路由不冒 `UnsupportedWsAction`，无业务 Effect）。
//! YAGNI：现网前端不单独消费此帧 → 不实现业务逻辑（AGENTS.md §1.4）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

/// wire 值无 Status 后缀（常量名带 Status 是后端命名陷阱，full-map §2.17 实证）。
const CHANGE_APPROVAL_ACTION: &str = "change_channel_approval";

struct ChangeApprovalHandler;

impl WsMessageHandler for ChangeApprovalHandler {
    fn action(&self) -> &'static str {
        CHANGE_APPROVAL_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 真源 default vec![]：no-op（审批态由 update_channel PATCH 刷新）。
        Ok(())
    }
}

static CHANGE_APPROVAL_HANDLER: ChangeApprovalHandler = ChangeApprovalHandler;

#[distributed_slice(WS_HANDLERS)]
static CHANGE_APPROVAL_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: CHANGE_APPROVAL_ACTION,
    handler: &CHANGE_APPROVAL_HANDLER,
};
