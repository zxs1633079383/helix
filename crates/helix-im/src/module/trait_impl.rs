use super::*;

#[derive(serde::Deserialize)]
struct RuntimeIdentityPatch {
    auth_user_id: Option<String>,
    company_id: Option<String>,
}

impl Module for ImModule {
    fn name(&self) -> &'static str {
        "helix-im"
    }

    /// 路由判定：IM 模块处理所有 IM Inbound 帧和 IM 命令。
    /// PortReply / Timer 不走此方法（通过 corr_map 定向路由）。
    ///
    /// MV3-G02e 草稿族（`im_save_draft` / `im_query_draft`）在此与下面的 `handle`
    /// **同源放行**（`crate::draft::is_draft_command`）：accepts 与 handle 各写一份命令名
    /// 正是「query 族被 accepts 闸静默丢弃」旧事故的成因，本族不重复该反模式。
    fn accepts(&self, tick: &Tick) -> bool {
        if let Tick::Command(cmd) = tick {
            if cmd.name.as_ref() == crate::presence::COMMAND
                || cmd.name.as_ref() == crate::recent_history::COMMAND
                || crate::draft::is_draft_command(cmd.name.as_ref())
            {
                return true;
            }
        }
        acl::from_tick::accepts_tick(tick)
    }

    /// 处理一个 Tick，零 I/O，零 await。
    fn handle(&mut self, tick: &Tick, now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        let start = out.as_slice().len();
        self.failure_context.enter(tick);
        self.render_scope.observe_reply(tick);
        let result = self.handle_scoped_tick(tick, now_ms, out);
        self.render_scope
            .scope_http_requests(&self.config, tick, start, out);
        let offline = matches!(tick, Tick::PortReply { outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::NetworkUnavailable), .. });
        let service_unreachable = matches!(tick, Tick::PortReply { outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Network | helix_core::tick::PortError::Timeout), .. });
        if offline || service_unreachable {
            let code = if offline { "NETWORK_UNAVAILABLE" } else { "SERVICE_UNREACHABLE" };
            let phase = if offline { "before_io" } else { "after_io" };
            let outcome = if offline { "not_submitted" } else { "unknown" };
            out.visit_mut_from(start, |effect| {
                if let helix_core::Effect::Emit { event } = effect {
                    if let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&event.0) {
                        if let Some(data) = value.get_mut("data").and_then(serde_json::Value::as_object_mut) {
                            data.insert("networkFailure".into(), serde_json::json!({"code":code,"phase":phase,"outcome":outcome,"reqId":self.failure_context.current_request_id()}));
                            if let Ok(bytes) = serde_json::to_vec(&value) { event.0 = bytes.into(); }
                        }
                    }
                }
            });
        }
        // 包括提前 return 和错误路径在内，任何跨端 Emit 都必须经过同一个业务闸门。
        self.render_scope.guard_effects(&self.config, start, out);
        self.failure_context
            .finish(Some(tick), start, out, result, |corr| {
                self.state
                    .corr_map
                    .get(&corr)
                    .and_then(CorrelationContext::diagnostic_request_id)
            })
    }

    /// 启动：加载持久频道归属后才允许其渲染；I/O 仍全部由端口执行。
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        let start = out.as_slice().len();
        let result = self.start_scoped_module(out);
        self.render_scope.guard_effects(&self.config, start, out);
        self.failure_context
            .finish(None, start, out, result, |corr| {
                self.state
                    .corr_map
                    .get(&corr)
                    .and_then(CorrelationContext::diagnostic_request_id)
            })
    }

    /// 停止生命周期也不允许已失效公司投影穿过出口。
    fn on_stop(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        let start = out.as_slice().len();
        self.presence_clear(true, 0, out);
        let result = self.stop_scoped_module(out);
        self.render_scope.guard_effects(&self.config, start, out);
        let result = self
            .failure_context
            .finish(None, start, out, result, |corr| {
                self.state
                    .corr_map
                    .get(&corr)
                    .and_then(CorrelationContext::diagnostic_request_id)
            });
        self.failure_context.clear();
        result
    }
}

