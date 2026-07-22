//! S1 新增 WS action（编辑 / 已读 / channel 维护 / 成员 / 排程）的 Effect 工厂。
//!
//! 与 `to_effect.rs` 分文件（§1 单文件 ≤300 行硬顶；`to_effect.rs` 现在是 re-export facade）。
//! 职责同 ACL-1：把 IM 概念翻译成 core 通用 Effect/StorageOp——表名 / 列名 / `im:` 前缀作**数据**
//! 经 spec 传给 core，core 不解释（守 HX-C001）。
//!
//! 行为真源：现网 cses-client `handlers/{post,channel,schedule}.rs` + `event_bus.rs:44 emit_im`。

use crate::state::ChannelId;
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

/// 把 `{event, data}` JSON 包成 Emit Effect（信封铁律：与既有 emit_* 同形态）。
///
/// `data` 由调用方按现网 emit payload 形态拼好（如 `{channelId, deleteAt}`）。静态 shape，
/// 序列化失败=编程错（`expect`，非边界输入；边界输入零信任已在 handler 解析阶段完成）。
pub(super) fn emit(event: &'static str, data: serde_json::Value) -> Effect {
    use serde_json::json;
    let payload = json!({ "event": event, "data": data });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_s1: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

fn first_i64(data: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| match data.get(*key) {
        Some(serde_json::Value::Number(value)) => value.as_i64(),
        Some(serde_json::Value::String(value)) => value.parse::<i64>().ok(),
        _ => None,
    })
}

fn first_bool(data: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| match data.get(*key) {
        Some(serde_json::Value::Bool(value)) => Some(*value),
        Some(serde_json::Value::Number(value)) => value.as_i64().map(|value| value != 0),
        Some(serde_json::Value::String(value)) if value == "1" => Some(true),
        Some(serde_json::Value::String(value)) if value == "0" => Some(false),
        Some(serde_json::Value::String(value)) if value.eq_ignore_ascii_case("true") => Some(true),
        Some(serde_json::Value::String(value)) if value.eq_ignore_ascii_case("false") => {
            Some(false)
        }
        _ => None,
    })
}

// ─── channel 维护事件（created / closed / member-updated / memberNickname）──────

/// `channel_created` → `im:channel:created`（真源 channel.rs:569）。
///
/// 透传帧 `data` 原始字节（含 owner/members/picture 等，前端直接消费）；非法 JSON 降级 null。
pub fn emit_channel_created(channel_id: ChannelId, raw: &[u8]) -> Effect {
    let channel: serde_json::Value = serde_json::from_slice(raw).unwrap_or(serde_json::Value::Null);
    let display_name = channel
        .get("displayName")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(channel_id.as_str());
    let channel_type = crate::channel_kind::channel_type(&channel);
    let channel_kind = crate::channel_kind::channel_kind(channel_type);
    let member_count = channel
        .get("memberCount")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let created_at = first_i64(&channel, &["createdAt", "createAt", "created_at"]).unwrap_or(0);
    let last_activity_at =
        first_i64(&channel, &["lastPostAt", "last_post_at"]).unwrap_or(created_at);
    let top = first_bool(&channel, &["channelIsTop", "isTop", "is_top"]).unwrap_or(false);
    emit(
        "im:channel:created",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "channel_id": channel_id.as_str(),
            "displayName": display_name,
            "group": channel_kind,
            "channelType": channel_type,
            "channelKind": channel_kind,
            "memberCount": member_count,
            "createdAt": created_at,
            "lastActivityAt": last_activity_at,
            "top": top,
            "projectionSource": "channel_created",
            "props": channel,
            "channel": channel,
        }),
    )
}

/// `channel_close` → `im:channel:closed`（真源 channel.rs:593 `{channelId, deleteAt}`）。
pub fn emit_channel_closed(channel_id: ChannelId, delete_at: i64) -> Effect {
    emit(
        "im:channel:closed",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "closed": true,
            "deleteAt": delete_at,
        }),
    )
}

/// 自己退群成功的 render-ready 终态；与 channel closed 严格区分。
pub fn emit_channel_left(channel_id: ChannelId) -> Effect {
    emit(
        "im:channel:left",
        serde_json::json!({
            "id": channel_id.as_str(),
            "channelId": channel_id.as_str(),
            "left": true,
        }),
    )
}

/// `channel_member_update` → `im:channel:member-updated`（真源 channel.rs:626）。
///
/// 透传帧 `data`（含 memberChange）原始字节；非法 JSON 降级 null。
pub fn emit_channel_member_updated(channel_id: ChannelId, raw: &[u8]) -> Effect {
    let channel: serde_json::Value = serde_json::from_slice(raw).unwrap_or(serde_json::Value::Null);
    emit(
        "im:channel:member-updated",
        serde_json::json!({ "channel_id": channel_id.as_str(), "channel": channel }),
    )
}

/// 群属性保存回声的 render-ready 终态；仅复制服务端实际携带的设置字段。
pub fn emit_channel_settings_updated(
    channel_id: ChannelId,
    channel: &serde_json::Value,
    setting_version: u64,
    channel_capabilities: serde_json::Value,
) -> Option<Effect> {
    let mut settings = serde_json::Map::new();
    for key in [
        "displayName",
        "header",
        "purpose",
        "orient",
        "module",
        "source",
        "pictureType",
        "picture",
        "notice",
    ] {
        if let Some(value) = channel.get(key) {
            settings.insert(key.to_string(), value.clone());
        }
    }
    if settings.is_empty() {
        return None;
    }
    Some(emit(
        "im:channel:settings-updated",
        serde_json::json!({
            "channelId": channel_id.as_str(),
            "settings": settings,
            "settingVersion": setting_version,
            "channelCapabilities": channel_capabilities,
        }),
    ))
}

/// `update_channel_member_nickName` → `im:channel:memberNickname`（真源 channel.rs:757）。
pub fn emit_member_nickname(channel_id: ChannelId, user_id: &str, nick_name: &str) -> Effect {
    emit(
        "im:channel:memberNickname",
        serde_json::json!({
            "channelId": channel_id.as_str(),
            "userId": user_id,
            "nickName": nick_name,
        }),
    )
}

mod schedule;
mod storage;
pub use schedule::{
    emit_schedule_canceled, emit_schedule_created, owner_schedule_hydration,
    schedule_preview_authorized,
};
pub use storage::{
    channel_set_cols, delete_channel_members, update_member_nickname, upsert_channel_members,
    MemberRow,
};
