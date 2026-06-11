//! ImModule：实现 helix-core Module trait 的 IM 业务模块
//!
//! ## 职责
//!
//! - 实现 `Module` trait（4 方法：name / accepts / handle / on_start+on_stop）
//! - 持有 ImState（纯数据，无 I/O）
//! - 通过 ACL-1 翻译 IM 概念 ↔ core Effect/Tick
//!
//! ## 不变量（关键）
//!
//! - `handle` 严格同步，零 await，零 I/O，零 spawn
//! - 所有副作用通过 EffectSink 输出，由 driver 异步兑现

use helix_core::{Module, Tick, EffectSink, CoreError, Effect};
use helix_core::effect::{TimerId, StorageOp, ScanSpec, UpsertSpec, SqlValue, HttpRequest};
use crate::state::{ImState, ConnState, ChannelId, Seq, TemporaryId, SendStatus};
use crate::channel::Channel;
use crate::pending_send::PendingSend;
use crate::acl;
use crate::error::ImError;

/// IM 业务模块配置
pub struct ImConfig {
    /// WS endpoint（如 "wss://api.example.com/ws"）
    pub ws_url: String,
    /// HTTP API base URL（如 "https://api.example.com"）
    pub api_base_url: String,
    /// 心跳间隔（毫秒，默认 8000）
    pub ping_interval_ms: u64,
    /// 发送超时（毫秒，默认 15000）
    pub send_timeout_ms: u64,
}

impl Default for ImConfig {
    fn default() -> Self {
        Self {
            ws_url: String::new(),
            api_base_url: String::new(),
            ping_interval_ms: 8_000,
            send_timeout_ms: 15_000,
        }
    }
}

/// IM 业务模块（实现 helix_core::Module trait）
pub struct ImModule {
    pub(crate) config: ImConfig,
    pub(crate) state: ImState,
    /// Correlation 分配器（自增）
    ///
    /// BLK-1b：模块从 1 开始，避免与 engine 的 Correlation 冲突。
    /// 多模块场景由各模块从不同起点分配；更健壮的方案是 core 统一 IdSource。
    next_corr: u64,
    /// TimerId 分配器（自增）
    ///
    /// BLK-1b：从 100 开始，避免与 engine 内部潜在分配冲突。
    next_timer: u64,
}

impl ImModule {
    pub fn new(config: ImConfig) -> Self {
        Self {
            config,
            state: ImState::new(),
            next_corr: 1,
            next_timer: 100,
        }
    }

    pub fn alloc_corr(&mut self) -> helix_core::Correlation {
        let c = helix_core::Correlation::from_raw(self.next_corr);
        self.next_corr += 1;
        c
    }

    pub fn alloc_timer(&mut self) -> TimerId {
        let t = TimerId::from_raw(self.next_timer);
        self.next_timer += 1;
        t
    }

    /// 测试 / driver 初始化用：预注册一个 channel（初始 cursor 值由调用方提供）
    pub fn register_channel(&mut self, id: ChannelId, initial_cursor: u64) {
        self.state.channels.insert(id, Channel::new(id, initial_cursor));
    }

