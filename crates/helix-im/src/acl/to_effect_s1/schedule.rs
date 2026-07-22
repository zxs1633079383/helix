//! Schedule projection factories.

use super::emit;
use crate::state::ChannelId;
use helix_core::Effect;

fn schedule_field<'a>(data: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    let schedule = data.get("schedule").unwrap_or(data);
    keys.iter()
        .find_map(|key| schedule.get(*key))
        .or_else(|| keys.iter().find_map(|key| data.get(*key)))
}

/// schedule 全文和摘要只在服务端明确授权，或 owner 与当前登录用户一致时开放。
pub fn schedule_preview_authorized(data: &serde_json::Value, auth_user_id: &str) -> bool {
    let explicitly_allowed = schedule_field(
        data,
        &[
            "canViewContent",
            "can_view_content",
            "viewerCanViewContent",
            "viewer_can_view_content",
        ],
    )
    .and_then(serde_json::Value::as_bool)
    .unwrap_or(false);
    let owner_matches = !auth_user_id.is_empty()
        && schedule_field(
            data,
            &[
                "scheduleOwnerUserId",
                "schedule_owner_user_id",
                "ownerUserId",
                "owner_user_id",
                "userId",
                "user_id",
            ],
        )
        .and_then(serde_json::Value::as_str)
            == Some(auth_user_id);
    explicitly_allowed || owner_matches
}

fn schedule_projection(
    channel_id: ChannelId,
    data: &serde_json::Value,
    revision_fallback: Option<u64>,
    can_view_content: bool,
    status_fallback: &'static str,
    has_schedule_post: bool,
) -> serde_json::Value {
    let schedule_id = schedule_field(data, &["scheduleId", "schedule_id"])
        .and_then(serde_json::Value::as_str)
        .map_or(serde_json::Value::Null, |value| serde_json::json!(value));
    let send_at = schedule_field(
        data,
        &[
            "sendAt",
            "send_at",
            "schedulePostAt",
            "schedule_post_at",
            "scheduledAt",
            "scheduled_at",
        ],
    )
    .and_then(serde_json::Value::as_i64)
    .map_or(serde_json::Value::Null, |value| serde_json::json!(value));
    let status = if has_schedule_post {
        schedule_field(data, &["status"])
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(status_fallback)
    } else {
        // canceled 事件是终态，不允许迟到或错误的 wire status 把 UI 恢复成 scheduled。
        status_fallback
    };
    let revision = schedule_field(data, &["revision"])
        .and_then(serde_json::Value::as_u64)
        .or(revision_fallback)
        .unwrap_or(0);
    // A cancellation is terminal. Even the owner must not receive a retired draft body in the
    // canceled projection, otherwise a late consumer could resurrect already removed content.
    let can_view_active_content = can_view_content && has_schedule_post;
    let message = if can_view_active_content {
        schedule_field(data, &["message"])
            .and_then(serde_json::Value::as_str)
            .map_or(serde_json::Value::Null, |value| {
                serde_json::Value::String(value.to_string())
            })
    } else {
        serde_json::Value::Null
    };
    let message_preview = if can_view_active_content {
        schedule_field(data, &["messagePreview", "message_preview"])
            .and_then(serde_json::Value::as_str)
            .map(|value| value.chars().take(96).collect::<String>())
            .map_or(serde_json::Value::Null, serde_json::Value::String)
    } else {
        serde_json::Value::Null
    };
    serde_json::json!({
        "id": channel_id.as_str(),
        "channelId": channel_id.as_str(),
        "hasSchedulePost": has_schedule_post,
        "scheduleId": schedule_id,
        "sendAt": send_at,
        "scheduleSendAt": send_at,
        "status": status,
        "scheduleStatus": status,
        "revision": revision,
        "scheduleRevision": revision,
        "scheduleMessage": message,
        "messagePreview": message_preview,
        "scheduleMessagePreview": message_preview,
    })
}

/// Normalize `getSchedule` only when the response belongs to the attached channel and viewer.
pub fn owner_schedule_hydration(
    data: &serde_json::Value,
    expected_channel_id: &str,
    viewer_user_id: &str,
) -> Option<serde_json::Value> {
    if expected_channel_id.is_empty() || viewer_user_id.is_empty() {
        return None;
    }
    let channel_id =
        schedule_field(data, &["channelId", "channel_id"]).and_then(serde_json::Value::as_str)?;
    if channel_id != expected_channel_id || !schedule_preview_authorized(data, viewer_user_id) {
        return None;
    }
    Some(schedule_projection(
        ChannelId::from_str(channel_id)?,
        data,
        None,
        true,
        "scheduled",
        true,
    ))
}

/// `post_schedule_created` → render-ready `im:channel:schedule-created`。
pub fn emit_schedule_created(
    channel_id: ChannelId,
    data: &serde_json::Value,
    revision_fallback: Option<u64>,
    can_view_content: bool,
) -> Effect {
    emit(
        "im:channel:schedule-created",
        schedule_projection(
            channel_id,
            data,
            revision_fallback,
            can_view_content,
            "scheduled",
            true,
        ),
    )
}

/// `post_schedule_canceled` → render-ready `im:channel:schedule-canceled`。
pub fn emit_schedule_canceled(
    channel_id: ChannelId,
    data: &serde_json::Value,
    revision_fallback: Option<u64>,
    can_view_content: bool,
) -> Effect {
    emit(
        "im:channel:schedule-canceled",
        schedule_projection(
            channel_id,
            data,
            revision_fallback,
            can_view_content,
            "canceled",
            false,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_hydration_is_channel_and_viewer_scoped() {
        let channel_id = "chfixx00000000000000000001";
        let response = serde_json::json!({
            "channelId": channel_id,
            "scheduleId": "schedule-1",
            "scheduleOwnerUserId": "owner-1",
            "message": "owner secret",
            "sendAt": 1_780_000_000_000_i64,
            "revision": 7
        });
        let hydrated = owner_schedule_hydration(&response, channel_id, "owner-1").unwrap();
        assert_eq!(hydrated["scheduleId"], "schedule-1");
        assert_eq!(hydrated["scheduleMessage"], "owner secret");
        assert!(owner_schedule_hydration(&response, "channel-2", "owner-1").is_none());
        assert!(owner_schedule_hydration(&response, channel_id, "observer-1").is_none());
    }

    #[test]
    fn canceled_projection_never_republishes_retired_draft_content() {
        let effect = emit_schedule_canceled(
            ChannelId::from_str("chfixx00000000000000000001").unwrap(),
            &serde_json::json!({
                "channelId": "chfixx00000000000000000001",
                "scheduleOwnerUserId": "owner-1",
                "message": "retired secret",
                "messagePreview": "retired",
                "revision": 8
            }),
            None,
            true,
        );
        let Effect::Emit { event } = effect else {
            panic!("expected emit")
        };
        let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
        assert_eq!(payload["data"]["scheduleStatus"], "canceled");
        assert!(payload["data"]["scheduleMessage"].is_null());
        assert!(payload["data"]["scheduleMessagePreview"].is_null());
    }
}
