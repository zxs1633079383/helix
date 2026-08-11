//! `update_channel_member_nickName` action handler（群昵称定点更新，spec §S1）。
//!
//! ⚠️ wire action 拼写 = `update_channel_member_nickName`（驼峰混 snake，**原样保留**，spec §S1
//! + §8 待核对点已坐实）。
//!
//! 行为真源：现网 `handlers/channel.rs:702 handle_member_nickname_update` →
//! `channel_member_repo::update_nickname`（复合 PK UPDATE）→ emit `im:channel:member-nickname`。
//!
//! wire：`{channelId, userId, nickName}`。helix：`update_member_nickname`（复合 PK upsert，
//! 冲突仅改 nick_name；本地缺成员行插占位，权威靠 path1 全量覆盖——见 to_effect_s1 裁决注释）。
//! 三键缺一 → noop（边界零信任，helix-im 不变量 4）。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const NICKNAME_ACTION: &str = "update_channel_member_nickName";

struct NicknameHandler;

impl WsMessageHandler for NicknameHandler {
    fn action(&self) -> &'static str {
        NICKNAME_ACTION
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
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };
        // userId 缺 → 无定点目标 → noop（不可识别 payload 静默忽略）。
        let Some(user_id) = data
            .get("userId")
            .or_else(|| data.get("user_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            return Ok(());
        };
        let nick_name = data
            .get("nickName")
            .or_else(|| data.get("nick_name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        out.push(crate::acl::to_effect_s1::update_member_nickname(
            channel_id, user_id, nick_name,
        ));
        out.push(crate::acl::to_effect_s1::emit_member_nickname(
            channel_id, user_id, nick_name,
        ));
        Ok(())
    }
}

static NICKNAME_HANDLER: NicknameHandler = NicknameHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&NICKNAME_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: NICKNAME_ACTION,
        handler: &NICKNAME_HANDLER,
    }
}
