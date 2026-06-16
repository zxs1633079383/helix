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

use crate::acl;
use crate::channel::Channel;
use crate::error::ImError;
use crate::pending_send::PendingSend;
use crate::state::{ChannelId, ConnState, ImState, SendStatus, Seq, TemporaryId};
use helix_core::effect::{HttpRequest, ScanSpec, SqlValue, StorageOp, TimerId, UpsertSpec};
use helix_core::{CoreError, Effect, EffectSink, Module, Tick};

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
        self.state
            .channels
            .insert(id, Channel::new(id, initial_cursor));
    }

    /// 查询 per-channel cursor（测试 / 诊断用）
    pub fn cursor_for(&self, channel_id: ChannelId) -> Option<u64> {
        self.state
            .channels
            .get(&channel_id)
            .map(|ch| ch.cursor.value().0)
    }

    /// 查询 pending_sends 数量（测试用）
    pub fn pending_send_count(&self) -> usize {
        self.state.pending_sends.len()
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
    ///
    /// ## E4 修复
    ///
    /// 新增 scan_corr 分支：on_start 发出的 Scan{_helix_monotonic} PortReply
    /// 在此解析 rows（scope_key TEXT + value INTEGER）→ register_channel + 批量发 proactive sync Http。
    fn handle_port_reply(
        &mut self,
        corr: helix_core::Correlation,
        outcome: &helix_core::tick::PortOutcome,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

        // 路由 0（E4）：on_start Scan 回报 → 加载 cursor + 触发 proactive resync
        if self.state.scan_corr == Some(corr) {
            self.state.scan_corr = None;
            match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_scan_reply(reply, out)?;
                }
                PortOutcome::Err(e) => {
                    // 降级：scan 失败（包括首次启动表不存在 / no such table）不崩，空 channel 集仍可用
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?e,
                        "scan _helix_monotonic failed (likely first run, no cursors yet), starting with empty channel set"
                    );
                }
            }
            return Ok(());
        }

        // 路由 1：channel sync Persist 回报（cursor 推进）
        if let Some(channel_id) = self.state.corr_to_channel.remove(&corr) {
            // 同时检查 continuation_pending（E3 续拉意图）
            let wants_continuation = self.state.continuation_pending.remove(&corr).is_some();
            match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.on_persist_ok(corr, out)?;
                    }
                    // E3：续拉检查（cursor 已推进后才发，确保 fromSeq 严格递增）
                    if wants_continuation {
                        self.maybe_continue_sync(channel_id, out);
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        channel_id = channel_id.0,
                        corr = corr.raw(),
                        error = ?e,
                        "persist failed, cursor not advanced, will re-sync"
                    );
                    // persist 失败：清除 last_sync_from_seq 避免遗留脏状态
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.last_sync_from_seq = None;
                    }
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
            // B1 gate_inflight 清除：sync 回报到达，清除在途标记
            if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                ch.inflight_sync = None;
            }
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

    /// E4：处理 on_start Scan{_helix_monotonic} 的 PortReply
    ///
    /// `_helix_monotonic` 是 advance_cursor 写入的实际表（scope_key TEXT, value INTEGER）。
    /// engine_loop 将 rows 序列化为 JSON 数组 `[{"scope_key": "<channel_id>", "value": N}, ...]`。
    ///
    /// scope_key 是 channel_id.to_string()（IM 业务约定，见 acl/to_effect.rs::advance_cursor）。
    /// 反序列化时把 TEXT scope_key parse 回 u64；value 即 last event seq。
    fn handle_scan_reply(
        &mut self,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // engine_loop 约定：rows → JSON 数组 [{"col": val}, ...]
        // _helix_monotonic 表：scope_key TEXT（channel_id 字符串）, value INTEGER（cursor）
        let rows: Vec<serde_json::Value> = if reply.0.is_empty() {
            vec![]
        } else {
            serde_json::from_slice(reply.0.as_ref()).unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    raw = %String::from_utf8_lossy(reply.0.as_ref()),
                    "scan reply JSON parse failed, treating as empty rows"
                );
                vec![]
            })
        };

        for row in &rows {
            // scope_key 是 TEXT（channel_id.to_string()），需要从字符串解析回 u64
            let channel_id = match row["scope_key"].as_str() {
                Some(s) => match s.parse::<u64>() {
                    Ok(v) if v > 0 => v,
                    Ok(_) | Err(_) => {
                        tracing::warn!(
                            scope_key = s,
                            "scan row scope_key not a valid channel_id u64, skipping"
                        );
                        continue;
                    }
                },
                None => {
                    tracing::warn!(row = ?row, "scan row missing scope_key, skipping");
                    continue;
                }
            };
            let cursor = match row["value"].as_i64() {
                Some(v) if v >= 0 => v as u64,
                _ => 0u64,
            };
            // register_channel 幂等：若已存在则不覆盖（Connected 路径可能已注册）
            self.state
                .channels
                .entry(ChannelId(channel_id))
                .or_insert_with(|| crate::channel::Channel::new(ChannelId(channel_id), cursor));
        }

        tracing::info!(
            channel_count = rows.len(),
            "scan_corr resolved, loaded {} channels from cursor store",
            rows.len()
        );

        // 仅在已连接时触发 proactive resync（复用 Connected 路径逻辑）
        // 若此时尚未连接，Connected 事件到来后会自行 resync
        if self.state.conn == crate::state::ConnState::Connected {
            self.emit_proactive_resync(out);
        }

        Ok(())
    }

    /// A4：处理 hello 握手帧——存 connectionId + 置 Connected + emit 信号 + resync。
    ///
    /// 现网时序：transport Connected（→ Connecting）→ 服务端单向下发 hello（此处）
    /// → 握手完成可用。connectionId 存入 state，A3b 盖章进后续 IM HTTP 身份头。
    /// resync 推迟到此刻（cursor 已由 on_start 的 Scan 载入），不在 Connected 时发。
    fn handle_hello(&mut self, connection_id: String, out: &mut EffectSink) {
        self.state.conn = ConnState::Connected;
        self.state.connection_id = Some(connection_id.clone());
        out.push(crate::acl::to_effect::emit_connection_established(
            &connection_id,
        ));
        self.emit_proactive_resync(out);
        tracing::info!("helix-im: hello handshake complete, connection established");
    }

    /// 向所有已知 channel 发 proactive sync Http（复用逻辑，hello 握手和 scan_reply 共用）
    fn emit_proactive_resync(&mut self, out: &mut EffectSink) {
        // A3b：connectionId 身份头（clone 一次，避免后续 &mut self.state 借用冲突）。
        let conn_id = self.state.connection_id.clone();
        let channel_cursors: Vec<(ChannelId, Seq)> = self
            .state
            .channels
            .iter()
            .map(|(&id, ch)| (id, ch.cursor.value()))
            .collect();
        for (channel_id, from_seq) in channel_cursors {
            // B1 gate_inflight 守卫：跳过已有在途 sync 的 channel
            if self
                .state
                .channels
                .get(&channel_id)
                .map_or(false, |ch| ch.inflight_sync.is_some())
            {
                continue;
            }
            let sync_corr = self.alloc_corr_internal();
            self.state.corr_to_sync.insert(sync_corr, channel_id);
            if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                ch.inflight_sync = Some(crate::state::InflightSync(sync_corr));
            }
            out.push(crate::acl::to_effect::sync_notify(
                &self.config.api_base_url,
                channel_id,
                from_seq,
                sync_corr,
                conn_id.as_deref(),
            ));
        }
    }

    /// E3 续拉：Persist 落库成功后，若上批 sync 标记了 needs_continuation，
    /// 以推进后的 cursor（新 fromSeq）发出下一批 sync Http。
    ///
    /// 终止保证（双重）：
    /// 1. fromSeq（= cursor after commit）必须严格 > last_sync_from_seq（上次发出续拉时的 fromSeq）
    ///    这保证每次续拉都在推进位置；若 cursor 未前进，说明所有事件均是幂等 dup → 停止
    /// 2. B1 gate_inflight 守卫：同 channel 同时只允许一个在途 sync
    fn maybe_continue_sync(&mut self, channel_id: ChannelId, out: &mut EffectSink) {
        // A3b：connectionId 身份头（clone 一次，避免后续 &mut self.state 借用冲突）。
        let conn_id = self.state.connection_id.clone();
        // 读取 cursor 和 last_sync_from_seq（不可变借用）
        let (cursor_seq, prev_from_seq, has_inflight) = match self.state.channels.get(&channel_id) {
            Some(ch) => (
                ch.cursor.value(),
                ch.last_sync_from_seq,
                ch.inflight_sync.is_some(),
            ),
            None => return,
        };

        // B1：若已有在途 sync（意外情况），跳过
        if has_inflight {
            tracing::warn!(
                channel_id = channel_id.0,
                "maybe_continue_sync: inflight_sync already set, skipping continuation"
            );
            return;
        }

        // 终止保证：新 fromSeq（= cursor after commit）必须严格 > prev_from_seq
        // 若相等，说明服务端返回的事件均低于上次 fromSeq（全部幂等丢弃，cursor 未前进）
        if let Some(prev_from) = prev_from_seq {
            if cursor_seq.0 <= prev_from.0 {
                tracing::warn!(
                    channel_id = channel_id.0,
                    cursor = cursor_seq.0,
                    prev_from_seq = prev_from.0,
                    "continuation aborted: cursor did not advance past prev sync fromSeq (all events were dup-drops)"
                );
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.last_sync_from_seq = None;
                }
                return;
            }
        }

        // 记录本次续拉的 fromSeq（下一批检查时用）
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            ch.last_sync_from_seq = Some(cursor_seq);
        }

        let sync_corr = self.alloc_corr_internal();
        self.state.corr_to_sync.insert(sync_corr, channel_id);
        if let Some(ch) = self.state.channels.get_mut(&channel_id) {
            ch.inflight_sync = Some(crate::state::InflightSync(sync_corr));
        }
        out.push(crate::acl::to_effect::sync_notify(
            &self.config.api_base_url,
            channel_id,
            cursor_seq,
            sync_corr,
            conn_id.as_deref(),
        ));
        tracing::debug!(
            channel_id = channel_id.0,
            from_seq = cursor_seq.0,
            "E3: continuation sync Http emitted"
        );
    }

    /// 处理 sync/notify HTTP 响应
    ///
    /// ## E3 修复
    ///
    /// `needs_continuation=true` 时，Persist 落库后（on_persist_ok 路径），
    /// 此函数还需记录「续拉意图」——通过在 channel 上设置 `last_sync_max_seq`。
    /// 实际续拉在 `on_persist_ok` → `maybe_continue_sync` 中触发，确保 cursor 已推进。
    /// 终止条件：events 为空但 needs_continuation=true → 停止 + log warn（防服务端 bug）。
    fn handle_sync_reply(
        &mut self,
        corr: helix_core::Correlation,
        channel_id: ChannelId,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use crate::parser::parse_sync_response;
        use crate::sync_session::SyncResponse;

        let response = parse_sync_response(reply.0.as_ref())
            .map_err(|e| ImError::Parse(format!("sync response parse error: {}", e)))?;

        match response {
            SyncResponse::NoChange => {
                // cursor 已是最新，无事可做
            }

            SyncResponse::Events {
                events,
                needs_continuation,
            } => {
                if events.is_empty() {
                    if needs_continuation {
                        // 终止保证：服务端返回空 events 但仍声称需续拉 → 防无限循环
                        tracing::warn!(
                            channel_id = channel_id.0,
                            "needs_continuation=true but events is empty, stopping continuation to prevent infinite loop"
                        );
                    }
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
                // E3：在 corr_to_channel 同时记录是否需要续拉
                // 通过 state.continuation_pending 标记（此 corr 落库成功后续拉）
                if needs_continuation {
                    self.state
                        .continuation_pending
                        .insert(persist_corr, channel_id);
                }
            }

            SyncResponse::TooLong { reset_to } => {
                // too_long：清内存 buffer + cursor 复位到 reset_to-1 + Emit
                let reset_cursor = Seq(reset_to.0.saturating_sub(1));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.buffer.clear();
                    ch.gate = None;
                    ch.cursor = crate::state::Cursor::new(reset_cursor);
                    ch.last_sync_from_seq = None;
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
                    ch.last_sync_from_seq = None;
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
        let maybe_channel = self
            .state
            .channels
            .iter()
            .find(|(_, ch)| ch.gate.as_ref().map_or(false, |g| g.timer_id == timer_id))
            .map(|(id, _)| *id);

        if let Some(channel_id) = maybe_channel {
            // gate 超时：吐 Http{sync/notify}
            let from_seq = self.state.channels[&channel_id].cursor.value();
            let sync_corr = self.alloc_corr_internal();
            self.state.corr_to_sync.insert(sync_corr, channel_id);

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
    fn handle_send_message(&mut self, payload: &[u8], out: &mut EffectSink) -> Result<(), ImError> {
        // 解析命令载荷
        let cmd: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| ImError::Parse(format!("im_send_message payload: {}", e)))?;

        let channel_id = cmd["channel_id"]
            .as_u64()
            .ok_or_else(|| ImError::Parse("missing channel_id".to_string()))?;
        let tmp_id_str = cmd["temporary_id"]
            .as_str()
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
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(tmp_id_str.clone()),
                    ),
                    (
                        "channel_id".to_string(),
                        SqlValue::Integer(channel_id.0 as i64),
                    ),
                    ("message".to_string(), SqlValue::Text(text.clone())),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("sending".to_string()),
                    ),
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
                headers: {
                    // A3b：Content-Type + connectionId 身份头（hello 后 Some）
                    let mut h = vec![("Content-Type".to_string(), "application/json".to_string())];
                    h.extend(crate::acl::to_effect::connection_id_headers(
                        self.state.connection_id.as_deref(),
                    ));
                    h
                },
                body: Some(bytes::Bytes::from(serde_json::to_vec(&body).unwrap())),
            },
        });

        // 3. ScheduleTimer{T1, 15s}
        out.push(Effect::ScheduleTimer {
            id: t1_timer,
            after_ms: self.config.send_timeout_ms,
        });

        // 4. Emit(im:post:sending)
        out.push(crate::acl::to_effect::emit_post_sending(
            channel_id,
            &tmp_id_str,
        ));

        // 注册 PendingSend（等待 echo 对账）
        let mut ps = PendingSend::new(tmp_id.clone(), t1_timer);
        ps.status = SendStatus::Local;
        self.state.pending_sends.insert(tmp_id, ps);
        self.state
            .corr_to_send
            .insert(p1_corr, TemporaryId(tmp_id_str));

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
    fn handle(&mut self, tick: &Tick, now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        match tick {
            Tick::Inbound(bytes) => {
                // A4：先 peek 是否 hello 握手帧（现网 Go 连上后单向下发，无需回 ack）。
                if let Some(connection_id) = crate::parser::parse_hello(bytes.as_bytes()) {
                    self.handle_hello(connection_id, out);
                    return Ok(());
                }
                // 非 hello → 现有事件路径（ACL-1 解析入站帧）
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
                    Err(e) => {
                        return Err(CoreError::ModuleError {
                            module: self.name(),
                            source: Box::new(e),
                        })
                    }
                }
            }

            Tick::Command(cmd) => match cmd.name.as_ref() {
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
                // 清除所有 channel 的 inflight_sync（连接断开，所有在途 Http 不会回报）
                // 若不清除，重连后 proactive resync 会被 B1 守卫全部跳过
                for ch in self.state.channels.values_mut() {
                    ch.inflight_sync = None;
                    ch.last_sync_from_seq = None;
                }
                // 清除 corr_to_sync（在途 sync Http corr，断线后不会再收到 PortReply）
                self.state.corr_to_sync.clear();
                self.state.continuation_pending.clear();
                tracing::debug!("helix-im: disconnected, cleared inflight sync state");
            }
        }
        Ok(())
    }

    /// 启动：读取 cursor + arm ping timer + 吐 Scan（§4.8 proactive resync 所需）
    ///
    /// Scan 吐 Persist{corr, ops:[Scan(_helix_monotonic)]}。
    /// _helix_monotonic 是 advance_cursor（acl/to_effect.rs）写入的实际 cursor 表，
    /// 列：scope_key TEXT PRIMARY KEY（= channel_id.to_string()）, value INTEGER（= cursor seq）。
    /// PortReply 回来后 handle_scan_reply 解析 scope_key→u64 + value，注册 channel，
    /// 然后对全部已知 channel 发 proactive sync Http（§4.8）。
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        // 1. 吐 Persist{corr, Scan(_helix_monotonic)} 载入全部 cursor
        // 注意：此表由 monotonic_upsert 懒建，首次启动（无任何 cursor 写过）时为空或不存在。
        // driver scan 对 no such table 应降级返回 PortOutcome::Err，handle_scan_reply 处理降级。
        let scan_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: scan_corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "_helix_monotonic",
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

