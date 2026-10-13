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
const TOP_KEYS: &[&str] = &["channelIsTop", "channel_is_top", "top"];
const TOP_USER_KEYS: &[&str] = &["userId", "user_id", "memberUserId", "member_user_id"];
const NOTIFY_MODES: &[&str] = &["NORMAL", "STRONG", "IGNORE"];

/// 把 Go owner.id 兼容为 Angular authority 使用的 owner.userId，同时保留原 wire 字段。
fn normalize_owner_projection(channel: &mut serde_json::Value) {
    let Some(owner) = channel
        .get_mut("owner")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    if owner.get("userId").is_some() {
        return;
    }
    let Some(owner_id) = owner
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    owner.insert("userId".to_string(), serde_json::Value::String(owner_id));
}

/// 判断 update_channel 是否携带成员视角的置顶绝对态。
fn has_top_intent(channel: &serde_json::Value, data: &serde_json::Value) -> bool {
    TOP_KEYS
        .iter()
        .any(|key| channel.get(*key).is_some() || data.get(*key).is_some())
}

/// 读取置顶绝对态；非布尔值不允许进入 channel_member 写路径。
fn top_intent_value(channel: &serde_json::Value, data: &serde_json::Value) -> Option<bool> {
    TOP_KEYS.iter().find_map(|key| {
        channel
            .get(*key)
            .or_else(|| data.get(*key))
            .and_then(serde_json::Value::as_bool)
    })
}

/// 读取显式 actor；置顶路径禁止用 auth fallback 代替后端 userId。
fn top_intent_user_id<'a>(
    channel: &'a serde_json::Value,
    data: &'a serde_json::Value,
) -> Option<&'a str> {
    TOP_USER_KEYS.iter().find_map(|key| {
        channel
            .get(*key)
            .or_else(|| data.get(*key))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
    })
}

/// 置顶是单独成员状态；混入频道共享字段时整帧拒绝，避免误写 channel.is_top。
fn top_intent_has_mixed_fields(channel: &serde_json::Value, data: &serde_json::Value) -> bool {
    const ALLOWED_CHANNEL_KEYS: &[&str] = &[
        "id",
        "channelId",
        "channel_id",
        "userId",
        "user_id",
        "memberUserId",
        "member_user_id",
        "channelIsTop",
        "channel_is_top",
        "top",
    ];
    const ALLOWED_DATA_KEYS: &[&str] = &[
        "channel",
        "channelId",
        "channel_id",
        "userId",
        "user_id",
        "memberUserId",
        "member_user_id",
        "channelIsTop",
        "channel_is_top",
        "top",
    ];
    let channel_mixed = channel
        .as_object()
        .map(|object| {
            object
                .keys()
                .any(|key| !ALLOWED_CHANNEL_KEYS.contains(&key.as_str()))
        })
        .unwrap_or(true);
    let data_mixed = if data.get("channel").is_some() {
        data.as_object()
            .map(|object| {
                object
                    .keys()
                    .any(|key| !ALLOWED_DATA_KEYS.contains(&key.as_str()))
            })
            .unwrap_or(true)
    } else {
        false
    };
    channel_mixed || data_mixed
}

/// 判断 update_channel 是否携带当前 viewer 的通知状态补丁。
fn has_notify_intent(channel: &serde_json::Value, data: &serde_json::Value) -> bool {
    channel.get("notify").is_some() || data.get("notify").is_some()
}

/// 严格解析 Go 的通知枚举，并拒绝同一帧内相互冲突的内外层值。
fn notify_intent_value(
    channel: &serde_json::Value,
    data: &serde_json::Value,
) -> Result<Option<String>, ImError> {
    let mut value: Option<String> = None;
    for raw in [channel.get("notify"), data.get("notify")]
        .into_iter()
        .flatten()
    {
        let Some(candidate) = raw.as_str() else {
            return Err(ImError::Parse(
                "update_channel notify must be a string enum".to_string(),
            ));
        };
        if !NOTIFY_MODES.contains(&candidate) {
            return Err(ImError::Parse(format!(
                "update_channel notify has invalid mode: {candidate}"
            )));
        }
        if value.as_deref().is_some_and(|current| current != candidate) {
            return Err(ImError::Parse(
                "update_channel notify has conflicting values".to_string(),
            ));
        }
        value = Some(candidate.to_string());
    }
    Ok(value)
}

