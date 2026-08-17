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
        let authority = response_data(outcome).unwrap_or_else(|| serde_json::json!({}));
        let member_ids = authority
            .get("memberIds")
            .or_else(|| authority.get("member_ids"))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(serde_json::Value::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let participant_count = authority
            .get("participantCount")
            .or_else(|| authority.get("participant_count"))
            .and_then(serde_json::Value::as_i64)
            .filter(|value| *value >= 0)
            .unwrap_or(member_ids.len() as i64);
        let topic_json = serde_json::json!({
            "channelId": projection.topic_channel_id.as_str(),
            "members": member_ids,
            "participantCount": participant_count,
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

    /// matching PersistOk 后读取 root durable fact，禁止从 HTTP body 直接拼事件。
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
            CorrelationContext::OutboundMakeTopicRootRead { projection },
        );
    }

    /// 从持久化后的 root 行取得父频道，并先建立 topic channel.root_id 持久屏障。
    pub(super) fn handle_make_topic_thread_readback(
        &mut self,
        projection: MakeTopicProjection,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), crate::error::ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            return Ok(());
        };
        let rows = serde_json::from_slice::<Vec<serde_json::Value>>(reply.0.as_ref()).map_err(
            |error| crate::error::ImError::Parse(format!("topic root readback: {error}")),
        )?;
        let Some(root_row) = rows.first() else {
            return Ok(());
        };
        let parent_channel_id = root_row
            .get("channel_id")
            .or_else(|| root_row.get("channelId"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                crate::error::ImError::Parse(
                    "make-topic root row omitted parent channel".to_string(),
                )
            })?;
        let channel_id = ChannelId::from_str(parent_channel_id).ok_or_else(|| {
            crate::error::ImError::Parse(
                "make-topic root row has invalid parent channel".to_string(),
            )
        })?;
        // HTTP make-topic 的首段 upsert 还不知道父频道；root durable readback 是唯一可信来源。
        // 先把 root_id 写回 topic channel，再允许 channel_created/post_updated 对 UI 可见。
        let relation_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: relation_corr,
            ops: vec![StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "channel",
                key_col: "id",
                key_vals: vec![SqlValue::Text(
                    projection.topic_channel_id.as_str().to_string(),
                )],
                patch: vec![(
                    "root_id".to_string(),
                    SqlValue::Text(channel_id.as_str().to_string()),
                )],
            })],
        });
        self.state.corr_map.insert(
            relation_corr,
            CorrelationContext::OutboundMakeTopicRelationPersist {
                projection,
                parent_channel_id: channel_id,
                root_row: Box::new(root_row.clone()),
            },
        );
        Ok(())
    }

    /// 关系补丁成功后，才从已读回的 durable root 发布话题创建绝对态。
    pub(super) fn handle_make_topic_relation_persist_reply(
        &mut self,
        projection: MakeTopicProjection,
        parent_channel_id: ChannelId,
        root_row: Box<serde_json::Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), crate::error::ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            out.push(crate::acl::to_effect::emit_topic_operation_status(
                &projection.req_id,
                &projection.root_message_id,
                "remote-failed",
                Some(projection.topic_channel_id.as_str()),
                Some("relation-persist-failed"),
                false,
            ));
            return Ok(());
        }
        let root_row = *root_row;
        let mut topic = root_row
            .get("topic")
            .and_then(|value| match value {
                serde_json::Value::Object(_) => Some(value.clone()),
                serde_json::Value::String(encoded) => serde_json::from_str(encoded)
                    .ok()
                    .filter(serde_json::Value::is_object),
                _ => None,
            })
            .unwrap_or_else(|| serde_json::json!({}));
        let members = topic
            .get("members")
            .and_then(serde_json::Value::as_array)
            .filter(|values| !values.is_empty())
            .map(|values| serde_json::Value::Array(values.clone()))
            .unwrap_or_else(|| {
                root_row
                    .get("user_id")
                    .or_else(|| root_row.get("userId"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| serde_json::json!([value]))
                    .unwrap_or_else(|| serde_json::json!([]))
            });
        // 话题创建沿用既有频道绝对态，父频道与源消息锚点只从 durable root 读回确定。
        out.push(
            crate::event::channel::created(serde_json::json!({
                "id": projection.topic_channel_id.as_str(),
                "type": "T",
                "displayName": projection.display_name,
                "rootId": parent_channel_id.as_str(),
                "parentPostId": projection.root_message_id,
                "members": members,
                "memberCount": members.as_array().map_or(0, Vec::len),
                "unreadCount": 0,
                "mentionCount": 0,
            }))?
            .into_effect(),
        );

        let mut fields = crate::ws::parser::extract_post_fields(&root_row);
        if let Some(topic_object) = topic.as_object_mut() {
            topic_object
                .entry("channelId".to_string())
                .or_insert_with(|| serde_json::json!(projection.topic_channel_id.as_str()));
            topic_object
                .entry("participantCount".to_string())
                .or_insert_with(|| serde_json::json!(members.as_array().map_or(0, Vec::len)));
        }
        fields.topic = topic.to_string();
        // make-topic 已改变父频道中的 root row；旧 locate/older/newer coverage 含创建前快照，
        // 必须在发布可见事件前按频道失效，后续导航才能重新取得 durable topic 绝对态。
        self.state
            .invalidate_recent_message_coverage(parent_channel_id);
        self.state
            .invalidate_timeline_navigation_coverage(parent_channel_id);
        // 根消息的 postUpdated 必须排在 channelCreated 之后，且只投影 durable root 的字段。
        out.push(crate::acl::to_effect::emit_post_updated_for_viewer(
            parent_channel_id,
            projection.revision,
            projection.root_message_id.as_str(),
            &fields,
            "",
        ));
        Ok(())
    }

    /// 将成功 HTTP authority 转成单行 channel 持久屏障，失败或不完整回包保持无可见副作用。
    pub(super) fn handle_outbound_channel_settings_reply(
        &mut self,
        expected_channel_id: ChannelId,
        causation_id: Option<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        tracing::debug!(
            target: "cses.im.permission",
            stage = "settings.reply.before_decode",
            channel_id = expected_channel_id.as_str(),
            causation_id = ?causation_id,
            outcome_ok = matches!(outcome, PortOutcome::Ok(_)),
            has_http_reply = matches!(outcome, PortOutcome::Ok(reply) if !reply.0.is_empty()),
            "收到频道设置 authority 回包，准备校验投影"
        );
        let projection =
            match decode_settings_projection(expected_channel_id, causation_id, outcome) {
                Ok(projection) => projection,
                Err(reason) => {
                    tracing::debug!(
                        target: "cses.im.permission",
                        stage = "settings.reply.rejected",
                        channel_id = expected_channel_id.as_str(),
                        reason,
                        "频道设置 authority 回包未通过 ADR-007/字段/版本校验，保持 fail-closed"
                    );
                    return;
                }
            };
        tracing::debug!(
            target: "cses.im.permission",
            stage = "settings.reply.decoded",
            channel_id = projection.channel_id.as_str(),
            setting_version = projection.setting_version,
            has_display_name = projection.display_name.is_some(),
            "频道设置 authority 投影解析成功"
        );
        let persist_corr = self.alloc_corr_internal();
        let row = channel_settings_row(&projection);
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
        tracing::debug!(
            target: "cses.im.permission",
            stage = "settings.persist.accepted",
            channel_id = expected_channel_id.as_str(),
            persist_corr = persist_corr.raw(),
            "频道设置持久化 effect 已入队，等待 durable readback"
        );
    }

    /// 在 channel PersistOk 后释放频道设置的 MessageV3 权威事件。
    pub(super) fn handle_channel_settings_persist_reply(
        &mut self,
        projection: ChannelSettingsProjection,
        outcome: &PortOutcome,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), crate::error::ImError> {
        if !matches!(outcome, PortOutcome::Ok(_)) {
            tracing::debug!(
                target: "cses.im.permission",
                stage = "settings.persist.rejected",
                channel_id = projection.channel_id.as_str(),
                setting_version = projection.setting_version,
                reason = "storage_effect_failed",
                "频道设置持久化未确认，不发布 Angular 权威事件"
            );
            return Ok(());
        }
        tracing::debug!(
            target: "cses.im.permission",
            stage = "settings.persist.completed",
            channel_id = projection.channel_id.as_str(),
            setting_version = projection.setting_version,
            "频道设置持久化已确认，准备生成 channel update"
        );
        let mut channel = projection.channel;
        let Some(channel_map) = channel.as_object_mut() else {
            tracing::debug!(
                target: "cses.im.permission",
                stage = "settings.emit.rejected",
                channel_id = projection.channel_id.as_str(),
                reason = "channel_projection_not_object",
                "频道设置投影不是对象，阻止伪造 Angular 事件"
            );
            return Ok(());
        };
        // 只补齐已由 Go authority 确认的标识和版本，其他字段原样保留，避免构造伪 DialogItem。
        channel_map.insert(
            "id".to_string(),
            serde_json::Value::String(projection.channel_id.as_str().to_string()),
        );
        channel_map.insert(
            "settingVersion".to_string(),
            serde_json::Value::from(projection.setting_version),
        );
        match crate::event::channel::update(channel) {
            Ok(event) => {
                tracing::debug!(
                    target: "cses.im.permission",
                    stage = "settings.emit.accepted",
                    channel_id = projection.channel_id.as_str(),
                    setting_version = projection.setting_version,
                    "频道设置 channel update 已生成，等待总线/Angular 消费"
                );
                out.push(event.into_effect());
            }
            Err(error) => {
                tracing::debug!(
                    target: "cses.im.permission",
                    stage = "settings.emit.rejected",
                    channel_id = projection.channel_id.as_str(),
                    reason = "channel_event_parse_failed",
                    error = ?error,
                    "频道设置 channel update 生成失败"
                );
            }
        }
        Ok(())
    }
}

