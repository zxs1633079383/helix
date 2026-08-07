use crate::error::ImError;
use crate::module::ImModule;
use crate::state::CorrelationContext;
use helix_core::EffectSink;

impl ImModule {
    /// 恢复完成后重新读取当前 viewer 的频道列表。
    pub(crate) fn refresh_channel_list(
        &mut self,
        causation_id: Option<String>,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "req_id": causation_id,
        }))
        .map_err(|error| ImError::Serialize(format!("attached dialog refresh: {error}")))?;
        self.handle_query_command("im_query_dialog_list", &payload, out)?;
        Ok(true)
    }

    /// P6 投影/状态命令分发（薄壳，契约在 `crate::query`）。
    ///
    /// - `im_query_messages_by_channel`：进入 local-first 三段状态机；driver 不解释 fallback。
    /// - `im_delete_all_dialogs`：清内存 channel gate + emit 刷新（物理 truncate 缺原语，见 query.rs）。
    pub(crate) fn handle_query_command(
        &mut self,
        name: &str,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match name {
            "im_query_messages_by_channel" => {
                self.dispatch_message_query(payload, out)?;
            }
            "im_query_channel_sync_page" => {
                self.start_channel_sync_page(payload, out)?;
            }
            "im_complete_channel_sync" => {
                self.complete_channel_sync(payload, out)?;
            }
            "im_query_dialog_list" => {
                // UC-3：低频按需拉全量会话列表（off hot-path）→ Scan channel + 注册 DialogListQuery；
                // PortReply 回报后只 emit 一次 `im:read:result` typed Result（前端零计算）。
                let corr = self.alloc_corr_internal();
                let effect = self.build_dialog_list_query_for_runtime(payload, corr)?;
                self.state.corr_map.insert(
                    corr,
                    CorrelationContext::DialogListQuery {
                        causation_id: crate::query::read_relay::read_req_id(payload),
                    },
                );
                out.push(effect);
            }
            "im_query_subtopics" => {
                // G13d：只按 RuntimeAuth 查询本地 parent/topic 关系；空 parent 直接 typed-empty。
                let req_id = crate::query::read_relay::read_req_id(payload);
                let request = match crate::query::parse_subtopics_query(payload) {
                    Ok(request) => request,
                    Err(error) => {
                        tracing::warn!(error = ?error, "subtopic query rejected");
                        out.push(self.emit_subtopics_result_for_runtime(
                            req_id.as_deref(),
                            None,
                            b"[]",
                        ));
                        return Ok(());
                    }
                };
                let Some(parent_channel_id) = request.parent_channel_id.clone() else {
                    out.push(self.emit_subtopics_result_for_runtime(
                        req_id.as_deref(),
                        None,
                        b"[]",
                    ));
                    return Ok(());
                };
                let corr = self.alloc_corr_internal();
                let effect = match self.build_subtopics_query_for_runtime(&request, corr) {
                    Ok(effect) => effect,
                    Err(error) => {
                        tracing::warn!(error = ?error, "subtopic query scope rejected");
                        out.push(self.emit_subtopics_result_for_runtime(
                            req_id.as_deref(),
                            Some(parent_channel_id.as_str()),
                            b"[]",
                        ));
                        return Ok(());
                    }
                };
                self.state.corr_map.insert(
                    corr,
                    CorrelationContext::SubtopicsQuery {
                        parent_channel_id,
                        causation_id: req_id,
                    },
                );
                out.push(effect);
            }
            "im_delete_all_dialogs" => {
                // 内存 channel gate 清空（core 拥有的状态；物理 DELETE 留待 truncate 原语，query.rs）。
                self.state.channels.clear();
                self.state.reset_recent_query_coverage();
                out.push(crate::query::emit_dialogs_cleared());
            }
            // G11b 上拉页：只用 reqId/scope/anchor correlation，不读取 attached visual window。
            n if n == crate::older_context::LOAD_OLDER_CONTEXT => {
                let (channel_id, anchor_post_id, anchor_create_at, page_size, request_id) =
                    crate::timeline_navigation::parse_older_request_with_anchor(payload)?;
                let state =
                    crate::timeline_navigation::TimelineNavigationState::older_without_window(
                        channel_id,
                        page_size,
                        request_id,
                        anchor_post_id,
                        anchor_create_at,
                    );
                self.start_timeline_navigation_or_local(state, out);
            }
            // G11c 下拉页：Go 解析 anchor，Helix 不保存 window/cursor。
            n if n == crate::timeline_navigation::LOAD_NEWER_CONTEXT => {
                let (channel_id, anchor_post_id, anchor_create_at, page_size, request_id) =
                    crate::timeline_navigation::parse_newer_request_with_anchor(payload)?;
                let state =
                    crate::timeline_navigation::TimelineNavigationState::newer_without_window(
                        channel_id,
                        page_size,
                        request_id,
                        anchor_post_id,
                        anchor_create_at,
                    );
                self.start_timeline_navigation_or_local(state, out);
            }
            // G11d 定位直接进入 Go context authority；两条命令名共享严格的持久读回链。
            n if n == crate::timeline_navigation::LOCATE_MESSAGE
                || n == crate::timeline_navigation::LOCATE_CONTEXT =>
            {
                let (channel_id, target_message_id, page_size, request_id, navigation_token) =
                    crate::timeline_navigation::parse_locate_request_for_command(payload, n)?;
                let navigation_token = navigation_token
                    .or_else(|| request_id.clone())
                    .unwrap_or_else(|| format!("locate:{target_message_id}"));
                let state = crate::timeline_navigation::TimelineNavigationState::locate(
                    channel_id,
                    format!("timeline:{}", navigation_token),
                    page_size,
                    request_id,
                    target_message_id,
                    navigation_token,
                );
                self.start_timeline_navigation_or_local(state, out);
            }
            _ => tracing::debug!("im: query dispatch miss '{}'", name),
        }
        Ok(())
    }

    /// 校验当前 RuntimeAuth 下的 channel-sync session，并发起有界本地 Scan。
    fn start_channel_sync_page(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = match crate::channel_sync::parse_page_request(payload) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(error = ?error, "channel sync page request rejected");
                if let Some(req_id) = crate::query::read_relay::read_req_id(payload) {
                    out.push(crate::read_relay::emit_read_error(
                        req_id.as_str(),
                        error.to_string().as_str(),
                    ));
                }
                return Ok(());
            }
        };
        let Some(session) = self.state.channel_sync_session.clone() else {
            tracing::debug!("channel sync page ignored before ready");
            if let Some(req_id) = request.req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync session is not ready",
                ));
            }
            return Ok(());
        };
        if !session.matches_scope(
            request.channel_sync_session_id.as_str(),
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        ) {
            tracing::warn!("channel sync page scope mismatch");
            if let Some(req_id) = request.req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync page scope mismatch",
                ));
            }
            return Ok(());
        }
        let offset = match request.next_cursor.as_deref() {
            None => 0,
            Some(cursor) => match crate::channel_sync::decode_cursor(cursor, &session) {
                Ok(offset) => offset,
                Err(error) => {
                    tracing::warn!(error = ?error, "channel sync cursor rejected");
                    if let Some(req_id) = request.req_id.as_deref() {
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            error.to_string().as_str(),
                        ));
                    }
                    return Ok(());
                }
            },
        };
        let corr = self.alloc_corr_internal();
        let effect = match crate::channel_sync::page_scan_effect(
            corr,
            session.company_id.as_str(),
            offset,
        ) {
            Ok(effect) => effect,
            Err(error) => {
                tracing::warn!(error = ?error, "channel sync page scan rejected");
                if let Some(req_id) = request.req_id.as_deref() {
                    out.push(crate::read_relay::emit_read_error(
                        req_id,
                        error.to_string().as_str(),
                    ));
                }
                return Ok(());
            }
        };
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ChannelSyncPage {
                channel_sync_session_id: session.channel_sync_session_id,
                generation: session.generation,
                offset,
                req_id: request.req_id,
            },
        );
        tracing::info!(
            corr = corr.raw(),
            offset,
            "channel sync page local scan scheduled"
        );
        out.push(effect);
        Ok(())
    }

    /// 关闭当前身份绑定的频道分页 session；重复 complete 是幂等 no-op。
    fn complete_channel_sync(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = match crate::channel_sync::parse_complete_request(payload) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(error = ?error, "channel sync complete request rejected");
                if let Some(req_id) = crate::query::read_relay::read_req_id(payload) {
                    out.push(crate::read_relay::emit_read_error(
                        req_id.as_str(),
                        error.to_string().as_str(),
                    ));
                }
                return Ok(());
            }
        };
        let Some(session) = self.state.channel_sync_session.as_mut() else {
            tracing::debug!("channel sync complete ignored before ready");
            if let Some(req_id) = request.req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync session is not ready",
                ));
            }
            return Ok(());
        };
        if session.channel_sync_session_id != request.channel_sync_session_id
            || session.account_id != self.config.auth_user_id
            || session.company_id != self.config.company_id
        {
            tracing::warn!("channel sync complete scope mismatch or already completed");
            if let Some(req_id) = request.req_id.as_deref() {
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "channel sync complete scope mismatch",
                ));
            }
            return Ok(());
        }
        let first_complete = !session.completed;
        session.completed = true;
        let session_id = session.channel_sync_session_id.clone();
        let generation = session.generation;
        if first_complete {
            // 关闭仅撤销当前 session 的在途 page；其他业务 correlation 不受影响。
            self.state.corr_map.retain(|_, context| {
                !matches!(
                    context,
                    CorrelationContext::ChannelSyncPage {
                        channel_sync_session_id: pending_session_id,
                        generation: pending_generation,
                        ..
                    }
                    | CorrelationContext::ChannelSyncPageMemberSnapshot {
                        channel_sync_session_id: pending_session_id,
                        generation: pending_generation,
                        ..
                    } if pending_session_id == &session_id && *pending_generation == generation
                )
            });
            // 保留一次 typed complete，供非 query waiter 的客户端消费。
            out.push(
                crate::event::channel_sync::complete(session_id.as_str(), generation)?
                    .into_effect(),
            );
        }
        if let Some(req_id) = request.req_id.as_deref() {
            out.push(crate::read_relay::emit_read_body(
                req_id,
                serde_json::json!({
                    "completed": true,
                    "channelSyncSessionId": session_id,
                    "generation": generation,
                }),
            ));
        }
        if first_complete && self.state.channel_sync_refresh_pending {
            self.state.channel_sync_refresh_pending = false;
            tracing::info!(
                "channel-sync-ready 延迟批次已合并：当前分页 session complete 后重新开放"
            );
            self.open_channel_sync_session(out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod phase2_contract_tests {
    use super::*;
    use helix_core::effect::Effect;
    use helix_core::EffectSink;

    const CHANNEL_ID: &str = "chfixx0000000000000000002a";

    /// 冻结查询入口的单 corr 注册、20 页长和当前窗口代际。
    #[test]
    fn message_query_registers_one_correlated_generation() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": CHANNEL_ID,
            "pageSize": 20,
            "req_id": "phase2-query-1",
        }))
        .expect("query payload serializes");
        let mut out = EffectSink::new();

        module
            .handle_query_command("im_query_messages_by_channel", &payload, &mut out)
            .expect("query command is accepted");

        let Some(Effect::Persist { corr, .. }) = out.as_slice().first() else {
            panic!("query command must start one local Scan persist");
        };
        assert_eq!(module.state.corr_map.len(), 1);
        assert!(matches!(
            module.state.corr_map.get(corr),
            Some(CorrelationContext::MessageQueryLocal {
                request,
                query_generation: 1,
                ..
            }) if request.limit == 20 && request.channel_id.as_str() == CHANNEL_ID
        ));
    }

    /// 账号/会话边界必须使旧窗口代际失效，避免迟到回包污染新身份。
    #[test]
    fn identity_reset_restarts_message_query_generation() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let channel_id =
            crate::state::ChannelId::from_str(CHANNEL_ID).expect("test channel id is valid");
        assert_eq!(
            module
                .state
                .begin_message_query_generation(channel_id, "latest"),
            1
        );
        assert_eq!(
            module
                .state
                .begin_message_query_generation(channel_id, "latest"),
            2
        );
        let old_epoch = module.state.query_session_epoch;
        module.state.reset_message_v3_identity();
        assert!(module.state.query_session_epoch > old_epoch);
        assert_eq!(
            module
                .state
                .begin_message_query_generation(channel_id, "latest"),
            1
        );
        assert!(!module
            .state
            .is_current_message_query_generation(channel_id, "latest", 2));

        module.state.channel_sync_session = Some(crate::channel_sync::ChannelSyncSession::new(
            "session-a".to_string(),
            1,
            "user-a",
            "company-a",
        ));
        module.state.reset_message_v3_identity();
        assert!(module.state.channel_sync_session.is_none());
    }

    /// 未附着窗口的 older 请求直接进入 Timeline Reactor authority HTTP，不读取视觉窗口。
    #[test]
    fn older_without_attached_window_uses_direct_authority_http() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let mut out = EffectSink::new();
        module
            .handle_query_command(
                crate::older_context::LOAD_OLDER_CONTEXT,
                &serde_json::to_vec(&serde_json::json!({
                    "channel_id": CHANNEL_ID,
                    "anchor_post_id": "anchor-not-attached",
                    "pageSize": 40,
                    "req_id": "older-failure-1",
                }))
                .expect("older payload serializes"),
                &mut out,
            )
            .expect("missing attachment must use direct authority HTTP");

        let Effect::Http { req, .. } = out.as_slice().first().expect("authority HTTP emitted")
        else {
            panic!("missing attachment must emit posts/getPostsAfterIndex HTTP");
        };
        assert_eq!(req.method, "POST");
        assert!(req.url.ends_with("/posts/getPostsAfterIndex"));
        let body: serde_json::Value =
            serde_json::from_slice(req.body.as_ref().expect("authority request body")).unwrap();
        assert_eq!(body["postIds"], "anchor-not-attached");
        assert_eq!(body["direction"], "older");
        assert_eq!(body["pageSize"], 40);
        assert_eq!(body["anchor"]["postId"], "anchor-not-attached");
        assert!(body["anchor"]["createAt"].is_null());
        assert!(req
            .headers
            .iter()
            .any(|(key, value)| key == "Cses-Track-Id" && value == "older-failure-1"));
    }

    /// ready 后 page 只注册当前 session 的 account-local Scan，未知 session fail-closed。
    #[test]
    fn channel_sync_page_registers_scoped_scan() {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "user-a".to_string();
        config.company_id = "company-a".to_string();
        let mut module = ImModule::new(config);
        module.state.channel_sync_session = Some(crate::channel_sync::ChannelSyncSession::new(
            "session-a".to_string(),
            4,
            "user-a",
            "company-a",
        ));
        let mut out = EffectSink::new();
        module
            .handle_query_command(
                "im_query_channel_sync_page",
                br#"{"channel_sync_session_id":"session-a","next_cursor":null,"page_size":20,"req_id":"page-1"}"#,
                &mut out,
            )
            .expect("page query accepted");
        let Some(Effect::Persist { corr, ops }) = out.as_slice().first() else {
            panic!("page query must emit local scan");
        };
        assert!(matches!(
            ops.first(),
            Some(helix_core::effect::StorageOp::Scan(scan))
                if scan.table == "channel"
                    && scan.limit == Some(21)
                    && matches!(
                        scan.filter,
                        Some(("team_id", helix_core::effect::SqlValue::Text(ref value)))
                            if value == "company-a"
                    )
        ));
        assert!(matches!(
            module.state.corr_map.get(corr),
            Some(CorrelationContext::ChannelSyncPage {
                channel_sync_session_id,
                generation: 4,
                offset: 0,
                req_id: Some(req_id),
            }) if channel_sync_session_id == "session-a" && req_id == "page-1"
        ));

        let session = module.state.channel_sync_session.as_ref().unwrap();
        let cursor = crate::channel_sync::encode_cursor(session, 20);
        module
            .handle_query_command(
                "im_query_channel_sync_page",
                &serde_json::to_vec(&serde_json::json!({
                    "channel_sync_session_id": "session-a",
                    "next_cursor": cursor,
                    "page_size": 20,
                    "req_id": "page-2",
                }))
                .unwrap(),
                &mut out,
            )
            .expect("second page query accepted");
        let Some(Effect::Persist { ops, .. }) = out.as_slice().last() else {
            panic!("second page query must emit local scan");
        };
        assert!(matches!(
            ops.first(),
            Some(helix_core::effect::StorageOp::Scan(scan)) if scan.limit == Some(41)
        ));

        let before = out.as_slice().len();
        module
            .handle_query_command(
                "im_query_channel_sync_page",
                br#"{"channel_sync_session_id":"other-session","page_size":20}"#,
                &mut out,
            )
            .expect("unknown session is handled");
        assert_eq!(out.as_slice().len(), before);
    }

    /// complete 只关闭同一身份的会话，并为 Host waiter 提供成功/失败回灌。
    #[test]
    fn channel_sync_complete_is_scoped_and_idempotent() {
        let mut config = crate::module::ImConfig::default();
        config.auth_user_id = "user-a".to_string();
        config.company_id = "company-a".to_string();
        let mut module = ImModule::new(config);
        module.state.channel_sync_session = Some(crate::channel_sync::ChannelSyncSession::new(
            "session-a".to_string(),
            4,
            "user-a",
            "company-a",
        ));
        let mut out = EffectSink::new();
        module
            .handle_query_command(
                "im_complete_channel_sync",
                br#"{"channel_sync_session_id":"other-session","req_id":"bad-complete"}"#,
                &mut out,
            )
            .expect("wrong scope is handled");
        assert_eq!(out.as_slice().len(), 1);
        let error: serde_json::Value = match &out.as_slice()[0] {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
            _ => panic!("scope failure must resolve query waiter"),
        };
        assert_eq!(error["event"], "im:read:result");
        assert_eq!(error["data"]["req_id"], "bad-complete");
        assert!(error["data"].get("error").is_some());
        out.clear();
        module
            .handle_query_command(
                "im_complete_channel_sync",
                br#"{"channel_sync_session_id":"session-a","req_id":"done-complete"}"#,
                &mut out,
            )
            .expect("complete emits event");
        assert_eq!(out.as_slice().len(), 2);
        let read_result = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => {
                    let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                    (value["event"] == "im:read:result").then_some(value)
                }
                _ => None,
            })
            .expect("complete waiter result");
        assert_eq!(read_result["data"]["req_id"], "done-complete");
        assert_eq!(read_result["data"]["body"]["completed"], true);
        assert!(
            module
                .state
                .channel_sync_session
                .as_ref()
                .unwrap()
                .completed
        );
        out.clear();
        module
            .handle_query_command(
                "im_complete_channel_sync",
                br#"{"channel_sync_session_id":"session-a","req_id":"done-again"}"#,
                &mut out,
            )
            .expect("duplicate complete is handled");
        assert_eq!(out.as_slice().len(), 1);
        let duplicate: serde_json::Value = match &out.as_slice()[0] {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
            _ => panic!("duplicate complete must resolve waiter"),
        };
        assert_eq!(duplicate["event"], "im:read:result");
    }
}