    /// 处理 PortReply（Persist/Http 的异步结果回报）
    ///
    /// ## MAJ-1 修复
    ///
    /// 原实现在 `state.pending_commits` 中查找，但 `channel.pending_commits` 才是
    /// cursor 推进的聚合根。修复：
    /// 1. `corr_to_channel`（Correlation → ChannelId 路由索引）
    /// 2. 查到 channel_id 后委托 `channel.on_persist_ok(corr, fx)` 推进 cursor
    /// 3. 15s 超时 timer → 通过 `corr_to_send` 找 PendingSend
    fn handle_port_reply(
        &mut self,
        corr: helix_core::Correlation,
        outcome: &helix_core::tick::PortOutcome,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

        // 路由 1：channel sync Persist 回报（cursor 推进）
        if let Some(channel_id) = self.state.corr_to_channel.remove(&corr) {
            match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.on_persist_ok(corr, out)?;
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = channel_id.0,
                        corr = corr.raw(),
                        error = ?e,
                        "persist failed, cursor not advanced, will re-sync"
                    );
                }
            }
            return Ok(());
        }

        // 路由 2：发送 Persist 回报（P1 乐观落库）
        if let Some(tmp_id) = self.state.corr_to_send.remove(&corr) {
            match outcome {
                PortOutcome::Ok(_) => {
                    // P1 落库成功：将 PendingSend 状态推进到 Sending
                    if let Some(ps) = self.state.pending_sends.get_mut(&tmp_id) {
                        ps.status = SendStatus::Sending;
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        tmp_id = ?tmp_id,
                        error = ?e,
                        "P1 optimistic persist failed"
                    );
                }
            }
            return Ok(());
        }

        // 路由 3：sync Http 回报（sync/notify 响应）
        if let Some(channel_id) = self.state.corr_to_sync.remove(&corr) {
            match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_sync_reply(corr, channel_id, reply, out)?;
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = channel_id.0,
                        corr = corr.raw(),
                        error = ?e,
                        "sync/notify failed"
                    );
                }
            }
            return Ok(());
        }

        Ok(())
    }

    /// 处理 sync/notify HTTP 响应
    fn handle_sync_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use crate::sync_session::SyncResponse;
        use crate::parser::parse_sync_response;

        let response = parse_sync_response(reply.0.as_ref())
            .map_err(|e| ImError::Parse(format!("sync response parse error: {}", e)))?;

        match response {
            SyncResponse::NoChange => {
                // cursor 已是最新，无事可做
            }

            SyncResponse::Events { events, needs_continuation: _ } => {
                if events.is_empty() {
                    return Ok(());
                }
                // peek/commit 三阶段：peek（只读预览，不推 cursor）
                // 为整批事件分配一个 Persist corr（先分配，避免借用冲突）
                let commit_target = events.iter().map(|e| e.seq).max().unwrap_or(Seq(0));
                let persist_corr = self.alloc_corr_internal();
                let persist_ops = crate::acl::to_effect::batch_upsert_events(&events);
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.insert(persist_corr, commit_target);
                }
                out.push(Effect::Persist {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                self.state.corr_to_channel.insert(persist_corr, channel_id);
            }

            SyncResponse::TooLong { reset_to } => {
                // too_long：清内存 buffer + cursor 复位到 reset_to-1 + Emit
                let reset_cursor = Seq(reset_to.0.saturating_sub(1));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.buffer.clear();
                    ch.gate = None;
                    ch.cursor = crate::state::Cursor::new(reset_cursor);
                }
                out.push(crate::acl::to_effect::emit_sync_too_long(channel_id));
            }

            SyncResponse::Snapshot(snap) => {
                // Snapshot：批量 upsert + cursor 跳至 reset_to（先分配，避免借用冲突）
                let persist_corr = self.alloc_corr_internal();
                let reset_to = snap.reset_to;
                let persist_ops = crate::acl::to_effect::batch_upsert_events(&snap.messages);
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.buffer.clear();
                    ch.gate = None;
                    ch.pending_commits.insert(persist_corr, reset_to);
                }
                out.push(Effect::Persist {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                self.state.corr_to_channel.insert(persist_corr, channel_id);
            }
        }

        let _ = corr;
        Ok(())
    }

    /// 内部 alloc_corr（不借用 self.state，仅借用分配器）
    fn alloc_corr_internal(&mut self) -> helix_core::Correlation {
        let c = helix_core::Correlation::from_raw(self.next_corr);
        self.next_corr += 1;
        c
    }

    /// 处理 Timer 触发
    fn handle_timer(
        &mut self,
        timer_id: TimerId,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 1. ping timer（self-arm 周期心跳）
        if self.state.ping_timer == Some(timer_id) {
            if self.state.conn == ConnState::Connected {
                // 吐 ping WS 帧（简化实现：用 Send{空帧}）
                out.push(Effect::Send {
                    transport: helix_core::effect::TransportId::from_raw(0),
                    frame: bytes::Bytes::from_static(b"{\"action\":\"ping\"}"),
                });
            }
            // 重新 arm（self-arm 实现无 tick() 的周期心跳）
            let new_timer = self.alloc_timer();
            out.push(Effect::ScheduleTimer {
                id: new_timer,
                after_ms: self.config.ping_interval_ms,
            });
            self.state.ping_timer = Some(new_timer);
            return Ok(());
        }

        // 2. backoff timer（重连到期）
        if self.state.backoff_timer == Some(timer_id) {
            self.state.backoff_timer = None;
            self.state.conn = ConnState::Connecting;
            // 吐 connect 意图（hello 帧）
            out.push(Effect::Send {
                transport: helix_core::effect::TransportId::from_raw(0),
                frame: bytes::Bytes::from_static(b"{\"action\":\"hello\"}"),
            });
            return Ok(());
        }

        // 3. gate timer（per-channel，1s 缺口后触发 sync/notify）
        //    找出哪个 channel 持有这个 gate timer
        let maybe_channel = self.state.channels.iter()
            .find(|(_, ch)| ch.gate.as_ref().map_or(false, |g| g.timer_id == timer_id))
            .map(|(id, _)| *id);

        if let Some(channel_id) = maybe_channel {
            // gate 超时：吐 Http{sync/notify}
            let from_seq = self.state.channels[&channel_id].cursor.value();
            let sync_corr = self.alloc_corr_internal();
            self.state.corr_to_sync.insert(sync_corr, channel_id);

            // 清除 gate（已触发）
            if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                ch.gate = None;
            }

            out.push(crate::acl::to_effect::sync_notify(
                &self.config.api_base_url,
                channel_id,
                from_seq,
                sync_corr,
            ));
            return Ok(());
        }

        // 4. 15s 发送超时 timer
        let maybe_tmp = self.state.pending_sends.iter()
            .find(|(_, ps)| ps.timeout_timer == timer_id)
            .map(|(id, _)| id.clone());

        if let Some(tmp_id) = maybe_tmp {
            if let Some(ps) = self.state.pending_sends.get_mut(&tmp_id) {
                ps.on_timeout(out);
            }
            return Ok(());
        }

        let _ = now_ms;
        Ok(())
    }

    /// 处理 im_send_message 命令（§4.5 发送链路）
    ///
    /// 吐四个 Effect：
    /// 1. Persist{P1, 乐观落库 Sending}
    /// 2. Http{H1, POST /posts/create}
    /// 3. ScheduleTimer{T1, 15s}
    /// 4. Emit(im:post:sending)
    fn handle_send_message(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 解析命令载荷
        let cmd: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| ImError::Parse(format!("im_send_message payload: {}", e)))?;

        let channel_id = cmd["channel_id"].as_u64()
            .ok_or_else(|| ImError::Parse("missing channel_id".to_string()))?;
        let tmp_id_str = cmd["temporary_id"].as_str()
            .ok_or_else(|| ImError::Parse("missing temporary_id".to_string()))?
            .to_string();
        let text = cmd["text"].as_str().unwrap_or("").to_string();

        let channel_id = ChannelId(channel_id);
        let tmp_id = TemporaryId(tmp_id_str.clone());

        // 分配 corr / timer
        let p1_corr = self.alloc_corr_internal();
        let h1_corr = self.alloc_corr_internal();
        let t1_timer = self.alloc_timer();

        // 1. Persist{P1, 乐观落库 status=Sending}
        out.push(Effect::Persist {
            corr: p1_corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![vec![
                    ("temporary_id".to_string(), SqlValue::Text(tmp_id_str.clone())),
                    ("channel_id".to_string(), SqlValue::Integer(channel_id.0 as i64)),
                    ("message".to_string(), SqlValue::Text(text.clone())),
                    ("send_status".to_string(), SqlValue::Text("sending".to_string())),
                ]],
                conflict_key: Some("temporary_id"),
            })],
        });

        // 2. Http{H1, POST /posts/create}
        let body = serde_json::json!({
            "channel_id": channel_id.0,
            "temporary_id": tmp_id_str,
            "message": text,
        });
        out.push(Effect::Http {
            corr: h1_corr,
            req: HttpRequest {
                method: "POST".to_string(),
                url: format!("{}/posts/create", self.config.api_base_url),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: Some(bytes::Bytes::from(serde_json::to_vec(&body).unwrap())),
            },
        });

        // 3. ScheduleTimer{T1, 15s}
        out.push(Effect::ScheduleTimer {
            id: t1_timer,
            after_ms: self.config.send_timeout_ms,
        });

        // 4. Emit(im:post:sending)
        out.push(crate::acl::to_effect::emit_post_sending(channel_id, &tmp_id_str));

        // 注册 PendingSend（等待 echo 对账）
        let mut ps = PendingSend::new(tmp_id.clone(), t1_timer);
        ps.status = SendStatus::Local;
        self.state.pending_sends.insert(tmp_id, ps);
        self.state.corr_to_send.insert(p1_corr, TemporaryId(tmp_id_str));

        Ok(())
    }

    /// 处理 echo 入站帧（WS 回声，§4.5 对账）
    ///
    /// echo 帧含 temporary_id → 按 tmp_id 找 PendingSend → reconcile
    /// Phase B 中 echo 解析路径尚未在 ingest 中调用（parser 需完整 echo 字段支持），
    /// 函数已实现但等待后续调用点接入。
    #[allow(dead_code)]
    fn handle_echo(
        &mut self,
        channel_id: ChannelId,
        seq: crate::state::Seq,
        tmp_id: &str,
        server_id: crate::state::ServerId,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let tmp_id = TemporaryId(tmp_id.to_string());
        // 先分配 corr，避免后续双重可变借用
        let reconcile_corr = self.alloc_corr_internal();
        if let Some(ps) = self.state.pending_sends.get_mut(&tmp_id) {
            ps.reconcile(server_id, reconcile_corr, out);
        }
        // 推进 cursor（独立借用，不与 pending_sends 冲突）
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            let _ = ch.cursor.try_advance(seq);
            out.push(crate::acl::to_effect::advance_cursor(channel_id, seq));
        }
        Ok(())
    }
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
    fn handle(
        &mut self,
        tick: &Tick,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), CoreError> {
        match tick {
            Tick::Inbound(_) => {
                // ACL-1 解析入站帧
                match acl::from_tick::parse_inbound_tick(tick) {
                    Ok(ev) => {
                        if let Some(ch) = self.state.channels.get_mut(&ev.channel_id) {
                            ch.ingest(ev, out, now_ms)
                                .map_err(|e| CoreError::ModuleError {
                                    module: self.name(),
                                    source: Box::new(e),
                                })?;
                        }
                        // 未注册 channel 的帧：静默忽略（driver 路由问题，不是 IM 错误）
                    }
                    Err(ImError::Parse(msg)) => {
                        // 解析失败：静默 drop（不让 IM wire 错误崩溃 engine）
                        tracing::debug!("im: inbound parse error: {}", msg);
                    }
                    Err(e) => return Err(CoreError::ModuleError {
                        module: self.name(),
                        source: Box::new(e),
                    }),
                }
            }

            Tick::Command(cmd) => {
                match cmd.name.as_ref() {
                    "im_send_message" => {
                        self.handle_send_message(cmd.payload.as_ref(), out)
                            .map_err(|e| CoreError::ModuleError {
                                module: self.name(),
                                source: Box::new(e),
                            })?;
                    }
                    _ => {
                        tracing::debug!("im: unhandled command '{}'", cmd.name);
                    }
                }
            }

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
                // 连接建立 → proactive resync：对所有 channel 发 sync/notify（§4.8）
                self.state.conn = ConnState::Connected;

                // 先收集 channel_id + from_seq，避免持有 channels 不可变借用时
                // 再借用 self.next_corr（alloc_corr_internal）和 self.state.corr_to_sync
                let channel_cursors: Vec<(ChannelId, Seq)> = self.state.channels.iter()
                    .map(|(&id, ch)| (id, ch.cursor.value()))
                    .collect();

                let channel_count = channel_cursors.len();
                for (channel_id, from_seq) in channel_cursors {
                    let sync_corr = self.alloc_corr_internal();
                    self.state.corr_to_sync.insert(sync_corr, channel_id);
                    out.push(crate::acl::to_effect::sync_notify(
                        &self.config.api_base_url,
                        channel_id,
                        from_seq,
                        sync_corr,
                    ));
                }

                tracing::info!("helix-im: connected, triggered proactive resync for {} channels",
                    channel_count);
            }

            Tick::Disconnected(_transport) => {
                self.state.conn = ConnState::Disconnected;
                // 重连退避是 driver 的责任；模块只感知断线状态
                tracing::debug!("helix-im: disconnected");
            }
        }
        Ok(())
    }

    /// 启动：读取 cursor + arm ping timer + 吐 Scan（§4.8 proactive resync 所需）
    ///
    /// Scan 吐 Persist{corr, ops:[Scan(channel_event_cursor)]}，
    /// PortReply 回来后驱动会把全部 cursor 行打包进 ReplyBytes，
    /// 模块在 handle_port_reply 里扫 channels 补 sync（§4.8）。
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        // 1. 吐 Persist{corr, Scan(channel_event_cursor)} 载入全部 cursor
        let scan_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: scan_corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "channel_event_cursor",
                limit: None,
            })],
        });
        self.state.scan_corr = Some(scan_corr);

        // 2. arm ping timer
        let ping_timer = self.alloc_timer();
        out.push(Effect::ScheduleTimer {
            id: ping_timer,
            after_ms: self.config.ping_interval_ms,
        });
        self.state.ping_timer = Some(ping_timer);
        self.state.conn = ConnState::Connecting;

        tracing::info!("helix-im: starting, ws={}", self.config.ws_url);
        Ok(())
    }

    /// 停止：取消所有 timer + 关闭连接
    fn on_stop(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        if let Some(ping_timer) = self.state.ping_timer.take() {
            out.push(Effect::CancelTimer { id: ping_timer });
        }
        if let Some(backoff_timer) = self.state.backoff_timer.take() {
            out.push(Effect::CancelTimer { id: backoff_timer });
        }
        // Cancel all channel gate timers
        let gate_timers: Vec<TimerId> = self.state.channels.values()
            .filter_map(|ch| ch.gate.as_ref().map(|g| g.timer_id))
            .collect();
        for tid in gate_timers {
            out.push(Effect::CancelTimer { id: tid });
        }
        // Cancel all pending send timeout timers
        let send_timers: Vec<TimerId> = self.state.pending_sends.values()
            .map(|ps| ps.timeout_timer)
            .collect();
        for tid in send_timers {
            out.push(Effect::CancelTimer { id: tid });
        }
        self.state.conn = ConnState::Closing;
        Ok(())
    }
}
