//! `channel_created` action handler（S3 path1：新建 channel 全量 upsert，spec §S1/§S3.1）。
//!
//! 行为真源：现网 `handlers/channel.rs:482 handle_create` → `channel_service::process_and_save`
//! （path1 全 52 列 upsert）→ emit `im:channel:created`。建话题 makeTopic 也发 channel_created。
//!
//! helix：`channel_write::program_full`（全 52 列 + JSON rename + 守卫）→ `upsert_channel_full`。
//! 成员（memberChange.join）同步写 `channel_member` 表（复合 PK，与 channel_member_update 同口径）。
//! 非法 data（缺 id / 非 26 字符）→ program_full None → 跳落库（仍 best-effort emit 由 channel_id
//! gate 决定；缺 id 则整体 noop，边界零信任 helix-im 不变量 4）。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_CREATED_ACTION: &str = "channel_created";

struct ChannelCreatedHandler;

impl WsMessageHandler for ChannelCreatedHandler {
    fn action(&self) -> &'static str {
        CHANNEL_CREATED_ACTION
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
        // id 硬 gate（缺/非 26 字符 → 整体 noop）。
        let Some(channel_id) = data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        super::channel_member_update::apply_channel_full_and_members(ctx, channel_id, data, out);

        // emit im:channel:created（透传帧 data 含 owner/members/picture，前端直接消费）。
        let raw = serde_json::to_vec(data).unwrap_or_default();
        out.push(crate::acl::to_effect_s1::emit_channel_created(
            channel_id, &raw,
        ));
        Ok(())
    }
}

static CHANNEL_CREATED_HANDLER: ChannelCreatedHandler = ChannelCreatedHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_CREATED_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_CREATED_ACTION,
        handler: &CHANNEL_CREATED_HANDLER,
    }
}
