use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, ConnState, CorrelationContext, Seq};
use helix_core::effect::{ScanSpec, StorageOp, TimerId};
use helix_core::{Effect, EffectSink};

impl ImModule {
    pub(crate) fn channel_cursors(&self) -> Vec<(ChannelId, Seq)> {
        let mut cursors: Vec<(ChannelId, Seq)> = self
            .state
            .channels
            .iter()
            .map(|(&id, ch)| (id, ch.cursor.value()))
            .collect();
        cursors.sort_unstable_by_key(|(id, _)| *id);
        cursors
    }

    /// 处理 Timer 触发
    pub(crate) fn handle_timer(
        &mut self,
        timer_id: TimerId,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 1. ping timer（self-arm 周期心跳）
        if self.state.ping_timer == Some(timer_id) {
            if self.state.conn == ConnState::Connected {
                // ping WS 帧带单调 seq（Go websocket_router 拒 seq≤0）+ UC-4.4 心跳 gap 补偿
                // piggyback（cursors/allHash）。cursor 快照确定性升序（HX-C010），allHash 在 ping_frame 内算。
                let seq = self.state.next_client_seq();
                let roots = crate::sync_increment::heartbeat_root_cursors(&self.state);
                out.push(crate::acl::to_effect::ping_frame(seq, &roots));
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
            // 不发 hello 帧：现网 Go 是服务端单向下发 hello，客户端不发（抽象 wire 残留，
            // 且 seq=0 会被 Go 拒）。重连应由 driver/host transport 层 connect（§7/A2 Effect::Connect
            // 待定）；此处仅置 Connecting，等真重连后服务端再下发 hello 走 handle_hello。
            return Ok(());
        }

        // 3. gate timer（per-channel，1s 缺口后触发 sync/notify）
        //    找出哪个 channel 持有这个 gate timer
        let maybe_channel = self
            .state
            .channels
            .iter()
            .find(|(_, ch)| ch.gate.as_ref().map_or(false, |g| g.timer_id == timer_id))
            .map(|(id, _)| *id);

        if let Some(channel_id) = maybe_channel {
            // 防风暴（根治配套）：若该 channel 已有在途 sync（大缺口立即 backfill 已发起，
            // 见 ws/handlers/gate.rs::trigger_backfill_if_large_gap），1s gate timer **不再重复发**
            // sync/notify——仅清 gate（在途 sync 回报会 commit 追平 cursor + flush buffer；若追平后
            // 仍有缺口，后续帧/续拉会重新 arm gate）。避免双发 sync + corr_map/窗口计数漂移。
            if self
                .state
                .channels
                .get(&channel_id)
                .is_some_and(|ch| ch.inflight_sync.is_some())
            {
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.gate = None;
                }
                return Ok(());
            }
            // gate 超时：吐 Http{sync/notify}
            let from_seq = self.state.channels[&channel_id].cursor.value();
            let sync_corr = self.alloc_corr_internal();
            self.state
                .corr_map
                .insert(sync_corr, CorrelationContext::SyncPull { channel_id });

            // 清除 gate（已触发）+ 设置 inflight（B1 守卫）
            if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                ch.gate = None;
                ch.inflight_sync = Some(crate::state::InflightSync(sync_corr));
            }

            out.push(crate::acl::to_effect::sync_notify(
                &self.config.api_base_url,
                channel_id,
                from_seq,
                sync_corr,
                self.state.connection_id.as_deref(),
            ));
            return Ok(());
        }

        // 4. 15s 发送超时 timer
        let maybe_tmp = self
            .state
            .pending_sends
            .iter()
            .find(|(_, ps)| ps.timeout_timer == timer_id)
            .map(|(id, _)| id.clone());

