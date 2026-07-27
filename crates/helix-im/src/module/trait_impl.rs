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
    fn accepts(&self, tick: &Tick) -> bool {
        acl::from_tick::accepts_tick(tick)
    }

    /// 处理一个 Tick，零 I/O，零 await。
    fn handle(&mut self, tick: &Tick, now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        match tick {
            Tick::Inbound(bytes) => {
                // 入口保留旧容错：坏 JSON inbound 是可丢 WS 噪音，不升级为 CoreError。
                let Ok(frame) = crate::ws::WsFrame::parse(bytes.as_bytes()) else {
                    return Ok(());
                };
                self.dispatch_ws_frame(&frame, now_ms, out)?;
            }

            Tick::Command(cmd) => match cmd.name.as_ref() {
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
                    if auth_user_id != self.config.auth_user_id
                        || company_id != self.config.company_id
                    {
                        self.state.reset_surface_runtime_for_identity_change();
                    }
                    self.config.auth_user_id = auth_user_id;
                    self.config.company_id = company_id;
                    // Native may deliver the trusted runtime identity after the WS hello.
                    // The identity reset intentionally invalidates A's recovery window, but a
                    // live B transport must immediately open its own window; otherwise every
                    // startup sync commit is misclassified as ordinary online work and emits
                    // one dialog Frame per channel.
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
                crate::client_api::INTERACTION_ACTION_INVOKE_COMMAND => {
                    self.handle_action_invocation(cmd.payload.as_ref(), now_ms, out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                crate::surface_runtime::SURFACE_RESYNC_COMMAND => {
                    self.handle_surface_resync(cmd.payload.as_ref(), now_ms, out)
                        .map_err(|e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })?;
                }
                crate::interaction_frame::FRAME_APPLIED_COMMAND => {
                    let ack =
                        crate::interaction_frame::FrameApplied::from_payload(cmd.payload.as_ref())
                            .map_err(|error| CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(crate::error::ImError::Parse(error.to_string())),
                            })?;
                    tracing::info!(
                        hop = "interaction_frame.applied",
                        corr_key = %ack.transaction_id,
                        transaction_id = %ack.transaction_id,
                        applied_slots = ack.applied_slots,
                        result = ?ack.result,
                        "renderer committed InteractionFrame V2"
                    );
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
                "im_create_posts" if crate::forward::is_source_id_request(cmd.payload.as_ref()) => {
                    crate::forward::start(self, cmd.payload.as_ref(), now_ms, out).map_err(
                        |e| CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        },
                    )?;
                }
                // Tier 1 outbound 自驱动命令（read/revoke/leave/schedule/cancel/create/makeTopic）：
                // 纯 HTTP-fire，业务契约在 crate::commands（endpoint 锚现网真源）。
                name if crate::commands::is_outbound(name) => {
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
                    // fire-and-forget 不注册）。S7：byIds 成员快照走 OutboundMembersByIds（额外 emit im:channel:members）。
                    if crate::commands::is_read(name) {
                        if let Some(req_id) = crate::read_relay::read_req_id(cmd.payload.as_ref()) {
                            let ctx = if name == "im_channel_load_increment_by_channel_id" {
                                CorrelationContext::OutboundIncrementHydration { req_id }
                            } else if name == "im_channels_members_by_ids" {
                                CorrelationContext::OutboundMembersByIds { req_id }
                            } else if name == "im_user_candidates" {
                                CorrelationContext::OutboundContactCandidates { req_id }
                            } else if name == "im_get_replies" || name == "im_get_reply_branch" {
                                // G12 回复族冻结请求语义，PortReply 只生成 typed thread Frame。
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
                            } else if name == "im_get_posts"
                                && serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                                    .ok()
                                    .and_then(|value| {
                                        value.get("locate").and_then(|flag| flag.as_bool())
                                    })
                                    .unwrap_or(false)
                            {
                                let post_id = serde_json::from_slice::<serde_json::Value>(
                                    cmd.payload.as_ref(),
                                )
                                .ok()
                                .and_then(|value| {
                                    value
                                        .get("post_ids")
                                        .and_then(serde_json::Value::as_array)
                                        .cloned()
                                })
                                .and_then(|ids| {
                                    ids.into_iter()
                                        .find_map(|id| id.as_str().map(str::to_owned))
                                })
                                .unwrap_or_default();
                                CorrelationContext::OutboundLocatePost { req_id, post_id }
                            } else if name == "im_post_read_list" {
                                CorrelationContext::OutboundPostReaders { req_id }
                            } else {
                                CorrelationContext::OutboundReadReply { req_id }
                            };
                            self.state.corr_map.insert(corr, ctx);
                        }
                    } else if name == "im_create_posts" {
                        if let Some(req_id) = crate::read_relay::read_req_id(cmd.payload.as_ref()) {
                            self.state
                                .corr_map
                                .insert(corr, CorrelationContext::OutboundCreatePosts { req_id });
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
                self.handle_port_reply(*corr, outcome, now_ms, out)
                    .map_err(|e| CoreError::ModuleError {
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
                //   3. OptimisticSend / ScanCursors 不动（旧代码同样不清）。
                for ctx in self.state.corr_map.values_mut() {
                    if let CorrelationContext::ChannelPersist {
                        wants_continuation, ..
                    } = ctx
                    {
                        *wants_continuation = false;
                    }
                }
                self.state
                    .corr_map
                    .retain(|_, ctx| !matches!(ctx, CorrelationContext::SyncPull { .. }));
                tracing::debug!("helix-im: disconnected, cleared inflight sync state");
            }
        }
        Ok(())
    }

    /// 启动：Scan `channel_event_cursor` 载入 cursor，注册已知 channel 后触发 proactive sync。
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
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
        Ok(())
    }

    /// 停止：取消所有 timer + 关闭连接
    fn on_stop(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
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
