//! `quit_company` action handler（用户退出公司，WS 全量 19 事件补齐，P4）。
//!
//! 行为真源：cses-client `router.rs` 未单列此 arm → 落 `_ => vec![]` no-op（router.rs:46）。
//! 后端 `cses_team.go:120 leaveAllChannelsForTeam` 推 `WebsocketEventQuitCompany`，wire `data` =
//! `{channels: [channelMember]}`（受影响群成员列表，见 full-map partials/5 §2.18）；离群本地态
//! 实际由配套发的 `channel_close` / `channel_member_update`（memberChange.leave 物理删）覆盖，
//! 本帧无独立落库副作用。
//!
//! helix 不伪造成员表写入：本帧只把 Go `channels[]` 转成移动端 render 所需的
//! `im:channel:member-updated` thin signal；离群本地态仍由配套 channel_close /
//! channel_member_update 覆盖。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const QUIT_COMPANY_ACTION: &str = "quit_company";

struct QuitCompanyHandler;

impl WsMessageHandler for QuitCompanyHandler {
    fn action(&self) -> &'static str {
        QUIT_COMPANY_ACTION
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
        let raw = serde_json::to_vec(data).unwrap_or_default();
        if let Some(channels) = data.get("channels").and_then(serde_json::Value::as_array) {
            for item in channels {
                if let Some(channel_id) = item
                    .get("channelId")
                    .or_else(|| item.get("channel_id"))
                    .or_else(|| item.get("id"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(ChannelId::from_str)
                {
                    out.push(crate::acl::to_effect_s1::emit_channel_member_updated(
                        channel_id, &raw,
                    ));
                }
            }
        }
        Ok(())
    }
}

static QUIT_COMPANY_HANDLER: QuitCompanyHandler = QuitCompanyHandler;

inventory::submit! {
    WsHandlerRegistration {
        action: QUIT_COMPANY_ACTION,
        handler: &QUIT_COMPANY_HANDLER,
    }
}