        if let Some(tmp_id) = maybe_tmp {
            let failed_channel_id = self
                .state
                .pending_sends
                .get(&tmp_id)
                .filter(|pending| {
                    pending.status != crate::state::SendStatus::Sent
                        && pending.status != crate::state::SendStatus::UnSend
                        && !pending.upload_failed
                })
                .and_then(|pending| pending.body.as_ref())
                .and_then(|body| body.get("channelId"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if let Some(ps) = self.state.pending_sends.get_mut(&tmp_id) {
                ps.on_timeout(out);
            }
            if let Some(channel_id) = failed_channel_id {
                out.push(crate::acl::to_effect::emit_post_send_failed(
                    &channel_id,
                    tmp_id.0.as_str(),
                ));
            }
            return Ok(());
        }

        let _ = now_ms;
        Ok(())
    }

    /// A4 + B-rest：处理 hello 握手帧——存 connectionId + 置 Connected + emit established
    /// + 触发 increment HTTP（取代直接 resync）。
    ///
    /// 现网时序：transport Connected（→ Connecting）→ 服务端单向下发 hello（此处）
    /// → 握手完成可用。connectionId 存入 state，A3b 盖章进后续 IM HTTP 身份头。
    ///
    /// B-rest 扭转（设计文档 §3 Phase 2）：hello 后**不再直接 emit_proactive_resync**，
    /// 改为触发 increment HTTP（push 模型，带已知 channel 游标）+ emit im:channels:loaded。
    /// 真增量走 WS `increment_channel` 单播（→ registry handler），global-end 收尾时才
    /// 对增量群发 proactive sync。会话边界 reset increment 批次集（§7 跨会话残留）。
    #[allow(dead_code)]
    pub(crate) fn handle_hello(&mut self, connection_id: String, out: &mut EffectSink) {
        self.state.conn = ConnState::Connected;
        self.state.connection_id = Some(connection_id.clone());
        // §7：每个 hello 窗口起点清空上一会话残留的 increment 批次集（防 sync 风暴）。
        self.state.reset_increment_batch();
        out.push(crate::acl::to_effect::emit_connection_established(
            &connection_id,
        ));

        // B-rest：触发 increment HTTP（带已知 channel 游标）。HTTP 仅 ack 无数据。
        let conn_id = self.state.connection_id.clone();
        let cursors = self.channel_cursors();
        self.state.pending_increment_bootstrap_after_scan = cursors.is_empty();
        let inc_corr = self.alloc_corr_internal();
        out.push(crate::acl::to_effect::increment_http_trigger(
            &self.config.api_base_url,
            &cursors,
            inc_corr,
            conn_id.as_deref(),
        ));
        out.push(crate::acl::to_effect::emit_channels_loaded());
        tracing::info!("helix-im: hello handshake complete, increment HTTP triggered");
    }

    /// 向所有已知 channel 发 proactive sync Http（scan_reply 路径复用）。
    pub(crate) fn emit_proactive_resync(&mut self, out: &mut EffectSink) {
        // NS-1：channels 是 HashMap，迭代序逐进程随机（SipHash 随机种子）。本路径逐 channel
        // 单调 alloc_corr + push 有序 Http Effect，故必须确定性排序锚定可回放（GOAL §1 确定性，
        // ChannelId: Ord）。冷路径（resync）排序零热路径代价。
        let mut all: Vec<ChannelId> = self.state.channels.keys().copied().collect();
        all.sort_unstable();
        self.emit_proactive_resync_for(&all, out);
    }

    /// 向指定 channel 集发 proactive sync Http（B-rest global-end 用增量群子集；
    /// scan_reply 用全集）。逐 channel 走既有 gate/peek-commit 链路。
    pub(crate) fn emit_proactive_resync_for(
        &mut self,
        targets: &[ChannelId],
        out: &mut EffectSink,
    ) {
        let api_base_url = self.config.api_base_url.clone();
        let auth_user_id = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc_corr| {
            let mut ctx =
                crate::ws::ImWsContext::new(state, 0, &api_base_url, &auth_user_id, alloc_corr);
            crate::ws::handlers::increment_channel_end::emit_proactive_resync_for(
                &mut ctx, targets, out,
            );
        });
    }

    /// 启动：Scan `channel_event_cursor` 载入 cursor，注册已知 channel 后触发 proactive sync。
    pub(crate) fn start_lifecycle(&mut self, out: &mut EffectSink) -> Result<(), ImError> {
        // 1. 吐 Persist{corr, Scan(channel_event_cursor)} 载入全部 cursor
        // 首次启动表为空；driver scan no-table 降级为 Err，由 handle_scan_reply 处理。
        let scan_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: scan_corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "channel_event_cursor",
                limit: None,
                filter: None,
                order_desc_col: None,
            })],
        });
        self.state
            .corr_map
            .insert(scan_corr, CorrelationContext::ScanCursors);

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
    pub(crate) fn stop_lifecycle(&mut self, out: &mut EffectSink) -> Result<(), ImError> {
        if let Some(ping_timer) = self.state.ping_timer.take() {
            out.push(Effect::CancelTimer { id: ping_timer });
        }
        if let Some(backoff_timer) = self.state.backoff_timer.take() {
            out.push(Effect::CancelTimer { id: backoff_timer });
        }
        // Cancel all channel gate timers
        let gate_timers: Vec<TimerId> = self
            .state
            .channels
            .values()
            .filter_map(|ch| ch.gate.as_ref().map(|g| g.timer_id))
            .collect();
        for tid in gate_timers {
            out.push(Effect::CancelTimer { id: tid });
        }
        // Cancel all pending send timeout timers
        let send_timers: Vec<TimerId> = self
            .state
            .pending_sends
            .values()
            .map(|ps| ps.timeout_timer)
            .collect();
        for tid in send_timers {
            out.push(Effect::CancelTimer { id: tid });
        }
        self.state.conn = ConnState::Closing;
        Ok(())
    }
}
