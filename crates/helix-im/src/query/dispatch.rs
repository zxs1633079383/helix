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
            "im_query_dialog_list" => {
                // UC-3：低频按需拉全量会话列表（off hot-path）→ Scan channel + 注册 DialogListQuery；
                // PortReply 回报后 emit `message.left-dialog-list` 完整 snapshot（前端零计算）。
                let corr = self.alloc_corr_internal();
                let effect = crate::query::build_dialog_list_query(payload, corr)?;
                self.state.corr_map.insert(
                    corr,
                    CorrelationContext::DialogListQuery {
                        causation_id: crate::query::read_relay::read_req_id(payload),
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
            // G11a 上拉页：与 newer 共享复合 cursor、cache barrier 和 readback 状态机。
            n if n == crate::older_context::LOAD_OLDER_CONTEXT => {
                let (channel_id, anchor_post_id, page_size, request_id) =
                    crate::timeline_navigation::parse_older_request(payload)?;
                let (page, _, cursor) = self
                    .state
                    .timeline_state
                    .paging_context_for_anchor(channel_id.as_str(), &anchor_post_id)
                    .ok_or_else(|| {
                        ImError::Parse(
                            "im_load_older_context requires one attached timeline containing anchor"
                                .to_string(),
                        )
                    })?;
                let state = crate::timeline_navigation::TimelineNavigationState::older(
                    channel_id,
                    page,
                    page_size,
                    request_id,
                    anchor_post_id,
                    cursor,
                );
                self.start_timeline_navigation_http(state, out);
            }
            // G10b 下拉页：cursor 只从 attached typed state 解析，端侧不能伪造排序边界。
            n if n == crate::timeline_navigation::LOAD_NEWER_CONTEXT => {
                let (channel_id, anchor_post_id, page_size, request_id) =
                    crate::timeline_navigation::parse_newer_request(payload)?;
                let (page, _, cursor) = self
                    .state
                    .timeline_state
                    .paging_context_for_anchor(channel_id.as_str(), &anchor_post_id)
                    .ok_or_else(|| {
                        ImError::Parse(
                            "im_load_newer_context requires one attached timeline containing anchor"
                                .to_string(),
                        )
                    })?;
                let state = crate::timeline_navigation::TimelineNavigationState::newer(
                    channel_id,
                    page,
                    page_size,
                    request_id,
                    anchor_post_id,
                    cursor,
                );
                self.start_timeline_navigation_http(state, out);
            }
            // G11a 定位直接进入 Go context authority；本地缓存不能替代居中窗口的权威上下文。
            n if n == crate::timeline_navigation::LOCATE_MESSAGE => {
                let (channel_id, target_message_id, page_size, request_id, navigation_token) =
                    crate::timeline_navigation::parse_locate_request(payload)?;
                let (window_token, _) = self
                    .state
                    .timeline_state
                    .unique_attached_window_for_channel(channel_id.as_str())
                    .ok_or_else(|| {
                        ImError::Parse(
                            "im_locate_message requires one attached timeline for channel"
                                .to_string(),
                        )
                    })?;
                let navigation_token = navigation_token
                    .or_else(|| request_id.clone())
                    .unwrap_or_else(|| format!("locate:{target_message_id}"));
                self.state.timeline_state.begin_locate_navigation(
                    channel_id.as_str(),
                    window_token.as_str(),
                    navigation_token.as_str(),
                );
                let state = crate::timeline_navigation::TimelineNavigationState::locate(
                    channel_id,
                    window_token,
                    page_size,
                    request_id,
                    target_message_id,
                    navigation_token,
                );
                self.start_timeline_navigation_http(state, out);
            }
            _ => tracing::debug!("im: query dispatch miss '{}'", name),
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
    }
}
