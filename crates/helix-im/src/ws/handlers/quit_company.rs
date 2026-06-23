//! `quit_company` action handler（用户退出公司，WS 全量 19 事件补齐，P4）。
//!
//! 行为真源：cses-client `router.rs` 未单列此 arm → 落 `_ => vec![]` no-op（router.rs:46）。
//! 后端 `cses_team.go:120 leaveAllChannelsForTeam` 推 `WebsocketEventQuitCompany`，wire `data` =
//! `{channels: [channelMember]}`（受影响群成员列表，见 full-map partials/5 §2.18）；离群本地态
//! 实际由配套发的 `channel_close` / `channel_member_update`（memberChange.leave 物理删）覆盖，
//! 本帧无独立落库副作用。
//!
//! 故 helix 对齐真源：graceful no-op（经 registry 路由不冒 `UnsupportedWsAction`，无业务 Effect）。
//! YAGNI：现网前端不单独消费此帧 → 不实现业务逻辑（CLAUDE.md §1.4）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const QUIT_COMPANY_ACTION: &str = "quit_company";

struct QuitCompanyHandler;

impl WsMessageHandler for QuitCompanyHandler {
    fn action(&self) -> &'static str {
        QUIT_COMPANY_ACTION
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 真源 default vec![]：no-op（离群态由配套 channel_close / channel_member_update 覆盖）。
        Ok(())
    }
}

static QUIT_COMPANY_HANDLER: QuitCompanyHandler = QuitCompanyHandler;

#[distributed_slice(WS_HANDLERS)]
static QUIT_COMPANY_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: QUIT_COMPANY_ACTION,
    handler: &QUIT_COMPANY_HANDLER,
};
