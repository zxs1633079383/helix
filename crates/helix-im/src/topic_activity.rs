//! Topic child-post authority -> durable parent root-card activity projection.

use helix_core::effect::{BatchUpdateSpec, Effect, GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::{json, Value};

use crate::dialog_patch::PendingDialogPatch;
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
    /// 已提交的 topic dialog patch 才能启动 root 反向读，duplicate authority 不重复计数。
    pub(crate) fn schedule_topic_activity_projection(
        &mut self,
        pending: &PendingDialogPatch,
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
            // Dialog patch readback keeps the prior T total; the committed post contributes one.
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
        let Some((root_message_id, title, topic_message_count)) = relation.or_else(|| {
            self.state
                .dialog_list_frame
                .topic_link_metadata(pending.channel_id.as_str())
                .map(|(root_message_id, title)| (root_message_id, title, 0))
        }) else {
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
        let parent_topic_message_count = self
            .state
            .dialog_list_frame
            .topic_count_for(parent_channel_id.as_str())
            .saturating_add(1)
            .max(reply_count);
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
            if let Some(scope) = self.state.dialog_list_frame.scope().cloned() {
                let update = json!({
                    "channelId": parent_channel_id.as_str(),
                    "topicMsgCount": parent_topic_message_count,
                });
                if let Some(frame) = self
                    .state
                    .dialog_list_frame
                    .patch_from_channel_update(scope, &update)
                {
                    out.push(frame);
                }
            }
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

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use helix_core::effect::StorageOp;
    use helix_core::tick::ReplyBytes;

    use super::*;
    use crate::interaction_frame::SurfaceScope;
    use crate::module::ImConfig;

    /// Live post-forward authority must schedule the durable parent root readback.
    #[test]
    fn live_topic_post_schedules_parent_activity_projection() {
        let topic_channel_id = crate::state::test_channel_id(70);
        let pending = PendingDialogPatch {
            channel_id: topic_channel_id,
            event_seq: 1,
            msg_id: "reply-live".to_string(),
            fields: crate::sync_session::PostFields {
                user_id: "user-live".to_string(),
                msg_type: "TEXT".to_string(),
                message: "实时消息".to_string(),
                simple_message: "实时消息".to_string(),
                create_at: 101,
                ..crate::sync_session::PostFields::default()
            },
            update: crate::channel_write::PostChannelUpdate {
                channel_id: topic_channel_id,
                unread_delta: 1,
                unread_post_id: Some("reply-live".to_string()),
                last_post: "{}".to_string(),
                has_schedule_post: false,
                msg_create_at: 101,
                visible: true,
                sender_user_id: "user-live".to_string(),
                post_id: "reply-live".to_string(),
                last_message: "实时消息".to_string(),
                mentions: Vec::new(),
                urgent_user_ids: Vec::new(),
                mention_hit: false,
                urgent_hit: false,
            },
            source: "post-forward-authority".to_string(),
            causation_id: None,
        };
        let channel_reply = serde_json::to_vec(&json!([{
            "id": topic_channel_id.as_str(),
            "root_post_id": "root-live",
            "display_name": "实时话题",
            "topic_msg_count": 26
        }]))
        .unwrap();
        let mut module = ImModule::new(ImConfig::default());
        let mut out = EffectSink::new();

        module.schedule_topic_activity_projection(&pending, &channel_reply, &mut out);

        assert!(matches!(
            out.as_slice(),
            [Effect::Persist { ops, .. }]
                if matches!(ops.as_slice(), [StorageOp::Get(get)] if get.table == "message")
        ));
        let Effect::Persist { corr, .. } = &out.as_slice()[0] else {
            panic!("live topic post schedules a root read")
        };
        let Some(CorrelationContext::TopicActivityRootRead { projection }) =
            module.state.corr_map.remove(corr)
        else {
            panic!("root read retains topic activity authority")
        };
        assert_eq!(projection.topic_message_count, 27);
    }

    /// Root readback 必须单调增加回复数并写入 latestReply，不直接发布未持久化 Frame。
    #[test]
    fn root_readback_builds_durable_topic_activity_patch() {
        let topic_channel_id = crate::state::test_channel_id(71);
        let parent_channel_id = crate::state::test_channel_id(72);
        let mut module = ImModule::new(ImConfig::default());
        let mut out = EffectSink::new();
        let reply = serde_json::to_vec(&json!([{
            "id":"root-1",
            "channel_id":parent_channel_id.as_str(),
            "reply_count":3,
            "topic":"{\"replyCount\":2}"
        }]))
        .unwrap();

        module
            .handle_topic_activity_root_read(
                TopicActivityProjection {
                    topic_channel_id,
                    root_message_id: "root-1".to_string(),
                    title: "研发话题".to_string(),
                    post_id: "reply-3".to_string(),
                    author_user_id: "user-3".to_string(),
                    message_type: "TEXT".to_string(),
                    preview_text: "第三条".to_string(),
                    created_at: 30,
                    topic_message_count: 0,
                },
                &PortOutcome::Ok(ReplyBytes(Bytes::from(reply))),
                &mut out,
            )
            .unwrap();

        let Effect::Persist { ops, .. } = &out.as_slice()[0] else {
            panic!("topic activity requires one durable root patch")
        };
        let StorageOp::BatchUpdate(update) = &ops[0] else {
            panic!("root relation updates by message id")
        };
        let SqlValue::Text(topic) = &update.patch[0].1 else {
            panic!("topic activity persists as JSON text")
        };
        let topic: Value = serde_json::from_str(topic).unwrap();
        assert_eq!(topic["replyCount"], 3);
        assert_eq!(topic["lastActivityAt"], 30);
        assert_eq!(topic["latestReply"]["author"]["userId"], "user-3");
        assert_eq!(topic["latestReply"]["preview"]["text"], "第三条");
    }

    /// 服务端 root 绝对回复数必须覆盖被 post_update 擦回 relation-only 的旧 topic 摘要。
    #[test]
    fn root_readback_recovers_reply_count_from_root_authority() {
        let topic_channel_id = crate::state::test_channel_id(76);
        let parent_channel_id = crate::state::test_channel_id(77);
        let mut module = ImModule::new(ImConfig::default());
        let mut out = EffectSink::new();
        let reply = serde_json::to_vec(&json!([{
            "id":"root-1",
            "channel_id":parent_channel_id.as_str(),
            "reply_count":10,
            "topic":"{\"channelId\":\"topic-only\"}"
        }]))
        .unwrap();

        module
            .handle_topic_activity_root_read(
                TopicActivityProjection {
                    topic_channel_id,
                    root_message_id: "root-1".to_string(),
                    title: "研发话题".to_string(),
                    post_id: "reply-10".to_string(),
                    author_user_id: "user-10".to_string(),
                    message_type: "TEXT".to_string(),
                    preview_text: "第十条".to_string(),
                    created_at: 100,
                    topic_message_count: 0,
                },
                &PortOutcome::Ok(ReplyBytes(Bytes::from(reply))),
                &mut out,
            )
            .unwrap();

        let Effect::Persist { ops, .. } = &out.as_slice()[0] else {
            panic!("topic activity requires one durable root patch")
        };
        let StorageOp::BatchUpdate(update) = &ops[0] else {
            panic!("root relation updates by message id")
        };
        let SqlValue::Text(topic) = &update.patch[0].1 else {
            panic!("topic activity persists as JSON text")
        };
        let topic: Value = serde_json::from_str(topic).unwrap();
        assert_eq!(topic["replyCount"], 10);
        let StorageOp::BatchUpdate(channel_update) = &ops[1] else {
            panic!("topic count updates by topic channel id")
        };
        assert_eq!(channel_update.table, "channel");
        assert!(matches!(
            channel_update.key_vals.as_slice(),
            [SqlValue::Text(channel_id)] if channel_id == parent_channel_id.as_str()
        ));
        let SqlValue::Integer(topic_count) = &channel_update.patch[0].1 else {
            panic!("topic count persists as integer")
        };
        assert_eq!(*topic_count, 10);
    }

    /// 相同 reply id 的重放与更旧的乱序 post 都不得重复计数或回退卡片。
    #[test]
    fn duplicate_and_older_topic_posts_do_not_rewrite_root_activity() {
        let topic_channel_id = crate::state::test_channel_id(74);
        let parent_channel_id = crate::state::test_channel_id(75);
        for (post_id, created_at) in [("reply-latest", 30), ("reply-older", 29)] {
            let mut module = ImModule::new(ImConfig::default());
            let mut out = EffectSink::new();
            let reply = serde_json::to_vec(&json!([{
                "id":"root-1",
                "channel_id":parent_channel_id.as_str(),
                "topic":"{\"replyCount\":3,\"latestReplyId\":\"reply-latest\",\"lastActivityAt\":30}"
            }]))
            .unwrap();

            module
                .handle_topic_activity_root_read(
                    TopicActivityProjection {
                        topic_channel_id,
                        root_message_id: "root-1".to_string(),
                        title: "研发话题".to_string(),
                        post_id: post_id.to_string(),
                        author_user_id: "user-3".to_string(),
                        message_type: "TEXT".to_string(),
                        preview_text: "不应覆盖".to_string(),
                        created_at,
                        topic_message_count: 0,
                    },
                    &PortOutcome::Ok(ReplyBytes(Bytes::from(reply))),
                    &mut out,
                )
                .unwrap();

            assert!(out.as_slice().is_empty());
        }
    }

    /// Root patch 成功后只为已 attach 的父群 latest slot 排队权威本地 readback。
    #[test]
    fn durable_topic_activity_refreshes_attached_parent_timeline() {
        let parent_channel_id = crate::state::test_channel_id(73);
        let mut module = ImModule::new(ImConfig::default());
        module
            .state
            .timeline_projector
            .register_attachment(&SurfaceScope::Timeline {
                channel_id: parent_channel_id.as_str().to_string(),
                window_token: "latest".to_string(),
            });
        let mut out = EffectSink::new();

        module
            .handle_topic_activity_persist(
                parent_channel_id,
                3,
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .unwrap();

        assert!(matches!(
            out.as_slice(),
            [Effect::Persist { ops, .. }]
                if matches!(ops.as_slice(), [StorageOp::Scan(scan)] if scan.table == "message")
        ));
    }

    /// Topic activity 提交后必须把绝对消息数同步到父群，T 行保持无话题徽标。
    #[test]
    fn durable_topic_activity_patches_attached_parent_dialog_count() {
        let parent_channel_id = crate::state::test_channel_id(75);
        let topic_channel_id = crate::state::test_channel_id(76);
        let mut module = ImModule::new(ImConfig::default());
        let scope = crate::dialog_frame::DialogListScope::new("viewer", "company");
        let parent_row = crate::dialog_frame::row_from_dialog_projection(&json!({
            "id": parent_channel_id.as_str(),
            "type": "P",
            "displayName": "父群",
            "topicCount": 9,
        }))
        .expect("parent dialog row");
        let topic_row = crate::dialog_frame::row_from_dialog_projection(&json!({
            "id": topic_channel_id.as_str(),
            "type": "T",
            "displayName": "研发话题",
            "rootPostId": "root-1",
            "topicCount": 10,
        }))
        .expect("topic dialog row");
        module
            .state
            .dialog_list_frame
            .snapshot(scope, vec![parent_row, topic_row]);
        let mut out = EffectSink::new();

        module
            .handle_topic_activity_persist(
                parent_channel_id,
                10,
                &PortOutcome::Ok(ReplyBytes(Bytes::new())),
                &mut out,
            )
            .unwrap();

        let frame = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
                _ => None,
            })
            .expect("topic dialog count frame");
        let items = frame["data"]["mutations"][0]["mutation"]["patch"]["view"]["sections"][0]
            ["items"]
            .as_array()
            .expect("dialog items");
        let parent = items
            .iter()
            .find(|item| item["channelId"] == parent_channel_id.as_str())
            .expect("parent dialog");
        let topic = items
            .iter()
            .find(|item| item["channelId"] == topic_channel_id.as_str())
            .expect("topic dialog");
        assert_eq!(parent["topicCount"], 10);
        assert_eq!(topic["topicCount"], 0);
    }
}
