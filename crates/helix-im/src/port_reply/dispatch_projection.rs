use crate::module::ImModule;
use crate::state::{ChannelId, ChannelSettingsProjection, CorrelationContext, MakeTopicProjection};
use helix_core::effect::{BatchUpdateSpec, Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    pub(super) fn handle_outbound_make_topic_reply(
        &mut self,
        expected_root_message_id: &str,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let Some(projection) = decode_make_topic_projection(expected_root_message_id, outcome)
        else {
            return;
        };
        let persist_corr = self.alloc_corr_internal();
        let topic_json = serde_json::json!({
            "channelId": projection.topic_channel_id.as_str(),
        })
        .to_string();
        let channel_row: Row = vec![
            (
                "id".to_string(),
                SqlValue::Text(projection.topic_channel_id.as_str().to_string()),
            ),
            ("type".to_string(), SqlValue::Text("T".to_string())),
            (
                "root_post_id".to_string(),
                SqlValue::Text(projection.root_message_id.clone()),
            ),
            (
                "updated_at".to_string(),
                SqlValue::Integer(projection.revision as i64),
            ),
        ];
        let message_patch: Row = vec![
            ("topic".to_string(), SqlValue::Text(topic_json)),
            (
                "update_at".to_string(),
                SqlValue::Integer(projection.revision as i64),
            ),
        ];
        out.push(Effect::Persist {
            corr: persist_corr,
            ops: vec![
                StorageOp::BatchUpsert(UpsertSpec {
                    table: "channel",
                    rows: vec![channel_row],
                    conflict_key: Some("id"),
                    exclude_from_update: Vec::new(),
                }),
                StorageOp::BatchUpdate(BatchUpdateSpec {
                    table: "message",
                    key_col: "id",
                    key_vals: vec![SqlValue::Text(projection.root_message_id.clone())],
                    patch: message_patch,
                }),
            ],
        });
        self.state.corr_map.insert(
            persist_corr,
            CorrelationContext::OutboundMakeTopicPersist { projection },
        );
    }

    pub(super) fn handle_make_topic_persist_reply(
        &mut self,
        projection: MakeTopicProjection,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            return;
        }
        let req_id = format!("make-topic:{}", projection.root_message_id);
        out.push(crate::render_ready_replies::emit_channel_replies(
            &req_id,
            crate::render_ready_replies::FlatReplyProjection {
                channel_id: projection.topic_channel_id.as_str().to_string(),
                root_message_id: projection.root_message_id,
                projection_mode: crate::render_ready_replies::ReplyProjectionMode::Snapshot,
                revision: projection.revision,
                reply_ids: Vec::new(),
                nodes: Vec::new(),
                reply_count: 0,
                has_more: false,
            },
        ));
    }

    pub(super) fn handle_outbound_channel_settings_reply(
        &mut self,
        expected_channel_id: ChannelId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let viewer_role = self
            .state
            .viewer_channel_roles
            .get(&expected_channel_id)
            .map(String::as_str);
        let Some(projection) =
            decode_settings_projection(expected_channel_id, outcome, viewer_role)
        else {
            return;
        };
        let persist_corr = self.alloc_corr_internal();
        let mut row: Row = vec![
            (
                "id".to_string(),
                SqlValue::Text(projection.channel_id.as_str().to_string()),
            ),
            (
                "purpose".to_string(),
                SqlValue::Text(projection.purpose.clone()),
            ),
            (
                "header".to_string(),
                SqlValue::Text(projection.header.clone()),
            ),
            (
                "updated_at".to_string(),
                SqlValue::Integer(projection.setting_version as i64),
            ),
        ];
        if let Some(display_name) = projection.display_name.as_ref() {
            row.push((
                "display_name".to_string(),
                SqlValue::Text(display_name.clone()),
            ));
        }
        out.push(Effect::Persist {
            corr: persist_corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                table: "channel",
                rows: vec![row],
                conflict_key: Some("id"),
                exclude_from_update: Vec::new(),
            })],
        });
        self.state.corr_map.insert(
            persist_corr,
            CorrelationContext::OutboundChannelSettingsPersist { projection },
        );
    }

    pub(super) fn handle_channel_settings_persist_reply(
        &mut self,
        projection: ChannelSettingsProjection,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            return;
        }
        let mut channel = serde_json::json!({
            "purpose": projection.purpose,
            "header": projection.header,
        });
        if let (Some(settings), Some(display_name)) =
            (channel.as_object_mut(), projection.display_name)
        {
            settings.insert(
                "displayName".to_string(),
                serde_json::Value::String(display_name),
            );
        }
        if let Some(settings) = channel.as_object_mut() {
            settings.insert(
                "channelId".to_string(),
                serde_json::Value::String(projection.channel_id.as_str().to_string()),
            );
        }
        if let Some(scope) = self.state.dialog_list_frame.scope().cloned() {
            if let Some(frame) = self
                .state
                .dialog_list_frame
                .patch_from_channel_update(scope, &channel)
            {
                out.push(frame);
            }
        }
        if let Some(effect) = crate::acl::to_effect_s1::emit_channel_settings_updated(
            projection.channel_id,
            &channel,
            projection.setting_version,
            projection.channel_capabilities,
        ) {
            out.push(effect);
        }
    }
}

fn response_data(outcome: &PortOutcome) -> Option<serde_json::Value> {
    let PortOutcome::Ok(reply) = outcome else {
        return None;
    };
    let raw = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()).ok()?;
    let response: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    if response.get("status").and_then(serde_json::Value::as_str) != Some("SUCCESS") {
        return None;
    }
    response
        .get("data")
        .cloned()
        .filter(serde_json::Value::is_object)
}

fn decode_make_topic_projection(
    expected_root_message_id: &str,
    outcome: &PortOutcome,
) -> Option<MakeTopicProjection> {
    let data = response_data(outcome)?;
    let root_message_id = data
        .get("rootMessageId")
        .or_else(|| data.get("rootId"))
        .and_then(serde_json::Value::as_str)?;
    if root_message_id != expected_root_message_id {
        return None;
    }
    let topic_channel_id = data
        .get("channelId")
        .or_else(|| data.get("topicChannelId"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)?;
    let revision = data
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .filter(|revision| *revision <= i64::MAX as u64)?;
    Some(MakeTopicProjection {
        root_message_id: root_message_id.to_string(),
        topic_channel_id,
        revision,
    })
}

fn decode_settings_projection(
    expected_channel_id: ChannelId,
    outcome: &PortOutcome,
    viewer_role: Option<&str>,
) -> Option<ChannelSettingsProjection> {
    let data = response_data(outcome)?;
    let channel_id = data
        .get("channelId")
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)?;
    if channel_id != expected_channel_id {
        return None;
    }
    let settings = data
        .get("settings")
        .filter(|value| value.is_object())
        .unwrap_or(&data);
    let display_name = settings
        .get("displayName")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let purpose = settings.get("purpose")?.as_str()?.to_string();
    let header = settings.get("header")?.as_str()?.to_string();
    let setting_version = data
        .get("settingVersion")
        .or_else(|| settings.get("settingVersion"))
        .and_then(serde_json::Value::as_u64)
        .filter(|version| *version <= i64::MAX as u64)?;
    let capabilities_by_role = data
        .get("capabilitiesByRole")
        .or_else(|| settings.get("capabilitiesByRole"));
    Some(ChannelSettingsProjection {
        channel_id,
        display_name,
        purpose,
        header,
        setting_version,
        channel_capabilities: crate::channel_capabilities::for_viewer(
            capabilities_by_role,
            viewer_role,
        ),
    })
}
