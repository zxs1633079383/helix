//! Topic child-post authority -> durable parent root-card activity projection.

use helix_core::effect::{BatchUpdateSpec, Effect, GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::{json, Value};

use crate::channel_update::PendingChannelUpdate;
use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};

/// 已持久化 topic child post 投向 parent root card 所需的最小 authority 快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicActivityProjection {
    pub(crate) topic_channel_id: ChannelId,
    pub(crate) root_message_id: String,
    pub(crate) title: String,
    pub(crate) post_id: String,
    pub(crate) author_user_id: String,
    pub(crate) message_type: String,
    pub(crate) preview_text: String,
    pub(crate) created_at: i64,
    pub(crate) topic_message_count: u64,
}

impl ImModule {
    /// 已提交的 topic 频道更新才能启动 root 反向读，duplicate authority 不重复计数。
    pub(crate) fn schedule_topic_activity_projection(
        &mut self,
        pending: &PendingChannelUpdate,
        channel_reply: &[u8],
        out: &mut EffectSink,
    ) {
        if !pending.update.visible {
            return;
        }
        let channel = serde_json::from_slice::<Value>(channel_reply)
            .ok()
            .and_then(|value| value.as_array()?.first().cloned());
        let relation = channel.as_ref().and_then(|row| {
            let root_message_id = text(
                row,
                &[
                    "root_post_id",
                    "rootPostId",
                    "root_message_id",
                    "rootMessageId",
                ],
            )?;
            let title = text(row, &["display_name", "displayName", "name"])?;
            // 频道读回保留此前话题总数，本次已提交 post 再贡献一条。
            let topic_message_count = integer(
                row,
                &[
                    "total_msg_count",
                    "totalMsgCount",
                    "topic_msg_count",
                    "topicMsgCount",
                ],
            )
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or_default()
            .saturating_add(1);
            Some((
                root_message_id.to_string(),
                title.to_string(),
                topic_message_count,
            ))
        });
        let Some((root_message_id, title, topic_message_count)) = relation else {
            return;
        };
        let projection = TopicActivityProjection {
            topic_channel_id: pending.channel_id,
            root_message_id: root_message_id.clone(),
            title,
            post_id: pending.msg_id.clone(),
            author_user_id: pending.fields.user_id.clone(),
            message_type: pending.fields.msg_type.clone(),
            preview_text: if pending.fields.simple_message.is_empty() {
                pending.fields.message.clone()
            } else {
                pending.fields.simple_message.clone()
            },
            created_at: pending.fields.create_at,
            topic_message_count,
        };
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "message",
                key_col: "id",
                key_val: SqlValue::Text(root_message_id),
            })],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TopicActivityRootRead { projection },
        );
    }

    /// Root durable row 提供 parent channel 与旧计数；本步单调合并新 activity 后再写回。
    pub(crate) fn handle_topic_activity_root_read(
        &mut self,
        projection: TopicActivityProjection,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(root) = storage_rows(outcome).into_iter().next() else {
            return Ok(());
        };
        let parent_channel_id = text(&root, &["channel_id", "channelId"])
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("topic root omitted parent channel".to_string()))?;
        let existing = json_value(&root, &["topic"]).unwrap_or_else(|| json!({}));
        if existing.get("latestReplyId").and_then(Value::as_str)
            == Some(projection.post_id.as_str())
            || existing
                .get("lastActivityAt")
                .and_then(Value::as_i64)
                .is_some_and(|latest| latest > projection.created_at)
        {
            return Ok(());
        }
        let next_local_count = existing
            .get("replyCount")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            .saturating_add(1);
        let reply_count = integer(&root, &["reply_count", "replyCount"])
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or_default()
            .max(next_local_count)
            .max(projection.topic_message_count);
        let parent_topic_message_count = reply_count;
        let mut topic = json!({
            "parentChannelId": parent_channel_id.as_str(),
            "topicChannelId": projection.topic_channel_id.as_str(),
            "channelId": projection.topic_channel_id.as_str(),
            "rootMessageId": projection.root_message_id.clone(),
            "title": projection.title.clone(),
            "replyCount": reply_count,
            "lastActivityAt": projection.created_at,
            "latestReplyId": projection.post_id,
        });
        if !projection.author_user_id.is_empty() {
            topic["latestReply"] = json!({
                "author": {"userId": projection.author_user_id},
                "preview": reply_preview(&projection.message_type, &projection.preview_text),
                "createdAt": projection.created_at,
            });
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![
                StorageOp::BatchUpdate(BatchUpdateSpec {
                    table: "message",
                    key_col: "id",
                    key_vals: vec![SqlValue::Text(projection.root_message_id)],
                    patch: vec![("topic".to_string(), SqlValue::Text(topic.to_string()))],
                }),
                StorageOp::BatchUpdate(BatchUpdateSpec {
                    table: "channel",
                    key_col: "id",
                    key_vals: vec![SqlValue::Text(parent_channel_id.as_str().to_string())],
                    patch: vec![(
                        "topic_msg_count".to_string(),
                        SqlValue::Integer(parent_topic_message_count.min(i64::MAX as u64) as i64),
                    )],
                }),
            ],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TopicActivityPersist {
                parent_channel_id,
                parent_topic_message_count,
            },
        );
        Ok(())
    }

    /// Root card 落稳后刷新父群 timeline，并原子推进父群的话题消息总数。
    pub(crate) fn handle_topic_activity_persist(
        &mut self,
        parent_channel_id: ChannelId,
        parent_topic_message_count: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if matches!(outcome, PortOutcome::Ok(_)) {
            self.refresh_attached_latest_timeline(parent_channel_id, None, out)?;
            out.push(
                crate::event::channel::update(json!({
                    "channelId": parent_channel_id.as_str(),
                    "topicMsgCount": parent_topic_message_count,
                }))?
                .into_effect(),
            );
        }
        Ok(())
    }
}

/// 将 topic 最新回复收紧成 reply preview 联合，卡片无需解释原始消息类型。
fn reply_preview(message_type: &str, text: &str) -> Value {
    match message_type.to_ascii_uppercase().as_str() {
        "DOCUMENT" => json!({"kind":"document","title":text}),
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "MEDIA" => {
            json!({"kind":"media","mediaType":message_type.to_ascii_lowercase(),"fileName":text})
        }
        "NOTICE" | "SYSTEM" | "SYSTEN" => json!({"kind":"system","text":text}),
        _ => json!({"kind":"text","text":text}),
    }
}

/// 解码 driver 的 rows JSON；失败保持空结果而不发布半份 root card。
fn storage_rows(outcome: &PortOutcome) -> Vec<Value> {
    let PortOutcome::Ok(reply) = outcome else {
        return Vec::new();
    };
    serde_json::from_slice::<Value>(reply.0.as_ref())
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

/// 兼容 SQLite snake row 与 render-ready camel row 的非空文本读取。
fn text<'a>(row: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
}

/// 兼容 SQLite snake row 与 render-ready camel row 的整数读取。
fn integer(row: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| row.get(*key).and_then(Value::as_i64))
}

/// 从文本列或对象列读取既有 topic JSON，供计数单调合并。
fn json_value(row: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|key| match row.get(*key) {
        Some(Value::String(raw)) => serde_json::from_str(raw).ok(),
        Some(value @ Value::Object(_)) => Some(value.clone()),
        _ => None,
    })
}