/// 将 Go authority 设置快照裁剪为 channel durable row；缺失可选字段时保持旧值。
fn channel_settings_row(projection: &ChannelSettingsProjection) -> Row {
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
    if let Some(orient) = projection.orient.as_ref() {
        row.push(("orient".to_string(), SqlValue::Text(orient.clone())));
    }
    row
}

fn response_data(outcome: &PortOutcome) -> Option<serde_json::Value> {
    response_data_with_reason(outcome).ok()
}

/// 解包并校验 Go 成功响应；返回有限原因供权限 trace 诊断，绝不暴露响应正文。
fn response_data_with_reason(outcome: &PortOutcome) -> Result<serde_json::Value, &'static str> {
    let PortOutcome::Ok(reply) = outcome else {
        return Err("port_outcome_not_ok");
    };
    let raw = crate::http_envelope::unwrap_success_envelope(reply.0.as_ref(), "channel_settings")
        .map_err(|_| "http_envelope_invalid_or_non_2xx")?;
    let response: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| "body_json_invalid")?;
    if response.get("status").and_then(serde_json::Value::as_str) != Some("SUCCESS") {
        return Err("body_status_not_success");
    }
    response
        .get("data")
        .cloned()
        .filter(serde_json::Value::is_object)
        .ok_or("body_data_missing_or_not_object")
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
) -> Result<ChannelSettingsProjection, &'static str> {
    let data = response_data_with_reason(outcome)?;
    let channel_id = data
        .get("channelId")
        .or_else(|| data.get("id"))
        .and_then(serde_json::Value::as_str)
        .ok_or("channel_id_missing_or_invalid")
        .and_then(|value| ChannelId::from_str(value).ok_or("channel_id_format_invalid"))?;
    if channel_id != expected_channel_id {
        return Err("channel_id_mismatch");
    }
    let settings = data
        .get("settings")
        .filter(|value| value.is_object())
        .unwrap_or(&data);
    let display_name = settings
        .get("displayName")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let orient = settings
        .get("orient")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let purpose = settings
        .get("purpose")
        .and_then(serde_json::Value::as_str)
        .ok_or("purpose_missing_or_invalid")?
        .to_string();
    let header = settings
        .get("header")
        .and_then(serde_json::Value::as_str)
        .ok_or("header_missing_or_invalid")?
        .to_string();
    let setting_version = data
        .get("settingVersion")
        .or_else(|| settings.get("settingVersion"))
        .and_then(serde_json::Value::as_u64)
        .filter(|version| *version <= i64::MAX as u64)
        .ok_or("setting_version_missing_or_invalid")?;
    Ok(ChannelSettingsProjection {
        channel_id,
        channel: data.clone(),
        display_name,
        orient,
        purpose,
        header,
        setting_version,
        causation_id,
    })
}

