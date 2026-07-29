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

/// 解析 Go 的 JSON 字符串/对象 lastPost，仅识别 revoke=true 的 G-04 分支。
fn revoke_last_post(data: &serde_json::Value) -> Result<Option<serde_json::Value>, ImError> {
    let channel = data.get("channel").unwrap_or(data);
    let Some(raw) = data
        .get("lastPost")
        .or_else(|| data.get("last_post"))
        .or_else(|| channel.get("lastPost"))
        .or_else(|| channel.get("last_post"))
    else {
        return Ok(None);
    };
    let post = match raw {
        serde_json::Value::String(encoded) => serde_json::from_str(encoded)
            .map_err(|error| ImError::Parse(format!("update_channel lastPost: {error}")))?,
        serde_json::Value::Object(_) => raw.clone(),
        _ => return Ok(None),
    };
    if !post
        .get("revoke")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }
    crate::event::post::revoke_last_post_from_authority(&post).map(Some)
}

impl WsMessageHandler for UpdateChannelHandler {
    fn action(&self) -> &'static str {
        UPDATE_CHANNEL_ACTION
    }

    /// G-04 lastPost 走写后读回 MessageV3；其余 channel patch 暂留旧投影路径。
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
        if let Some(last_post) = revoke_last_post(data)? {
            let corr = ctx.alloc_corr();
            let ops = crate::channel_write::message_v3_revoke_channel_ops(
                channel_id,
                ctx.auth_user_id,
                &last_post,
            );
            ctx.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::MessageV3RevokeChannelPersist { channel_id },
            );
            out.push(helix_core::Effect::PersistAtomic { corr, ops });
            return Ok(());
        }

        // 收 Some 字段 → PATCH（空集 = 无可写字段，update_channel_partial 返回 None → 不 push）。
        let mut ops = Vec::with_capacity(2);
        let cols = crate::channel_write::collect_present(channel);
        if let Some(op) = crate::acl::to_effect::update_channel_partial_op(channel_id, cols) {
            ops.push(op);
        }
        let member_dialog = if let Some((row, projection)) =
            crate::dialog_patch::member_dialog_from_update_channel(
                data,
                channel_id,
                ctx.auth_user_id,
            ) {
            if let Some(op) = crate::acl::to_effect::upsert_channel_member_dialog_op(row) {
                ops.push(op);
            }
            Some(crate::acl::to_effect::member_dialog_patch_data(
                channel_id,
                &projection,
                UPDATE_CHANNEL_ACTION,
            ))
        } else {
            None
        };
        let scope = ctx.state.dialog_list_frame.scope().cloned();
        let summary_attached = ctx
            .state
            .channel_summary_projector
            .current_view(channel_id.as_str())
            .is_some();
        if !ops.is_empty() && (scope.is_some() || summary_attached) {
            let corr = ctx.alloc_corr();
            out.push(helix_core::Effect::Persist { corr, ops });
            let causation_id = frame.cses_track_id().map(str::to_string).or_else(|| {
                frame
                    .event_seq()
                    .map(|seq| format!("update-channel-{}", seq.0))
            });
            let mut persisted_channel = channel.clone();
            if let Some(object) = persisted_channel.as_object_mut() {
                for key in ["settingVersion", "setting_version", "capabilitiesByRole"] {
                    if object.get(key).is_none() {
                        if let Some(value) = data.get(key) {
                            object.insert(key.to_string(), value.clone());
                        }
                    }
                }
            }
            ctx.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::UpdateChannelDialogPersist {
                    scope,
                    channel_id,
                    channel: Box::new(persisted_channel),
                    member_dialog: member_dialog.map(Box::new),
                    causation_id,
                },
            );
        } else if !ops.is_empty() {
            out.push(helix_core::Effect::PersistFire { ops });
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
