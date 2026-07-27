use crate::module::ImModule;
use crate::state::{ChannelId, ChannelSettingsProjection, CorrelationContext, MakeTopicProjection};
use helix_core::effect::{BatchUpdateSpec, GetSpec, Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};

impl ImModule {
    /// 将 make-topic HTTP authority 收口为 channel/message 的 durable relation barrier。
    pub(super) fn handle_outbound_make_topic_reply(
        &mut self,
        expected_root_message_id: &str,
        req_id: &str,
        display_name: &str,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            out.push(crate::acl::to_effect::emit_topic_operation_status(
                req_id,
                expected_root_message_id,
                "remote-failed",
                None,
                Some("remote-unavailable"),
                true,
            ));
            return;
        }
        let Some(projection) =
            decode_make_topic_projection(expected_root_message_id, req_id, display_name, outcome)
        else {
            out.push(crate::acl::to_effect::emit_topic_operation_status(
                req_id,
                expected_root_message_id,
                "remote-failed",
                None,
                Some("remote-invalid-response"),
                true,
            ));
            return;
        };
        out.push(crate::acl::to_effect::emit_topic_operation_status(
            req_id,
            expected_root_message_id,
            "remote-succeeded",
            Some(projection.topic_channel_id.as_str()),
            None,
            false,
        ));
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
                "display_name".to_string(),
                SqlValue::Text(projection.display_name.clone()),
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

    /// matching PersistOk 后读取 root durable fact，禁止从 HTTP body直接拼 renderer Frame。
    pub(super) fn handle_make_topic_persist_reply(
        &mut self,
        projection: MakeTopicProjection,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            out.push(crate::acl::to_effect::emit_topic_operation_status(
                &projection.req_id,
                &projection.root_message_id,
                "remote-failed",
                Some(projection.topic_channel_id.as_str()),
                Some("projection-persist-failed"),
                false,
            ));
            return;
        }
        let read_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: read_corr,
            ops: vec![StorageOp::Get(GetSpec {
                table: "message",
                key_col: "id",
                key_val: SqlValue::Text(projection.root_message_id.clone()),
            })],
        });
        self.state.corr_map.insert(
            read_corr,
            CorrelationContext::OutboundMakeTopicFrameRead { projection },
        );
    }

    /// 将成功 HTTP authority 转成单行 channel 持久屏障，失败或不完整回包保持无可见副作用。
    pub(super) fn handle_outbound_channel_settings_reply(
        &mut self,
        expected_channel_id: ChannelId,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let viewer_role = self
            .state
            .viewer_channel_roles
            .get(&expected_channel_id)
            .map(String::as_str)
            .or_else(|| {
                self.state
                    .channel_summary_projector
                    .current_view(expected_channel_id.as_str())
                    .map(|view| match view.viewer_role {
                        crate::interaction_frame::ChannelRole::Owner => "CREATOR",
                        crate::interaction_frame::ChannelRole::Admin => "MANAGER",
                        crate::interaction_frame::ChannelRole::Member => "MEMBER",
                    })
            });
        let Some(projection) =
            decode_settings_projection(expected_channel_id, causation_id, outcome, viewer_role)
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

    /// 在 channel PersistOk 后同时释放兼容事件与已 attach 的 typed summary Frame。
    pub(super) fn handle_channel_settings_persist_reply(
        &mut self,
        projection: ChannelSettingsProjection,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), crate::error::ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            return Ok(());
        }
        let mut channel = serde_json::json!({
            "purpose": projection.purpose.clone(),
            "header": projection.header.clone(),
        });
        if let (Some(settings), Some(display_name)) =
            (channel.as_object_mut(), projection.display_name.clone())
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
            projection.channel_capabilities.clone(),
        ) {
            out.push(effect);
        }
        self.project_channel_settings_projection_frame(&projection, now_ms, out)?;
        Ok(())
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
    req_id: &str,
    display_name: &str,
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
        req_id: req_id.to_string(),
        root_message_id: root_message_id.to_string(),
        topic_channel_id,
        display_name: display_name.to_string(),
        revision,
    })
}

/// 只接受同频道、完整设置字段与单调版本的服务端 SUCCESS 快照。
fn decode_settings_projection(
    expected_channel_id: ChannelId,
    causation_id: Option<String>,
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
        causation_id,
    })
}