#[cfg(test)]
mod tests {
    use super::{channel_settings_row, decode_settings_projection, response_data_with_reason};
    use crate::state::ChannelId;
    use bytes::Bytes;
    use helix_core::tick::{PortOutcome, ReplyBytes};
    use serde_json::{json, Value};

    /// 测试专用 ADR-007 base64 编码器，与 host 的标准表保持一致。
    fn encode_base64(bytes: &[u8]) -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).copied().unwrap_or_default() as u32;
            let b2 = chunk.get(2).copied().unwrap_or_default() as u32;
            let value = (b0 << 16) | (b1 << 8) | b2;
            output.push(TABLE[((value >> 18) & 63) as usize] as char);
            output.push(TABLE[((value >> 12) & 63) as usize] as char);
            output.push(if chunk.len() > 1 {
                TABLE[((value >> 6) & 63) as usize] as char
            } else {
                '='
            });
            output.push(if chunk.len() > 2 {
                TABLE[(value & 63) as usize] as char
            } else {
                '='
            });
        }
        output
    }

    /// 构造 Host 回灌的 HTTP 信封，确保测试覆盖真实 Go body 解包边界。
    fn outcome(data: Value) -> PortOutcome {
        let body = serde_json::to_vec(&json!({ "status": "SUCCESS", "data": data })).unwrap();
        let envelope = serde_json::to_vec(&json!({
            "status": 200,
            "headers": [],
            "body": encode_base64(&body),
        }))
        .unwrap();
        PortOutcome::Ok(ReplyBytes(Bytes::from(envelope)))
    }

    /// 成功设置回包必须能形成后续持久化与事件所需的完整投影。
    #[test]
    fn decodes_complete_settings_projection_from_adr007_reply() {
        let channel_id = ChannelId::from_str("chfixx00000000000000000001").unwrap();
        let projection = decode_settings_projection(
            channel_id,
            Some("op-settings-1".to_string()),
            &outcome(json!({
                "id": channel_id.as_str(),
                "purpose": "purpose",
                "header": "notice",
                "orient": "delivery-quality",
                "settingVersion": 17,
                "canEditChannelSettings": true,
            })),
        )
        .expect("complete authority projection should decode");
        assert_eq!(projection.channel_id, channel_id);
        assert_eq!(projection.setting_version, 17);
        assert_eq!(projection.purpose, "purpose");
        assert_eq!(projection.header, "notice");
        assert_eq!(projection.orient.as_deref(), Some("delivery-quality"));
        assert_eq!(projection.causation_id.as_deref(), Some("op-settings-1"));
        let row = channel_settings_row(&projection);
        assert!(row.iter().any(|(column, value)| {
            column == "orient"
                && matches!(value, helix_core::effect::SqlValue::Text(value) if value == "delivery-quality")
        }));
    }

    /// 缺少 purpose/header/version 的 HTTP 200 不能制造空 channel update。
    #[test]
    fn rejects_sparse_success_body_instead_of_emitting_partial_projection() {
        let channel_id = ChannelId::from_str("chfixx00000000000000000001").unwrap();
        let sparse = outcome(json!({
            "id": channel_id.as_str(),
            "mentionPermission": "MANAGER",
        }));
        assert_eq!(
            decode_settings_projection(channel_id, None, &sparse),
            Err("purpose_missing_or_invalid")
        );
    }

    /// 只有 ADR-007 成功 body 才能进入设置投影解析。
    #[test]
    fn rejects_non_success_body_before_projection() {
        let body = serde_json::to_vec(&json!({ "status": "ERROR", "data": {} })).unwrap();
        let envelope = serde_json::to_vec(&json!({
            "status": 200,
            "headers": [],
            "body": encode_base64(&body),
        }))
        .unwrap();
        let outcome = PortOutcome::Ok(ReplyBytes(Bytes::from(envelope)));
        assert_eq!(
            response_data_with_reason(&outcome),
            Err("body_status_not_success")
        );
    }

    /// HTTP 非 2xx 即使 body 伪装 SUCCESS 也不能推进设置持久化屏障。
    #[test]
    fn rejects_non_2xx_http_envelope_before_body_success() {
        let body = serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {"id": "chfixx00000000000000000001"}
        }))
        .unwrap();
        let envelope = serde_json::to_vec(&json!({
            "status": 409,
            "headers": [],
            "body": encode_base64(&body),
        }))
        .unwrap();
        let outcome = PortOutcome::Ok(ReplyBytes(Bytes::from(envelope)));
        assert_eq!(
            response_data_with_reason(&outcome),
            Err("http_envelope_invalid_or_non_2xx")
        );
    }
}