impl ImModule {
    /// 执行业务 Tick；最终输出由 Module 实现统一审查。
    fn handle_scoped_tick(
        &mut self,
        tick: &Tick,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), CoreError> {
        if self
            .presence_tick(tick, now_ms, out)
            .map_err(|e| CoreError::ModuleError {
                module: self.name(),
                source: Box::new(e),
            })?
        {
            return Ok(());
        }
        if let Tick::PortReply {
            corr,
            outcome: helix_core::tick::PortOutcome::Ok(reply),
        } = tick
        {
            if matches!(
                self.state.corr_map.get(corr),
                Some(
                    CorrelationContext::DialogListQuery {
                        channel_rows: None,
                        ..
                    } | CorrelationContext::ChannelViewSnapshotQuery { .. }
                        | CorrelationContext::ChannelSyncPage { .. }
                )
            ) {
                self.render_scope.observe_rows(reply.0.as_ref());
            }
        }
        self.diagnostics.now_ms = now_ms;
        if matches!(tick, Tick::Disconnected(_)) {
            self.bookmark_count.invalidate();
            self.presence_clear(false, now_ms, out);
        }
        let effect_start = out.as_slice().len();
        let corr_floor = self.next_corr;
        let recovery_reply = self.observe_sync_reply(tick);
        match tick {
            Tick::Inbound(bytes) => {
                // 入口保留旧容错：坏 JSON inbound 是可丢 WS 噪音，不升级为 CoreError。
                let Ok(frame) = crate::ws::WsFrame::parse(bytes.as_bytes()) else {
                    return Ok(());
                };
                self.dispatch_ws_frame(&frame, now_ms, out)?;
            }

            Tick::Command(cmd) => match cmd.name.as_ref() {
                crate::bookmark_count::COMMAND => {
                    self.bookmark_count_read(cmd.payload.as_ref(), out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                RUNTIME_IDENTITY_COMMAND => {
                    let identity: RuntimeIdentityPatch =
                        serde_json::from_slice(cmd.payload.as_ref()).map_err(|error| {
                            CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(crate::error::ImError::Parse(error.to_string())),
                            }
                        })?;
                    let auth_user_id = identity
                        .auth_user_id
                        .unwrap_or_else(|| self.config.auth_user_id.clone());
                    let company_id = identity
                        .company_id
                        .unwrap_or_else(|| self.config.company_id.clone());
                    if auth_user_id != self.config.auth_user_id {
                        self.render_scope = Default::default();
                    }
                    if auth_user_id != self.config.auth_user_id
                        || company_id != self.config.company_id
                    {
                        self.presence_clear(true, now_ms, out);
                        self.scope_hydration.reset();
                        // 原始 HTTP 读体可能不带频道/公司字段；切换身份时显式结束旧请求，
                        // A→B→A 后的迟到回包也不能重新晋升为当前公司的投影。
                        self.cancel_pending_reads(out);
                        self.finish_sync_observation(
                            crate::sync_observation::SyncResult::Cancelled,
                        );
                        self.cancel_category(out);
                        self.cancel_forward_details(None, "CANCELLED", out);
                        self.cancel_forwards(out)
                            .map_err(|e| CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(e),
                            })?;
                        self.state.reset_message_v3_identity();
                    }
                    self.config.auth_user_id = auth_user_id;
                    self.config.company_id = company_id;
                    // Native may deliver the trusted runtime identity after the WS hello.
                    // The identity reset intentionally invalidates A's recovery window, but a
                    // live B transport must immediately open its own window; otherwise every
                    // startup sync commit is misclassified as ordinary online work and emits
                    // one channel-list event per channel.
                    if self.state.connection_id.is_some()
                        && !self.config.auth_user_id.is_empty()
                        && !self
                            .state
                            .recovery_session
                            .is_active_for(self.config.auth_user_id.as_str())
                    {
                        self.state
                            .recovery_session
                            .begin(self.config.auth_user_id.as_str());
                    }
                }
                name if crate::translation::is_command(name) => {
                    self.translation_command(name, cmd.payload.as_ref(), out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                name if crate::query::forward_detail::is_command(name) => {
                    self.forward_detail_command(name, cmd.payload.as_ref(), out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                "im_send_message" => {
                    self.handle_send_message(cmd.payload.as_ref(), now_ms, out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                "im_retry_upload" => {
                    crate::send::retry_upload::handle_retry_upload(self, cmd.payload.as_ref(), out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                "im_retry_send" => {
                    crate::send::retry_send::handle_retry_send(
                        self,
                        cmd.payload.as_ref(),
                        now_ms,
                        out,
                    )
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?;
                }
                // 连接控制（04 文档表 B `IMWS:reconnect`）：helix-im 是 sans-IO，不持 transport——
                // 只 emit `im:net:reconnect_requested` 控制信号，由 driver 观察后重连 NativeTransport
                // （连接生命周期归 driver）。payload 无须解析（无参控制信号）。
                "im_reconnect" => {
                    out.push(crate::acl::to_effect::emit_reconnect_requested());
                }
                "im_sync_channels" => {
                    crate::sync::explicit::handle(self, cmd.payload.as_ref(), out).map_err(
                        |e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        },
                    )?;
                }
                "im_create_posts" => {
                    crate::forward::start(self, cmd.payload.as_ref(), now_ms, out).map_err(
                        |e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        },
                    )?;
                }
                // MV3-G02e 草稿域：Effect 仅 Persist、零领域事件、结构化 Command Result。
                // 必须排在 outbound / query 两个 guard 之前认领——草稿既不是 HTTP outbound，
                // 也不在 query 闭集内，落到 `_` 分支就会退回「unhandled command」死代码态。
                crate::recent_history::COMMAND => {
                    crate::recent_history::handle(self, cmd.payload.as_ref(), out).map_err(
                        |e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        },
                    )?;
                }
                name if crate::draft::is_draft_command(name) => {
                    crate::draft::handle_command(self, name, cmd.payload.as_ref(), now_ms, out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                // 分类接龙独立authority，不复用文字接龙字段或确认规则。
                name if crate::category_chain::is_command(name) => {
                    self.handle_category_command(name, cmd.payload.as_ref(), now_ms, out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                // 文字接龙命令拥有独立的 correlation/authority barrier，不能落入通用 fire-and-forget。
                name if crate::chain::is_command(name) => {
                    self.handle_chain_command(name, cmd.payload.as_ref(), out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                // Tier 1 outbound 自驱动命令（read/revoke/leave/schedule/cancel/create/makeTopic）：
                // 纯 HTTP-fire，业务契约在 crate::commands（endpoint 锚现网真源）。
                name if crate::commands::is_outbound(name) => {
                    if name == "im_create_schedule" {
                        match self.begin_schedule_media(cmd.payload.as_ref(), now_ms, out) {
                            Ok(true) => return Ok(()),
                            Ok(false) => {}
                            Err(source) => {
                                // 校验失败也必须结束关联等待，不能让界面等到 Host 超时。
                                if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(
                                    cmd.payload.as_ref(),
                                ) {
                                    if let Some(req_id) = payload["req_id"].as_str() {
                                        out.push(crate::read_relay::emit_read_error(
                                            req_id,
                                            "SCHEDULE_CONTENT_INVALID",
                                        ));
                                        return Ok(());
                                    }
                                }
                                return Err(CoreError::ModuleError {
                                    module: self.name(),
                                    source: Box::new(source),
                                });
                            }
                        }
                    }
                    let corr = self.alloc_corr_internal();
                    let topic_request = (name == "im_make_topic")
                        .then(|| make_topic_request_context(cmd.payload.as_ref()))
                        .flatten();
                    let effects = match crate::commands::handle_outbound(
                        name,
                        cmd.payload.as_ref(),
                        self.config.api_base_url.as_str(),
                        // 第二网关 base（spec06 方案 A）：vote/score 命令走它，既有命令忽略。
                        self.config.default_api_base_url.as_str(),
                        self.state.connection_id.as_deref(),
                        corr,
                    ) {
                        Ok(effects) => effects,
                        Err(error) if name == "im_make_topic" => {
                            if let Some((root_message_id, req_id, _)) = topic_request.as_ref() {
                                out.push(crate::acl::to_effect::emit_topic_operation_status(
                                    req_id,
                                    root_message_id,
                                    "engine-rejected",
                                    None,
                                    Some("invalid-request"),
                                    true,
                                ));
                                return Ok(());
                            }
                            return Err(CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(error),
                            });
                        }
                        Err(error) => {
                            return Err(CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(error),
                            });
                        }
                    };
                    // spec06 缺陷A：读族注册回灌上下文（PortReply 透传 `im:read:result{req_id,body}`；写族
                    // fire-and-forget 在下方注册失败终态；quickReply 复用关联回包，只确认 HTTP 结果而非 durable 回应）。S7：byIds 成员快照走 OutboundMembersByIds（额外 emit im:channel:members）。
                    if crate::commands::is_read(name) || name == "im_quick_reply" {
                        let explicit_req_id = crate::read_relay::read_req_id(cmd.payload.as_ref());
                        let increment_hydration = name == "im_channel_load_increment_by_channel_id";
                        if increment_hydration && self.config.auth_user_id.is_empty() {
                            return Err(CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(crate::ImError::Parse(
                                    "increment hydration requires RuntimeAuth account and tenant"
                                        .to_string(),
                                )),
                            });
                        }
                        // 读族只有明确 correlation 才允许回灌；hydration 兼容旧 driver 的同时，
                        // 由显式 transport req_id 决定是否发布 sender-only channelIncrement。
                        let emit_channel_increment =
                            increment_hydration && explicit_req_id.is_some();
                        let req_id = match explicit_req_id {
                            Some(req_id) => req_id,
                            None if increment_hydration => {
                                format!("increment-hydration-{}", corr.raw())
                            }
                            None => {
                                // 无 correlation 的旧读请求仍执行 HTTP，但回包不得进入任何 UI 投影。
                                for effect in effects {
                                    out.push(effect);
                                }
                                return Ok(());
                            }
                        };
                        {
                            let ctx = if increment_hydration {
                                CorrelationContext::OutboundIncrementHydration {
                                    req_id,
                                    emit_channel_increment,
                                    scope_channel: None,
                                }
                            } else if name == "im_channels_members_by_ids" {
                                CorrelationContext::OutboundMembersByIds { req_id }
                            } else if name == "im_user_candidates" {
                                CorrelationContext::OutboundContactCandidates { req_id }
                            } else if name == "im_get_posts_after_index" {
                                // G11h initial-window 独立于 locate/exact：HTTP 成功必须先过 durable read-back。
                                let args = serde_json::from_slice::<serde_json::Value>(
                                    cmd.payload.as_ref(),
                                )
                                .map_err(|error| {
                                    CoreError::ModuleError {
                                        module: self.name(),
                                        source: Box::new(crate::ImError::Parse(error.to_string())),
                                    }
                                })?;
                                if let Some(request) =
                                    crate::outbound::posts::read::initial_window_request(
                                        &args, name,
                                    )
                                    .map_err(|error| {
                                        CoreError::ModuleError {
                                            module: self.name(),
                                            source: Box::new(error),
                                        }
                                    })?
                                {
                                    CorrelationContext::OutboundInitialWindow {
                                        req_id: request.req_id,
                                        post_id: request.post_id,
                                        page_size: request.page_size,
                                    }
                                } else {
                                    let channel_id = args
                                        .get("channel_id")
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_owned);
                                    CorrelationContext::OutboundReadReply {
                                        req_id,
                                        command: name.to_string(),
                                        channel_id,
                                    }
                                }
                            } else if name == "im_get_posts" {
                                // G11g exact 查询独立于旧 locate 终态；locate 只保留兼容入参，不改变按 id 读回。
                                let requested_ids = serde_json::from_slice::<serde_json::Value>(
                                    cmd.payload.as_ref(),
                                )
                                .ok()
                                .and_then(|value| {
                                    crate::outbound::posts::read::exact_post_ids(&value, name).ok()
                                })
                                .unwrap_or_default();
                                CorrelationContext::OutboundExactPosts {
                                    req_id,
                                    requested_ids,
                                }
                            } else if name == "im_get_replies" || name == "im_get_reply_branch" {
                                // G12 回复族冻结请求语义，PortReply 只生成 typed thread event。
                                let request =
                                    crate::render_ready_replies::ReplyProjectionRequest::from_command(
                                        name,
                                        cmd.payload.as_ref(),
                                        corr.raw(),
                                        self.config.auth_user_id.as_str(),
                                    )
                                    .ok_or_else(|| CoreError::ModuleError {
                                        module: self.name(),
                                        source: Box::new(crate::ImError::Parse(
                                            "reply command requires a valid req_id and payload"
                                                .to_string(),
                                        )),
                                    })?;
                                if request.mode
                                    == crate::render_ready_replies::ReplyProjectionMode::Snapshot
                                    && !request.root_hint.is_empty()
                                {
                                    self.state
                                        .reply_projection_revisions
                                        .insert(request.root_hint.clone(), request.revision);
                                }
                                CorrelationContext::OutboundReplies { request }
                            } else if name == "im_post_read_list" {
                                CorrelationContext::OutboundPostReaders { req_id }
                            } else if name == "im_channel_load_post_pinned" {
                                let channel_id = crate::query::pinned_projection::parse_channel_id(
                                    cmd.payload.as_ref(),
                                )
                                .map_err(|error| CoreError::ModuleError {
                                    module: self.name(),
                                    source: Box::new(error),
                                })?;
                                let account_id = self.config.auth_user_id.clone();
                                let projection_key =
                                    crate::query::pinned_projection::projection_key(
                                        account_id.as_str(),
                                        channel_id,
                                    )
                                    .map_err(|error| {
                                        CoreError::ModuleError {
                                            module: self.name(),
                                            source: Box::new(error),
                                        }
                                    })?;
                                let epoch = self
                                    .state
                                    .pinned_projection_epochs
                                    .get(&channel_id)
                                    .copied()
                                    .unwrap_or(0);
                                CorrelationContext::OutboundPinnedReply {
                                    req_id,
                                    account_id,
                                    channel_id,
                                    projection_key,
                                    epoch,
                                }
                            } else {
                                let channel_id = serde_json::from_slice::<serde_json::Value>(
                                    cmd.payload.as_ref(),
                                )
                                .ok()
                                .and_then(|value| {
                                    value
                                        .get("channel_id")
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_owned)
                                });
                                CorrelationContext::OutboundReadReply {
                                    req_id,
                                    command: name.to_string(),
                                    channel_id,
                                }
                            };
                            self.state.corr_map.insert(corr, ctx);
                            if matches!(name, "im_bookmark_create" | "im_bookmark_delete") {
                                self.bookmark_count.write_started();
                            }
                        }
                    } else if name == "im_create_schedule" {
                        if let Ok(args) =
                            serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                        {
                            if let Some(channel_id) = args
                                .get("channel_id")
                                .and_then(serde_json::Value::as_str)
                                .and_then(ChannelId::from_str)
                            {
                                let request_id = args
                                    .get("req_id")
                                    .and_then(serde_json::Value::as_str)
                                    .filter(|value| !value.is_empty())
                                    .map(str::to_string);
                                if let Some(request_id) = request_id.as_ref() {
                                    self.state
                                        .pending_schedule_requests
                                        .insert(channel_id, request_id.clone());
                                }
                                self.state.corr_map.insert(
                                    corr,
                                    CorrelationContext::OutboundScheduleCreate {
                                        channel_id,
                                        request_id,
                                    },
                                );
                            }
                        }
                    } else if name == "im_cancel_schedule" {
                        if let Ok(args) =
                            serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                        {
                            if let Some(channel_id) = args
                                .get("channel_id")
                                .and_then(serde_json::Value::as_str)
                                .and_then(ChannelId::from_str)
                            {
                                let request_id = args
                                    .get("req_id")
                                    .and_then(serde_json::Value::as_str)
                                    .filter(|value| !value.is_empty())
                                    .map(str::to_string);
                                if let Some(request_id) = request_id.as_ref() {
                                    self.state
                                        .pending_schedule_cancel_requests
                                        .insert(channel_id, request_id.clone());
                                }
                                self.state.corr_map.insert(
                                    corr,
                                    CorrelationContext::OutboundScheduleCancel {
                                        channel_id,
                                        request_id,
                                    },
                                );
                            }
                        }
                    } else if name == "im_create_channel" {
                        if let Ok(args) =
                            serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                        {
                            let request_id = args
                                .get("req_id")
                                .and_then(serde_json::Value::as_str)
                                .filter(|value| !value.is_empty())
                                .map(str::to_string);
                            if let Some(members) = args
                                .get("user_ids")
                                .and_then(serde_json::Value::as_array)
                                .map(|user_ids| {
                                    user_ids
                                        .iter()
                                        .filter_map(serde_json::Value::as_str)
                                        .map(|user_id| serde_json::json!({ "id": user_id }))
                                        .collect::<Vec<_>>()
                                })
                                .filter(|members| !members.is_empty())
                            {
                                self.state.corr_map.insert(
                                    corr,
                                    CorrelationContext::OutboundChannelCreate {
                                        members,
                                        request_id,
                                    },
                                );
                            }
                        }
                    } else if name == "im_make_topic" {
                        if let Some((root_message_id, req_id, display_name)) = topic_request {
                            self.state.corr_map.insert(
                                corr,
                                CorrelationContext::OutboundMakeTopic {
                                    root_message_id: root_message_id.clone(),
                                    req_id: req_id.clone(),
                                    display_name,
                                },
                            );
                            out.push(crate::acl::to_effect::emit_topic_operation_status(
                                &req_id,
                                &root_message_id,
                                "remote-pending",
                                None,
                                None,
                                false,
                            ));
                        }
                    } else if name == "im_channel_change_info"
                        || name == "im_channel_change_notice"
                        || name == "im_channel_change_display_name"
                        || name == "im_channel_change_orient"
                        || name == "im_channel_change_permission"
                    {
                        if let Ok(value) =
                            serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                        {
                            if let Some(channel_id) = value
                                .get("channel_id")
                                .and_then(|id| id.as_str())
                                .and_then(ChannelId::from_str)
                            {
                                let causation_id = value
                                    .get("req_id")
                                    .and_then(serde_json::Value::as_str)
                                    .filter(|value| !value.is_empty())
                                    .map(str::to_string);
                                self.state.corr_map.insert(
                                    corr,
                                    CorrelationContext::OutboundChannelSettings {
                                        channel_id,
                                        causation_id,
                                    },
                                );
                            }
                        }
                    } else if name == "im_channel_leave" {
                        if let Some(channel_id) =
                            serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                                .ok()
                                .and_then(|value| {
                                    value
                                        .get("channel_id")
                                        .and_then(|id| id.as_str())
                                        .map(str::to_string)
                                })
                                .and_then(|id| ChannelId::from_str(&id))
                        {
                            self.state
                                .corr_map
                                .insert(corr, CorrelationContext::OutboundLeave { channel_id });
                        }
                    }
                    // 写请求同样由业务关联表结算一次；诊断记录不得复活已取消请求。
                    if !crate::commands::is_read(name) && !self.state.corr_map.contains_key(&corr) {
                        if let Some(req_id) = crate::read_relay::read_req_id(cmd.payload.as_ref()) {
                            self.state.corr_map.insert(
                                corr,
                                CorrelationContext::OutboundWriteReply { req_id },
                            );
                        }
                    }
                    for eff in effects {
                        out.push(eff);
                    }
                }
                // 投影/状态命令：最近消息由 helix-im 统一编排 local-first/远端 fallback。
                name if crate::query::is_query(name) => {
                    self.handle_query_command(name, cmd.payload.as_ref(), out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                _ => tracing::debug!("im: unhandled command '{}'", cmd.name),
            },

            Tick::PortReply { corr, outcome } => {
                let result = self.handle_port_reply(*corr, outcome, now_ms, out);
                if recovery_reply && result.is_err() {
                    self.finish_sync_observation(crate::sync_observation::SyncResult::Failed);
                }
                result.map_err(|e| CoreError::ModuleError {
                    module: self.name(),
                    source: Box::new(e),
                })?;
            }

            Tick::PortProgress { corr, progress } => {
                self.handle_file_upload_progress(*corr, *progress, out)
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?;
            }

            Tick::Timer(id) => {
                if self
                    .translation_timeout(*id, out)
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?
                {
                    return Ok(());
                }
                if self.forward_detail_timeout(*id, out) {
                    return Ok(());
                }
                if self
                    .forward_timeout(*id, out)
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?
                {
                    return Ok(());
                }
                if self
                    .handle_category_timeout(*id, out)
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?
                {
                    return Ok(());
                }
                if self.schedule_media_timeout(*id, out) {
                    return Ok(());
                }
                if self.schedule_multipart_timeout(*id, now_ms, out) {
                    return Ok(());
                }
                if let Some(key) = self.state.schedule_multipart_retry_timers.remove(id) {
                    self.schedule_multipart_retry(&key, out).map_err(|error| {
                        CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(error),
                        }
                    })?;
                    return Ok(());
                }
                if let Some(pending) = self.state.multipart_retry_timers.remove(id) {
                    self.emit_multipart_init(
                        pending.channel_id,
                        pending.temporary_id,
                        pending.target,
                        pending.input,
                        out,
                    )
                    .map_err(|error| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(error),
                    })?;
                    return Ok(());
                }
                self.handle_timer(*id, now_ms, out)
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?;
            }

            Tick::Connected(_transport) => {
                // A4：连接建立 ≠ 可用。现网 Go 服务端连上后**单向下发 hello 帧**；
                // 必须等收到 hello（→ Connected + connectionId）才能 resync——
                // 否则握手前发 sync/notify 会被 Go 401/403（缺 connectionId 身份头）。
                // 故此处只置 Connecting，resync 推迟到 hello Inbound 分支（handle_hello）。
                // Native reader 与 activation tick 来自两个异步源；hello 可能先入队。
                // connectionId 是服务端握手的权威凭据，晚到的 transport tick 不得把
                // 已握手状态从 Connected 降回 Connecting。
                if self.state.connection_id.is_none() {
                    self.state.conn = ConnState::Connecting;
                }
                // Do not publish a runtime transition here: the server hello
                // establishes the recovery epoch, and PersistOk is the only
                // completion boundary for a V2 frame.
                self.state.recovery_session.phase = crate::sync_session::RecoveryPhase::Comparing;
                tracing::debug!("helix-im: transport connected, awaiting hello handshake");
            }

            Tick::Disconnected(_transport) => {
                self.cancel_hydration_requests("hydration disconnected", out);
                self.finish_sync_observation(crate::sync_observation::SyncResult::Cancelled);
                self.state.increment_pull = None;
                self.state.corr_map.retain(|_, context| {
                    !matches!(
                        context,
                        CorrelationContext::IncrementPullHttp
                            | CorrelationContext::IncrementPullPersist
                            | CorrelationContext::OutboundIncrementHydration {
                                scope_channel: Some(_),
                                ..
                            }
                    )
                });
                self.state.conn = ConnState::Disconnected;
                self.state.recovery_session.phase = crate::sync_session::RecoveryPhase::Failed;
                self.state.reset_transport_query_session();
                // 清除所有 channel 的 inflight_sync（连接断开，所有在途 Http 不会回报）
                // 若不清除，重连后 proactive resync 会被 B1 守卫全部跳过
                for ch in self.state.channels.values_mut() {
                    ch.inflight_sync = None;
                    ch.last_sync_from_seq = None;
                }
                // 断线清理（语义保真 = 旧 corr_to_sync.clear() + continuation_pending.clear()）：
                //   1. 清除 SyncPull（在途 sync Http corr，断线后不会再收到 PortReply）。
                //   2. ChannelPersist 本体**保留**、仅把 wants_continuation 置 false——
                //      对应的 Effect::Persist 是本地 SQLite 写（driver spawn_blocking 执行），
                //      其 PortReply 与 WS 断线物理无关，断线后极可能照常完成并回报，
                //      留着可继续推进 cursor（HX-C008 cursor 单调推进）；只是不再续拉 sync。
                //      旧代码 continuation_pending.clear() 仅清续拉意图、故意不清 corr_to_channel
                //      本体（module.rs 路由1 仍能推 cursor），此处逐字节保真。
                //   3. OptimisticSend / ScanCursors / ScanChannelProjections 不动（本地回报与连接无关）。
                for ctx in self.state.corr_map.values_mut() {
                    if let CorrelationContext::ChannelPersist {
                        wants_continuation, ..
                    } = ctx
                    {
                        *wants_continuation = false;
                    }
                }
                self.state.corr_map.retain(|_, ctx| {
                    !matches!(
                        ctx,
                        CorrelationContext::SyncPull { .. }
                            | CorrelationContext::SyncPullBatch { .. }
                    )
                });
                // 在途HTTP关联已撤销，释放其全局窗口及旧队列；hello会重新建立恢复意图。
                self.state.sync_scheduler.reset();
                tracing::debug!("helix-im: disconnected, cleared inflight sync state");
            }
        }
        self.observe_sync_tick(&out.as_slice()[effect_start..], corr_floor);
        Ok(())
    }

    /// 启动：先 Scan `channel_event_cursor` 与 `channel` 投影，过滤终态后才触发 proactive sync。
    fn start_scoped_module(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        self.diagnose(crate::diagnostics::Observation {
            event: "login_sync_started",
            stage: "store",
            result: "started",
            ..Default::default()
        });
        self.start_lifecycle(out)
            .map_err(|e| CoreError::ModuleError {
                module: self.name(),
                source: Box::new(e),
            })?;
        self.state.media_recovery_ready = false;
        self.state.pending_media_recovery_compensation = None;
        let corr = self.alloc_corr_internal();
        out.push(helix_core::Effect::Persist {
            corr,
            ops: vec![helix_core::effect::StorageOp::Scan(
                helix_core::effect::ScanSpec {
                    table: "pending_media",
                    limit: None,
                    filter: None,
                    order_by: &[],
                },
            )],
        });
        self.state.pending_media_rehydrate_corr = Some(corr);
        let schedule_corr = self.alloc_corr_internal();
        out.push(helix_core::Effect::Persist {
            corr: schedule_corr,
            ops: vec![helix_core::effect::StorageOp::Scan(
                helix_core::effect::ScanSpec {
                    table: "schedule_media_intent",
                    limit: None,
                    filter: None,
                    order_by: &[],
                },
            )],
        });
        self.state.schedule_media_rehydrate_corr = Some(schedule_corr);
        Ok(())
    }

    /// 账号切换与停止共用读请求终态；先删除 corr，迟到回包不可重新发布。
    fn cancel_pending_reads(&mut self, out: &mut EffectSink) {
        // 接龙消息补全没有 UI req_id；停止/切换身份时撤销私有读取及连续槽位，迟到回包无副作用。
        let pending_chain_posts = &mut self.state.pending_chain_post_event_ids;
        let channels = &mut self.state.channels;
        self.state.corr_map.retain(|_, context| {
            if let CorrelationContext::CanonicalChainPostHydration { event } = context {
                pending_chain_posts.remove(
                    &crate::ws::handlers::channel_stream_event::chain_post_event_key(event),
                );
                if let Some(channel) = channels.get_mut(&event.channel_id) {
                    if channel.pending_stream_seq == Some(event.seq) {
                        channel.pending_stream_seq = None;
                    }
                }
                false
            } else {
                true
            }
        });
        self.cancel_bookmark_count(out);
        // Lifecycle cleanup suppresses terminal events, so this branch cannot fail on event
        // serialization; retain the helper's fallible API for user initiated cancellation.
        let _ = self.cancel_translations(None, false, out);
        let mut cancelled_reads: Vec<_> = self
            .state
            .corr_map
            .iter()
            .filter_map(|(corr, context)| match context {
                CorrelationContext::ReactionUnreadProjection { req_id, .. }
                | CorrelationContext::OutboundReadReply { req_id, .. }
                | CorrelationContext::OutboundMembersByIds { req_id }
                | CorrelationContext::OutboundContactCandidates { req_id } => {
                    Some((*corr, req_id.clone()))
                }
                _ => None,
            })
            .collect();
        cancelled_reads.sort_unstable_by_key(|(corr, _)| corr.raw());
        for (corr, req_id) in cancelled_reads {
            self.state.corr_map.remove(&corr);
            if req_id != "bookmark-count-calibration" {
                out.push(crate::read_relay::emit_read_error(&req_id, "CANCELLED"));
            }
        }
    }

    /// 停止时释放读关联、发送与频道计时器。
    fn stop_scoped_module(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        self.clear_all_multipart_runtime();
        // HashMap 的 drain 顺序不稳定；停止时按 TimerId 排序，保证同一输入产生确定性的
        // CancelTimer Effect 序列，避免回放/跨端证据因迭代顺序漂移。
        let mut multipart_retry_timers: Vec<_> = self
            .state
            .multipart_retry_timers
            .drain()
            .map(|(timer, _)| timer)
            .collect();
        multipart_retry_timers.sort_unstable_by_key(|timer| timer.raw());
        for timer in multipart_retry_timers {
            out.push(helix_core::Effect::CancelTimer { id: timer });
        }
        self.state.multipart_retry_attempts.clear();
        self.cancel_pending_reads(out);
        self.finish_sync_observation(crate::sync_observation::SyncResult::Cancelled);
        self.cancel_category(out);
        self.stop_schedule_media(out);
        self.stop_schedule_multipart(out);
        self.cancel_forward_details(None, "CANCELLED", out);
        self.cancel_forwards(out)
            .map_err(|e| CoreError::ModuleError {
                module: self.name(),
                source: Box::new(e),
            })?;
        self.diagnose(crate::diagnostics::Observation {
            event: "runtime_stopped",
            result: "interrupted",
            reason: "runtime_stopped",
            ..Default::default()
        });
        self.stop_lifecycle(out)
            .map_err(|e| CoreError::ModuleError {
                module: self.name(),
                source: Box::new(e),
            })
    }
}

/// 冻结 make-topic 的 root、request 与用户确认标题，供异步 authority/persist/readback 串联。
fn make_topic_request_context(payload: &[u8]) -> Option<(String, String, String)> {
    let value = serde_json::from_slice::<serde_json::Value>(payload).ok()?;
    let root_message_id = value
        .get("root_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let req_id = value
        .get("req_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())?
        .to_string();
    let display_name = value
        .get("display_name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("话题")
        .to_string();
    Some((root_message_id, req_id, display_name))
}
