//! `channel_member_role_updated` action handler（更新群成员角色，WS 全量 19 事件补齐，P4）。
//!
//! 后端 `cses_channel.go:1448` 推 `WebsocketEventChannelMemberRoleUpdated`，wire `data` =
//! `{channelId, userIds:[], role}`（见 full-map partials/5 §2.12）。
//!
//! 移动端契约要求该帧转为 `im:channel:member-updated`。Helix 先读回当前
//! `channel_member`，只更新已存在成员的 role，再以 PersistAtomic + read-back 释放完整 roster；
//! 不把 `{userIds, role}` delta 直接送给 Angular。

use helix_core::effect::{ScanSpec, SqlValue, StorageOp};
use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const ROLE_UPDATED_ACTION: &str = "channel_member_role_updated";

struct RoleUpdatedHandler;

/// 将 Go role wire 值收敛为本地成员表允许的绝对角色；未知值 fail-closed。
fn normalize_role(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_uppercase().as_str() {
        "OWNER" | "CREATOR" => Some("OWNER"),
        "BOSS" => Some("BOSS"),
        "ADMIN" | "MANAGER" | "MANGER" => Some("ADMIN"),
        "MEMBER" | "REMOVE" | "REMOVED" | "" => Some("MEMBER"),
        _ => None,
    }
}

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

        let Some(role) = data
            .get("role")
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_role)
        else {
            tracing::warn!(channel_id = %channel_id.as_str(), "channel member role update rejected: unknown role");
            return Ok(());
        };
        let Some(user_ids) = data
            .get("userIds")
            .or_else(|| data.get("user_ids"))
            .and_then(serde_json::Value::as_array)
        else {
            return Ok(());
        };
        let user_ids = user_ids
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if user_ids.is_empty() || ctx.state.inflight_member_role_updates.contains(&channel_id) {
            return Ok(());
        }
        let corr = ctx.alloc_corr();
        ctx.state.inflight_member_role_updates.insert(channel_id);
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelMemberRoleScan {
                channel_id,
                user_ids,
                role: role.to_string(),
            },
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "channel_member",
                limit: None,
                filter: Some((
                    "channel_id",
                    SqlValue::Text(channel_id.as_str().to_string()),
                )),
                order_by: &[],
            })],
        });
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
