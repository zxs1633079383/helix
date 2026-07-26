//! Schedule projection factories.

use super::emit;
use crate::state::ChannelId;
use helix_core::effect::{BatchUpdateSpec, Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::Effect;

/// 从兼容 wire 形状读取一个定时消息字段，不把 key 别名泄漏给调用方。
fn schedule_field<'a>(data: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    let schedule = data.get("schedule").unwrap_or(data);
    keys.iter()
        .find_map(|key| schedule.get(*key))
        .or_else(|| keys.iter().find_map(|key| data.get(*key)))
}

/// G08 权威 WS 事实的持久化形状；正文已按当前 viewer 权限裁剪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleFact {
    pub(crate) channel_id: ChannelId,
    pub(crate) schedule_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) message: String,
    pub(crate) message_preview: String,
    pub(crate) send_at: i64,
    pub(crate) status: String,
    pub(crate) revision: u64,
    pub(crate) has_schedule_post: bool,
}

impl ScheduleFact {
    /// 仅从合法 channel/revision 的 WS authority 构造 viewer-scoped durable fact。
    pub(crate) fn from_created_ws(
        data: &serde_json::Value,
        auth_user_id: &str,
        revision_fallback: Option<u64>,
    ) -> Option<Self> {
        let channel_id = schedule_field(data, &["channelId", "channel_id"])
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)?;
        let revision = schedule_field(data, &["revision"])
            .and_then(serde_json::Value::as_u64)
            .or(revision_fallback)
            .filter(|revision| *revision > 0)?;
        let can_view_content = schedule_preview_authorized(data, auth_user_id);
        let message = can_view_content
            .then(|| schedule_field(data, &["message"]).and_then(serde_json::Value::as_str))
            .flatten()
            .unwrap_or_default()
            .to_string();
        let message_preview = can_view_content
            .then(|| {
                schedule_field(data, &["messagePreview", "message_preview"])
                    .and_then(serde_json::Value::as_str)
            })
            .flatten()
            .map(|value| value.chars().take(96).collect())
            .unwrap_or_default();
        Some(Self {
            channel_id,
            schedule_id: schedule_field(data, &["scheduleId", "schedule_id"])
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            owner_user_id: schedule_field(
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
            .unwrap_or_default()
            .to_string(),
            message,
            message_preview,
            send_at: schedule_field(
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
            .unwrap_or(0),
            status: schedule_field(data, &["status"])
                .and_then(serde_json::Value::as_str)
                .filter(|status| !status.is_empty())
                .unwrap_or("scheduled")
                .to_string(),
            revision,
            has_schedule_post: true,
        })
    }

    /// 仅从合法 G09 cancel authority 构造清空正文的 durable terminal fact。
    pub(crate) fn from_canceled_ws(
        data: &serde_json::Value,
        revision_fallback: Option<u64>,
    ) -> Option<Self> {
        let channel_id = schedule_field(data, &["channelId", "channel_id"])
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)?;
        let revision = schedule_field(data, &["revision"])
            .and_then(serde_json::Value::as_u64)
            .or(revision_fallback)
            .filter(|revision| *revision > 0)?;
        Some(Self {
            channel_id,
            schedule_id: String::new(),
            owner_user_id: String::new(),
            message: String::new(),
            message_preview: String::new(),
            send_at: 0,
            status: "canceled".to_string(),
            revision,
            has_schedule_post: false,
        })
    }

    /// 将 authority fact 与 channel read model 放进同一个原子持久屏障。
    pub(crate) fn storage_ops(&self) -> Vec<StorageOp> {
        let schedule_row: Row = vec![
            (
                "channel_id".to_string(),
                SqlValue::Text(self.channel_id.as_str().to_string()),
            ),
            (
                "schedule_id".to_string(),
                SqlValue::Text(self.schedule_id.clone()),
            ),
            (
                "owner_user_id".to_string(),
                SqlValue::Text(self.owner_user_id.clone()),
            ),
            ("message".to_string(), SqlValue::Text(self.message.clone())),
            (
                "message_preview".to_string(),
                SqlValue::Text(self.message_preview.clone()),
            ),
            ("send_at".to_string(), SqlValue::Integer(self.send_at)),
            ("status".to_string(), SqlValue::Text(self.status.clone())),
            (
                "revision".to_string(),
                SqlValue::Integer(self.revision as i64),
            ),
        ];
        let channel_patch: Row = vec![
            (
                "has_schedule_post".to_string(),
                SqlValue::Integer(i64::from(self.has_schedule_post)),
            ),
            (
                "schedule_id".to_string(),
                SqlValue::Text(self.schedule_id.clone()),
            ),
            (
                "schedule_owner_user_id".to_string(),
                SqlValue::Text(self.owner_user_id.clone()),
            ),
            (
                "schedule_message".to_string(),
                SqlValue::Text(self.message.clone()),
            ),
            (
                "schedule_message_preview".to_string(),
                SqlValue::Text(self.message_preview.clone()),
            ),
            (
                "schedule_send_at".to_string(),
                SqlValue::Integer(self.send_at),
            ),
            (
                "schedule_status".to_string(),
                SqlValue::Text(self.status.clone()),
            ),
            (
                "schedule_revision".to_string(),
                SqlValue::Integer(self.revision as i64),
            ),
        ];
        vec![
            StorageOp::BatchUpsert(UpsertSpec {
                table: "channel_schedule",
                rows: vec![schedule_row],
                conflict_key: Some("channel_id"),
                exclude_from_update: Vec::new(),
            }),
            StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "channel",
                key_col: "id",
                key_vals: vec![SqlValue::Text(self.channel_id.as_str().to_string())],
                patch: channel_patch,
            }),
        ]
    }
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