// ─── A4 hello 握手回放测试（in-crate：直接断言 pub(crate) state）────────────
#[cfg(test)]
mod handshake_tests {
    use super::*;
    use helix_core::effect::TransportId;
    use helix_core::tick::InboundBytes;

    fn module_with_channel() -> ImModule {
        let mut m = ImModule::new(ImConfig::default());
        m.register_channel(ChannelId(1), 0);
        m
    }

    fn hello_frame(conn_id: &str) -> Tick {
        let body = serde_json::json!({
            "action": "hello",
            "data": { "connectionId": conn_id, "serverVersion": "x", "pingInterval": 30000u64 }
        });
        Tick::Inbound(InboundBytes(serde_json::to_vec(&body).unwrap().into()))
    }

    fn has_http(sink: &EffectSink) -> bool {
        sink.as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Http { .. }))
    }

    /// Connected → conn 变 Connecting，且不产 resync Http（现网 Go 握手前 resync 会 401）。
    #[test]
    fn connected_sets_connecting_no_resync() {
        let mut m = module_with_channel();
        let mut sink = EffectSink::new();
        m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
            .unwrap();
        assert_eq!(m.state.conn, ConnState::Connecting);
        assert!(!has_http(&sink), "握手前不得 resync");
    }

    /// hello Inbound → Connected + emit im:connection:established + resync Http + connectionId 存。
    #[test]
    fn hello_completes_handshake_and_resyncs() {
        let mut m = module_with_channel();
        let mut sink = EffectSink::new();
        m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
            .unwrap();
        sink.clear();
        m.handle(&hello_frame("CONN_ABC"), 1001, &mut sink).unwrap();
        assert_eq!(m.state.conn, ConnState::Connected);
        assert_eq!(m.state.connection_id.as_deref(), Some("CONN_ABC"));
        assert!(
            sink.as_slice()
                .iter()
                .any(|e| matches!(e, Effect::Emit { .. })),
            "应 emit im:connection:established"
        );
        assert!(has_http(&sink), "hello 后应 resync");
    }

    /// hello 后构造的 sync Http headers 含 ("connectionId", id)。
    #[test]
    fn sync_http_carries_connection_id_header() {
        let mut m = module_with_channel();
        let mut sink = EffectSink::new();
        m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
            .unwrap();
        sink.clear();
        m.handle(&hello_frame("CONN_XYZ"), 1001, &mut sink).unwrap();
        let req = sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Http { req, .. } => Some(req),
                _ => None,
            })
            .expect("应有 resync Http");
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "connectionId" && v == "CONN_XYZ"),
            "sync Http 须带 connectionId 身份头"
        );
    }

    /// 顺序不变性：先 Connected 后 hello，终态 Connected + connectionId 已存。
    #[test]
    fn order_connected_then_hello_terminal_connected() {
        let mut m = module_with_channel();
        let mut sink = EffectSink::new();
        m.handle(&Tick::Connected(TransportId::from_raw(0)), 1000, &mut sink)
            .unwrap();
        m.handle(&hello_frame("C1"), 1001, &mut sink).unwrap();
        assert_eq!(m.state.conn, ConnState::Connected);
        assert_eq!(m.state.connection_id.as_deref(), Some("C1"));
    }
}
