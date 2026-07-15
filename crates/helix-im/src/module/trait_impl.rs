use super::*;

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
                    let effects = crate::commands::handle_outbound(
                        name,
                        cmd.payload.as_ref(),
                        self.config.api_base_url.as_str(),
                        // 第二网关 base（spec06 方案 A）：vote/score 命令走它，既有命令忽略。
                        self.config.default_api_base_url.as_str(),
                        self.state.connection_id.as_deref(),
                        corr,
                    )
                    .map_err(|e| CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    })?;
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
                                // S8：回复族 → 双 emit（im:read:result 冻结 + im:channel:replies render-ready）。
                                let request = crate::render_ready_replies::ReplyProjectionRequest::from_command(
                                    name,
                                    cmd.payload.as_ref(),
                                    corr.raw(),
                                )
                                .expect("reply command with req_id must build projection request");
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
                                CorrelationContext::OutboundLocatePost { req_id }
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
                    } else if name == "im_create_channel" {
                        if let Ok(args) =
                            serde_json::from_slice::<serde_json::Value>(cmd.payload.as_ref())
                        {
                            if let Ok((members, _)) =
                                crate::outbound::registry::build_creator_member_users(&args, name)
                            {
                                self.state.corr_map.insert(
                                    corr,
                                    CorrelationContext::OutboundChannelCreate { members },
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
                self.state.conn = ConnState::Connecting;
                tracing::debug!("helix-im: transport connected, awaiting hello handshake");
            }

            Tick::Disconnected(_transport) => {
                self.state.conn = ConnState::Disconnected;
                self.state.reset_recent_query_coverage();
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
            })
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