/// 通知是当前 viewer 的单字段状态；混入频道共享字段时整帧拒绝，避免误写其它列。
fn notify_intent_has_mixed_fields(channel: &serde_json::Value, data: &serde_json::Value) -> bool {
    const ALLOWED_CHANNEL_KEYS: &[&str] = &[
        "id",
        "channelId",
        "channel_id",
        "userId",
        "user_id",
        "memberUserId",
        "member_user_id",
        "notify",
    ];
    const ALLOWED_DATA_KEYS: &[&str] = &[
        "channel",
        "id",
        "channelId",
        "channel_id",
        "userId",
        "user_id",
        "memberUserId",
        "member_user_id",
        "notify",
    ];
    let channel_mixed = channel
        .as_object()
        .map(|object| {
            object
                .keys()
                .any(|key| !ALLOWED_CHANNEL_KEYS.contains(&key.as_str()))
        })
        .unwrap_or(true);
    let data_mixed = if data.get("channel").is_some() {
        data.as_object()
            .map(|object| {
                object
                    .keys()
                    .any(|key| !ALLOWED_DATA_KEYS.contains(&key.as_str()))
            })
            .unwrap_or(true)
    } else {
        false
    };
    channel_mixed || data_mixed
}

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
        // 合法空摘要不属于撤回对象；仍走普通频道/成员权威写入及 Persist 屏障。
        serde_json::Value::String(encoded) if encoded.is_empty() => return Ok(None),
        serde_json::Value::String(encoded) => serde_json::from_str(encoded)
            .map_err(|error| ImError::Parse(format!("update_channel lastPost: {error}")))?,
        serde_json::Value::Object(_) => raw.clone(),
        // 普通 PATCH 的 null 不写入数据库，保留原有稀疏更新语义。
        serde_json::Value::Null => return Ok(None),
        _ => {
            return Err(ImError::Parse(
                "update_channel lastPost must be an object".into(),
            ))
        }
    };
    if !post.is_object() {
        return Err(ImError::Parse(
            "update_channel lastPost must be an object".into(),
        ));
    }
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

    /// G-04 lastPost 与普通频道更新都在持久化成功后发布 MessageV3 绝对态。
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

        // Go broadcasts `{id,userId,notify}` after the member transaction.  Treat it as a
        // strict viewer-local patch and read the durable channel row back before emitting.
        let notify_intent = has_notify_intent(channel, data);
        if notify_intent {
            let Some(notify) = notify_intent_value(channel, data)? else {
                return Ok(());
            };
            let Some(user_id) = top_intent_user_id(channel, data) else {
                return Ok(());
            };
            if user_id != ctx.auth_user_id || notify_intent_has_mixed_fields(channel, data) {
                return Ok(());
            }
            let Some((row, _projection)) =
                crate::channel_update::member_channel_from_update_channel(
                    data,
                    channel_id,
                    ctx.auth_user_id,
                    ctx.now_ms,
                )
            else {
                return Ok(());
            };
            let mut ops = Vec::with_capacity(3);
            let cols = crate::channel_write::collect_present(channel);
            if let Some(op) = crate::acl::to_effect::update_channel_partial_op(channel_id, cols) {
                ops.push(op);
            }
            if let Some(op) = crate::acl::to_effect::upsert_channel_member_channel_op(row) {
                ops.push(op);
            }
            // `notify` is already validated above; this guards the impossible empty-op case
            // if a future wire alias is added without a channel projection mapping.
            if ops.is_empty() {
                return Ok(());
            }
            ops.push(crate::acl::to_effect::get_channel_row_op(channel_id));
            let corr = ctx.alloc_corr();
            ctx.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::NotifyChannelPersist { channel_id },
            );
            out.push(helix_core::Effect::Persist { corr, ops });
            let _ = notify;
            return Ok(());
        }

        // Go `change/top` 回推 `{id,userId,channelIsTop}`；完整共享频道快照也会携带
        // `channelIsTop`，只有无共享字段的严格 actor shape 才属于成员投影。
        let has_top = has_top_intent(channel, data);
        let top_has_mixed_fields = has_top && top_intent_has_mixed_fields(channel, data);
        let top_actor = has_top.then(|| top_intent_user_id(channel, data)).flatten();
        if has_top && top_has_mixed_fields && top_actor.is_some() {
            return Ok(());
        }
        let top_intent = has_top && !top_has_mixed_fields;
        if top_intent {
            let Some(top) = top_intent_value(channel, data) else {
                return Ok(());
            };
            let Some(user_id) = top_actor else {
                return Ok(());
            };
            if user_id != ctx.auth_user_id {
                return Ok(());
            }
            let _ = top;
        }
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
        let cols = if top_intent {
            Vec::new()
        } else {
            crate::channel_write::collect_present(channel)
        };
        if let Some(op) = crate::acl::to_effect::update_channel_partial_op(channel_id, cols) {
            ops.push(op);
        }
        let member_channel = if let Some((row, projection)) =
            crate::channel_update::member_channel_from_update_channel(
                data,
                channel_id,
                ctx.auth_user_id,
                ctx.now_ms,
            ) {
            if let Some(op) = crate::acl::to_effect::upsert_channel_member_channel_op(row) {
                ops.push(op);
            }
            Some(crate::acl::to_effect::member_channel_update_data(
                channel_id,
                &projection,
                UPDATE_CHANNEL_ACTION,
            ))
        } else {
            None
        };
        if top_intent && member_channel.is_none() {
            return Ok(());
        }
        if !ops.is_empty() {
            let corr = ctx.alloc_corr();
            out.push(helix_core::Effect::Persist { corr, ops });
            let causation_id = frame.cses_track_id().map(str::to_string).or_else(|| {
                frame
                    .event_seq()
                    .map(|seq| format!("update-channel-{}", seq.0))
            });
            let mut persisted_channel = channel.clone();
            // PATCH 的 null 未写入数据库，不能作为清空事实透传给消费者。
            if let Some(fields) = persisted_channel.as_object_mut() {
                fields.retain(|_, value| !value.is_null());
            }
            normalize_owner_projection(&mut persisted_channel);
            if let Some(object) = persisted_channel.as_object_mut() {
                for key in ["settingVersion", "setting_version", "capabilitiesByRole"] {
                    if object.get(key).is_none() {
                        if let Some(value) = data.get(key) {
                            object.insert(key.to_string(), value.clone());
                        }
                    }
                }
                if top_intent {
                    if let Some(top) = member_channel
                        .as_ref()
                        .and_then(|member| member.get("dialogPatch"))
                        .and_then(|patch| patch.get("channelIsTop"))
                    {
                        object.insert("channelIsTop".to_string(), top.clone());
                    }
                }
            }
            ctx.state.corr_map.insert(
                corr,
                crate::state::CorrelationContext::UpdateChannelDialogPersist {
                    channel_id,
                    channel: Box::new(persisted_channel),
                    member_channel: if top_intent {
                        None
                    } else {
                        member_channel.map(Box::new)
                    },
                    causation_id,
                },
            );
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
