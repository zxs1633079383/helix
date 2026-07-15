//! `update_channel` action handler（S3 path2：channel PATCH）。
//!
//! 行为真源：现网 `channel_service.rs:100 apply_update` → `collect_present_fields`（只收 WS
//! `Some` 字段）→ `update_partial`（参数化 UPDATE + 白名单）。helix：`channel_write::collect_present`
//! 产白名单列集 → `to_effect::update_channel_partial`（UPDATE … WHERE id=?）。
//!
//! `None`/缺省字段不进 SET（DB 既有值保留）；空列集 → 无写意图（None，no-op）。
//! 成员 / owner / admin_users / boss 不走此路径（独立成员表，对齐现网 update_partial filter）。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const UPDATE_CHANNEL_ACTION: &str = "update_channel";

struct UpdateChannelHandler;

impl WsMessageHandler for UpdateChannelHandler {
    fn action(&self) -> &'static str {
        UPDATE_CHANNEL_ACTION
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
        let channel = data.get("channel").unwrap_or(data);
        // channel id 硬 gate（缺 / 非 26 字符 → no-op，边界零信任，helix-im 不变量 4）。
        let Some(channel_id) = channel
            .get("id")
            .or_else(|| data.get("channelId"))
            .or_else(|| data.get("channel_id"))
            .or_else(|| channel.get("channelId"))
            .or_else(|| channel.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };
        crate::channel_capabilities::remember_viewer_role(
            ctx.state,
            channel_id,
            channel,
            ctx.auth_user_id,
        );

        // 收 Some 字段 → PATCH（空集 = 无可写字段，update_channel_partial 返回 None → 不 push）。
        let cols = crate::channel_write::collect_present(channel);
        if let Some(eff) = crate::acl::to_effect::update_channel_partial(channel_id, cols) {
            out.push(eff);
        }
        if let Some((row, projection)) = crate::dialog_patch::member_dialog_from_update_channel(
            data,
            channel_id,
            ctx.auth_user_id,
        ) {
            if let Some(eff) = crate::acl::to_effect::upsert_channel_member_dialog(row) {
                out.push(eff);
            }
            out.push(crate::acl::to_effect::emit_channel_update_by_member_dialog(
                channel_id,
                &projection,
                UPDATE_CHANNEL_ACTION,
            ));
        }
        // 独立 update_channel 推送（per-member 改群属性如对话置顶 channel/change/top·改群名/公告
        // 等定向本人 PATCH）落库后须 emit `im:channel:update`（thin·{channel_id}）通知前端重查
        // dialogList。真实 Go 有时只给最小 channel 对象或把字段包在 data.channel 下；即使空 PATCH，
        // thin 信号仍是移动端所需的 render trigger，且不伪造 channel 行。
        out.push(crate::acl::to_effect::emit_channel_update(channel_id));
        let setting_version = channel
            .get("settingVersion")
            .or_else(|| data.get("settingVersion"))
            .and_then(serde_json::Value::as_u64)
            .or_else(|| frame.event_seq().map(|seq| seq.0))
            .unwrap_or(0);
        let role = ctx
            .state
            .viewer_channel_roles
            .get(&channel_id)
            .map(String::as_str);
        let matrix = channel
            .get("capabilitiesByRole")
            .or_else(|| data.get("capabilitiesByRole"));
        let capabilities = crate::channel_capabilities::for_viewer(matrix, role);
        if let Some(effect) = crate::acl::to_effect_s1::emit_channel_settings_updated(
            channel_id,
            channel,
            setting_version,
            capabilities,
        ) {
            out.push(effect);
        }
        Ok(())
    }
}

static UPDATE_CHANNEL_HANDLER: UpdateChannelHandler = UpdateChannelHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&UPDATE_CHANNEL_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: UPDATE_CHANNEL_ACTION,
        handler: &UPDATE_CHANNEL_HANDLER,
    }
}
