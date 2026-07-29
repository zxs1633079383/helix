//! G-08/G-09 creator 私有 schedule 权威事实与规范化写模型。

use crate::state::ChannelId;
use helix_core::effect::{BatchUpdateSpec, Row, SqlValue, StorageOp, UpsertSpec};

/// 从兼容 wire 形状读取一个 schedule 字段。
fn schedule_field<'a>(data: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    let schedule = data.get("schedule").unwrap_or(data);
    keys.iter()
        .find_map(|key| schedule.get(*key))
        .or_else(|| keys.iter().find_map(|key| data.get(*key)))
}

/// 服务端 schedule authority 的 creator 本地持久化形状。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleFact {
    pub(crate) channel_id: ChannelId,
    pub(crate) schedule_id: String,
    pub(crate) owner_user_id: String,
    message: String,
    message_preview: String,
    send_at: i64,
    status: String,
    pub(crate) revision: u64,
    has_schedule_post: bool,
}

impl ScheduleFact {
    /// 仅从合法 G-08 WS authority 构造 creator durable fact。
    pub(crate) fn from_created_ws(
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
            message: schedule_field(data, &["message"])
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message_preview: schedule_field(data, &["messagePreview", "message_preview"])
                .and_then(serde_json::Value::as_str)
                .map(|value| value.chars().take(96).collect())
                .unwrap_or_default(),
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

    /// 仅从合法 G-09 WS authority 构造清空正文的 creator terminal fact。
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
            message: String::new(),
            message_preview: String::new(),
            send_at: 0,
            status: "canceled".to_string(),
            revision,
            has_schedule_post: false,
        })
    }

    /// 将 authority fact 与 channel read model 放进同一个原子持久屏障。
    pub(crate) fn storage_ops(self) -> Vec<StorageOp> {
        let (schedule_id_for_schedule, schedule_id_for_channel) = duplicate_text(self.schedule_id);
        let (owner_for_schedule, owner_for_channel) = duplicate_text(self.owner_user_id);
        let (message_for_schedule, message_for_channel) = duplicate_text(self.message);
        let (preview_for_schedule, preview_for_channel) = duplicate_text(self.message_preview);
        let (status_for_schedule, status_for_channel) = duplicate_text(self.status);
        let schedule_row: Row = vec![
            (
                "channel_id".to_string(),
                SqlValue::Text(self.channel_id.as_str().to_string()),
            ),
            (
                "schedule_id".to_string(),
                SqlValue::Text(schedule_id_for_schedule),
            ),
            (
                "owner_user_id".to_string(),
                SqlValue::Text(owner_for_schedule),
            ),
            ("message".to_string(), SqlValue::Text(message_for_schedule)),
            (
                "message_preview".to_string(),
                SqlValue::Text(preview_for_schedule),
            ),
            ("send_at".to_string(), SqlValue::Integer(self.send_at)),
            ("status".to_string(), SqlValue::Text(status_for_schedule)),
            (
                "revision".to_string(),
                SqlValue::Integer(self.revision.min(i64::MAX as u64) as i64),
            ),
        ];
        let channel_patch: Row = vec![
            (
                "has_schedule_post".to_string(),
                SqlValue::Integer(i64::from(self.has_schedule_post)),
            ),
            (
                "schedule_id".to_string(),
                SqlValue::Text(schedule_id_for_channel),
            ),
            (
                "schedule_owner_user_id".to_string(),
                SqlValue::Text(owner_for_channel),
            ),
            (
                "schedule_message".to_string(),
                SqlValue::Text(message_for_channel),
            ),
            (
                "schedule_message_preview".to_string(),
                SqlValue::Text(preview_for_channel),
            ),
            (
                "schedule_send_at".to_string(),
                SqlValue::Integer(self.send_at),
            ),
            (
                "schedule_status".to_string(),
                SqlValue::Text(status_for_channel),
            ),
            (
                "schedule_revision".to_string(),
                SqlValue::Integer(self.revision.min(i64::MAX as u64) as i64),
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

/// 为两个规范化读模型创建恰好一份必要副本。
fn duplicate_text(value: String) -> (String, String) {
    (value.clone(), value)
}
