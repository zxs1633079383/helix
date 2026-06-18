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
use crate::state::{
    ChannelId, ConnState, CorrelationContext, ImState, SendStatus, Seq, TemporaryId,
};
use helix_core::effect::{HttpRequest, ScanSpec, SqlValue, StorageOp, TimerId, UpsertSpec};
use helix_core::{CoreError, Effect, EffectSink, Module, Tick};

/// IM 业务模块配置
pub struct ImConfig {
    /// WS endpoint（如 "wss://api.example.com/ws"）
    pub ws_url: String,
    /// HTTP API base URL（如 "https://api.example.com"）
    pub api_base_url: String,
    /// 当前登录用户 Id26（host 经同 `api_base_url` 机制注入，确定性身份）。
    ///
    /// 用途：① path3 未读 +1 的 sender 豁免（自己消息 +0）+ 可见性门控（viewers 不含自己 +0）；
    /// ② path1 全量 upsert 写 channel `user_id` 列（频道归属）。空串 = 无身份（登录前 / 未注入）→
    /// 退化为保守计入（不漏未读，sender 豁免下沉前端）——但**正常运行期 host 必须注入真值**，
    /// 否则 §5 验收「自己 +0 / 不可见 +0 / echo 无 unread」不达成（BLOCKING-1 根因）。
    pub auth_user_id: String,
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
            auth_user_id: String::new(),
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

    /// 查询某 temporary_id 的 PendingSend 状态（测试 / 诊断用）。
    /// 用于 C1 echo→reconcile 断言「对账后 PendingSend 推进到 Sent」。
    pub fn pending_send_status(&self, temporary_id: &str) -> Option<SendStatus> {
        self.state
            .pending_sends
            .get(&TemporaryId(temporary_id.to_string()))
            .map(|ps| ps.status)
    }

    /// 查询 increment_fetched 是否含某 channel（测试 / 诊断用）。
    pub fn increment_fetched_contains(&self, channel_id: ChannelId) -> bool {
        self.state.increment_fetched.contains(&channel_id)
    }

    /// 查询某 channel 的 increment_target（服务端水位，测试 / 诊断用）。
    /// cursor（本地确认）与 target（服务端水位）严格解耦——HX-C008 回归断言锚点。
    pub fn increment_target_for(&self, channel_id: ChannelId) -> Option<u64> {
        self.state
            .increment_target
            .get(&channel_id)
            .map(|seq| seq.0)
    }

    /// B-rest 测试 / driver 入口：直接喂一个解析好的 `increment_channel` 帧。
    ///
    /// 真路径是 `Tick::Inbound` → `WsFrame::parse` → WS registry；本 pub 包装让
    /// 上层（driver / 回放测试）能直接驱动 increment 流程。
    pub fn ingest_increment(
        &mut self,
        inc: &crate::sync_session::IncrementChannel,
        out: &mut EffectSink,
    ) {
        let api_base_url = self.config.api_base_url.as_str();
        let auth_user_id = self.config.auth_user_id.as_str();
        let next_corr = &mut self.next_corr;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(*next_corr);
            *next_corr += 1;
            corr
        };
        let mut ctx = crate::ws::ImWsContext::new(
            &mut self.state,
            0,
            api_base_url,
            auth_user_id,
            &mut alloc_corr,
        );
        crate::ws::handlers::increment_channel::apply_increment(&mut ctx, inc, out);
    }

    /// B-rest 测试 / driver 入口：直接喂 `increment_channel_end` 信号。
    /// `Some(ch)`=子 topic 结束；`None`=global end（触发增量群 sync）。
    pub fn ingest_increment_end(&mut self, ch: Option<ChannelId>, out: &mut EffectSink) {
        let api_base_url = self.config.api_base_url.as_str();
        let auth_user_id = self.config.auth_user_id.as_str();
        let next_corr = &mut self.next_corr;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(*next_corr);
            *next_corr += 1;
            corr
        };
        let mut ctx = crate::ws::ImWsContext::new(
            &mut self.state,
            0,
            api_base_url,
            auth_user_id,
            &mut alloc_corr,
        );
        crate::ws::handlers::increment_channel_end::apply_increment_end(&mut ctx, ch, out);
    }

    /// 处理 PortReply（Persist/Http 的异步结果回报）
    ///
    /// ## MAJ-1 修复
    ///
    /// 原实现在 `state.pending_commits` 中查找，但 `channel.pending_commits` 才是
    /// cursor 推进的聚合根。修复（统一 `corr_map` 单表 + `match CorrelationContext` 分发，
    /// 类别互斥靠 corr 全局唯一保证，见 state.rs `CorrelationContext` 不变量）：
    /// 1. `corr_map.remove(corr)` 取出上下文（取代旧 `corr_to_channel` 路由索引）
    /// 2. `ChannelPersist` 变体 → 委托 `channel.on_persist_ok(corr, fx)` 推进 cursor
    /// 3. 15s 超时 timer → 通过 `pending_sends`/`OptimisticSend` 变体找 PendingSend
    ///
    /// ## E4 修复
    ///
    /// `ScanCursors` 变体：on_start 发出的 Scan{channel_event_cursor} PortReply
    /// 在此解析 rows（channel_id TEXT + last_event_seq INTEGER）→ register_channel + 批量发 proactive sync Http。
    fn handle_port_reply(
        &mut self,
        corr: helix_core::Correlation,
        outcome: &helix_core::tick::PortOutcome,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

        // 单次 remove 取出上下文（取代原 4 张表逐一 if-let 试探）。
        // 未命中（late/未知 corr）→ no-op，边界零信任（等价原"无人认领 → Ok(())"）。
        let Some(ctx) = self.state.corr_map.remove(&corr) else {
            return Ok(());
        };

        match ctx {
            // 路由（E4）：on_start Scan 回报 → 加载 cursor + 触发 proactive resync
            CorrelationContext::ScanCursors => match outcome {
                PortOutcome::Ok(reply) => {
                    self.handle_scan_reply(reply, out)?;
                }
                PortOutcome::Err(e) => {
                    // 降级：scan 失败（包括首次启动表不存在 / no such table）不崩，空 channel 集仍可用
                    tracing::warn!(
                        corr = corr.raw(),
                        error = ?e,
                        "scan channel_event_cursor failed (likely first run, no cursors yet), starting with empty channel set"
                    );
                }
            },

            // 路由：channel sync Persist 回报（cursor 推进 + E3 续拉）
            CorrelationContext::ChannelPersist {
                channel_id,
                wants_continuation,
            } => match outcome {
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
                        channel_id = channel_id.as_str(),
                        corr = corr.raw(),
                        error = ?e,
                        "persist failed, cursor not advanced, will re-sync"
                    );
                    // persist 失败：清除 last_sync_from_seq 避免遗留脏状态
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.last_sync_from_seq = None;
                    }
                }
            },

            // 路由：发送 Persist 回报（P1 乐观落库）
            CorrelationContext::OptimisticSend { temporary_id } => match outcome {
                PortOutcome::Ok(_) => {
                    // P1 落库成功：将 PendingSend 状态推进到 Sending
                    if let Some(ps) = self.state.pending_sends.get_mut(&temporary_id) {
                        ps.status = SendStatus::Sending;
                    }
                }
                PortOutcome::Err(e) => {
                    tracing::warn!(
                        tmp_id = ?temporary_id,
                        error = ?e,
                        "P1 optimistic persist failed"
                    );
                }
            },

            // 路由：sync Http 回报（sync/notify 响应）
            CorrelationContext::SyncPull { channel_id } => {
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
                            channel_id = channel_id.as_str(),
                            corr = corr.raw(),
                            error = ?e,
                            "sync/notify failed"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// E4 / S2：处理 on_start Scan{channel_event_cursor} 的 PortReply
    ///
    /// `channel_event_cursor` 是 advance_cursor 写入的实际表（channel_id TEXT PK,
    /// last_event_seq INTEGER, updated_at INTEGER）。engine_loop 将 rows 序列化为 JSON
    /// 数组 `[{"channel_id": "<id>", "last_event_seq": N, "updated_at": T}, ...]`。
    ///
    /// channel_id 是 26 字符 Id26 字符串（IM 业务约定，见 acl/to_effect.rs::advance_cursor）。
    /// last_event_seq 即该 channel 的 cursor 水位。
    fn handle_scan_reply(
        &mut self,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // engine_loop 约定：rows → JSON 数组 [{"col": val}, ...]
        // channel_event_cursor 表：channel_id TEXT PK, last_event_seq INTEGER（cursor）
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
            // channel_id 是 TEXT（26 字符 id），解析回 ChannelId（零信任）
            let channel_id = match row["channel_id"].as_str().and_then(ChannelId::from_str) {
                Some(id) => id,
                None => {
                    tracing::warn!(
                        channel_id = ?row["channel_id"],
                        "scan row channel_id not a valid 26-char channel_id, skipping"
                    );
                    continue;
                }
            };
            let cursor = match row["last_event_seq"].as_i64() {
                Some(v) if v >= 0 => v as u64,
                _ => 0u64,
            };
            // register_channel 幂等：若已存在则不覆盖（Connected 路径可能已注册）
            self.state
                .channels
                .entry(channel_id)
                .or_insert_with(|| crate::channel::Channel::new(channel_id, cursor));
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
    fn handle_hello(&mut self, connection_id: String, out: &mut EffectSink) {
        self.state.conn = ConnState::Connected;
        self.state.connection_id = Some(connection_id.clone());
        // §7：每个 hello 窗口起点清空上一会话残留的 increment 批次集（防 sync 风暴）。
        self.state.reset_increment_batch();
        out.push(crate::acl::to_effect::emit_connection_established(
            &connection_id,
        ));

        // B-rest：触发 increment HTTP（带已知 channel 游标）。HTTP 仅 ack 无数据。
        let conn_id = self.state.connection_id.clone();
        let cursors: Vec<(ChannelId, Seq)> = self
            .state
            .channels
            .iter()
            .map(|(&id, ch)| (id, ch.cursor.value()))
            .collect();
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
    fn emit_proactive_resync(&mut self, out: &mut EffectSink) {
        // NS-1：channels 是 HashMap，迭代序逐进程随机（SipHash 随机种子）。本路径逐 channel
        // 单调 alloc_corr + push 有序 Http Effect，故必须确定性排序锚定可回放（GOAL §1 确定性，
        // ChannelId: Ord）。冷路径（resync）排序零热路径代价。
        let mut all: Vec<ChannelId> = self.state.channels.keys().copied().collect();
        all.sort_unstable();
        self.emit_proactive_resync_for(&all, out);
    }

    /// 向指定 channel 集发 proactive sync Http（B-rest global-end 用增量群子集；
    /// scan_reply 用全集）。逐 channel 走既有 gate/peek-commit 链路。
    fn emit_proactive_resync_for(&mut self, targets: &[ChannelId], out: &mut EffectSink) {
        let api_base_url = self.config.api_base_url.as_str();
        let auth_user_id = self.config.auth_user_id.as_str();
        let next_corr = &mut self.next_corr;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(*next_corr);
            *next_corr += 1;
            corr
        };
        let mut ctx = crate::ws::ImWsContext::new(
            &mut self.state,
            0,
            api_base_url,
            auth_user_id,
            &mut alloc_corr,
        );
        crate::ws::handlers::increment_channel_end::emit_proactive_resync_for(
            &mut ctx, targets, out,
        );
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
                channel_id = channel_id.as_str(),
                "maybe_continue_sync: inflight_sync already set, skipping continuation"
            );
            return;
        }

        // 终止保证：新 fromSeq（= cursor after commit）必须严格 > prev_from_seq
        // 若相等，说明服务端返回的事件均低于上次 fromSeq（全部幂等丢弃，cursor 未前进）
        if let Some(prev_from) = prev_from_seq {
            if cursor_seq.0 <= prev_from.0 {
                tracing::warn!(
                    channel_id = channel_id.as_str(),
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
        self.state
            .corr_map
            .insert(sync_corr, CorrelationContext::SyncPull { channel_id });
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
            channel_id = channel_id.as_str(),
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

        // #6 信封解层（仅 SyncPull 路径，scan 路径不经此）：ADR-007 driver 把每条 2xx 响应包成
        // `{status, headers, body: base64(raw_go_body)}` 信封（host run_http_envelope，native+FFI 共享）。
        // 直接把信封喂 parse_sync_response → 找不到 data.entries → 静默 NoChange → 0 persist / 0 emit
        // → 离线消息悄无声息丢失（#6 latent bug）。故此处先剥信封 + base64 decode 出裸 Go body 再解析。
        // 边界零信任（HX-C 不变量 4）：信封 / base64 畸形走 Result::Err，不 unwrap、不 panic、不静默吞。
        let raw_body = unwrap_sync_envelope(reply.0.as_ref())?;
        let response = parse_sync_response(raw_body.as_ref(), channel_id)
            .map_err(|e| ImError::Parse(format!("sync response parse error: {}", e)))?;

        match response {
            SyncResponse::NoChange => {
                // cursor 已是最新，无事可做
            }

            SyncResponse::Events {
                events,
                messages,
                next_seq,
                needs_continuation,
            } => {
                if events.is_empty() {
                    if needs_continuation {
                        // 终止保证：服务端返回空 events 但仍声称需续拉 → 防无限循环
                        tracing::warn!(
                            channel_id = channel_id.as_str(),
                            "needs_continuation=true but events is empty, stopping continuation to prevent infinite loop"
                        );
                    }
                    return Ok(());
                }

                // peek/commit 三阶段：peek（只读预览，不推 cursor）
                // 为整批事件分配一个 Persist corr（先分配，避免借用冲突）
                // cursor 锚服务端权威 next_seq（跳过尾部稀疏/phantom 空洞，防 cursor 落后→sync 风暴）；
                // 取 max(next_seq, 实际末条) 兜底异常响应（next_seq < 末条时不丢已落库事件）。
                let max_event_seq = events.iter().map(|e| e.seq).max().unwrap_or(Seq(0));
                let commit_target = next_seq.max(max_event_seq);
                let persist_corr = self.alloc_corr_internal();
                // C2：按 messages map 落库（phantom 跳过 / readBits 单调覆盖 / edit / revoke）。
                // persist_ops 可能 < events.len()（全 phantom = 空），但 cursor 仍推进到
                // commit_target——phantom 不卡 cursor（HX-C008 §1.6，真源 post.rs:1271-1280）。
                let mut persist_ops =
                    crate::acl::to_effect::batch_upsert_events_with_messages(&events, &messages);
                // A3/CAP-9：可见 type1 离线未读 +1 落 DB（并入同一 Persist 批，与消息落库同 corr →
                // unread 列写成功后 cursor 才推进；不只 emit 把 +N 推前端）。真源 feat module.rs:563-571。
                let auth_user_id = self.config.auth_user_id.as_str();
                persist_ops.extend(crate::acl::to_effect::sync_unread_bump_ops(
                    &events,
                    &messages,
                    auth_user_id,
                ));
                if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                    ch.pending_commits.insert(persist_corr, commit_target);
                }
                out.push(Effect::Persist {
                    corr: persist_corr,
                    ops: persist_ops,
                });
                // E3：续拉意图归并为 ChannelPersist 变体的 wants_continuation 字段
                // （旧 corr_to_channel + continuation_pending 两张表合并为一条 entry）。
                self.state.corr_map.insert(
                    persist_corr,
                    CorrelationContext::ChannelPersist {
                        channel_id,
                        wants_continuation: needs_continuation,
                    },
                );
                // C3-core：落库 Persist 入队后，逐事件 emit sync 驱动的 mutation 事件
                // （post:received/updated/deleted/read + channel:update-by-post）。phantom-aware
                // 与落库同口径（可见才动 UI）；真源 apply_event_with_messages（post.rs:1146-1448）。
                // 落库与 emit 同批 step 内吐出，前端经 im:__bus__ 消费（HX-C008：emit 不卡 cursor，
                // cursor 仍由 persist_corr 的 PortReply Ok 独立推进到 commit_target）。
                for emit in crate::acl::to_effect::sync_mutation_emits(&events, &messages) {
                    out.push(emit);
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
                // Snapshot 无续拉语义，wants_continuation 恒 false（与原行为一致：
                // 原 Snapshot 分支不写 continuation_pending）。
                self.state.corr_map.insert(
                    persist_corr,
                    CorrelationContext::ChannelPersist {
                        channel_id,
                        wants_continuation: false,
                    },
                );
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
                // ping WS 帧带单调 seq（Go websocket_router 拒 seq≤0）。
                let seq = self.state.next_client_seq();
                out.push(crate::acl::to_effect::ping_frame(seq));
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
            .as_str()
            .and_then(ChannelId::from_str)
            .ok_or_else(|| ImError::Parse("missing/invalid channel_id".to_string()))?;
        let tmp_id_str = cmd["temporary_id"]
            .as_str()
            .ok_or_else(|| ImError::Parse("missing temporary_id".to_string()))?
            .to_string();
        let text = cmd["text"].as_str().unwrap_or("").to_string();
        let tmp_id = TemporaryId(tmp_id_str.clone());

        // 分配 corr / timer
        let p1_corr = self.alloc_corr_internal();
        let h1_corr = self.alloc_corr_internal();
        let t1_timer = self.alloc_timer();

        // 1. Persist{P1, 乐观落库 status=Sending}
        // 表名 message（PK=temporary_id，对齐 678 真库；见 schema::IM_SCHEMA）
        out.push(Effect::Persist {
            corr: p1_corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                table: "message",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(tmp_id_str.clone()),
                    ),
                    (
                        "channel_id".to_string(),
                        SqlValue::Text(channel_id.as_str().to_string()),
                    ),
                    ("message".to_string(), SqlValue::Text(text.clone())),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("sending".to_string()),
                    ),
                ]],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
            })],
        });

        // 2. Http{H1, POST /posts/create}
        let body = serde_json::json!({
            "channel_id": channel_id.as_str(),
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
        // R4-retain：回填本 send 的 P1 Persist corr，对账时按它 O(1) `corr_map.remove`，
        // 取代旧 `corr_map.retain` 全表扫描清 OptimisticSend 路由。
        ps.persist_corr = Some(p1_corr);
        self.state.pending_sends.insert(tmp_id, ps);
        self.state.corr_map.insert(
            p1_corr,
            CorrelationContext::OptimisticSend {
                temporary_id: TemporaryId(tmp_id_str),
            },
        );

        Ok(())
    }

    /// `Tick::Inbound` 解析后的统一 dispatch：`WsFrame::parse` → WS registry by action。
    ///
    /// registry 策略注册表按顶层 `action` 路由到对应 handler（hello / increment_channel /
    /// increment_channel_end / post / posted / 运维 ignored / 无 action 的 legacy event）。
    /// 未知 action → `UnsupportedWsAction`（生产路径可观测）。各 handler 在 `ImWsContext`
    /// 上操作 state，副作用经 `out.push(Effect::…)`，保持 handle 同步纯函数。
    fn dispatch_ws_frame(
        &mut self,
        frame: &crate::ws::WsFrame,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), CoreError> {
        let module_name = self.name();
        let api_base_url = self.config.api_base_url.as_str();
        let auth_user_id = self.config.auth_user_id.as_str();
        let next_corr = &mut self.next_corr;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(*next_corr);
            *next_corr += 1;
            corr
        };
        let mut ctx = crate::ws::ImWsContext::new(
            &mut self.state,
            now_ms,
            api_base_url,
            auth_user_id,
            &mut alloc_corr,
        );
        crate::ws::dispatch_ws(&mut ctx, frame, out).map_err(|e| CoreError::ModuleError {
            module: module_name,
            source: Box::new(e),
        })
    }
}

/// 剥 ADR-007 HTTP 信封（仅 SyncPull 路径用）：`{status, headers, body: base64(raw_go_body)}`
/// → 裸 Go body 字节。host `run_http_envelope` 的反序列化端，base64 decode 在 helix-im 内联
/// （HX-C001：不依赖 helix-driver-host）。边界零信任：信封 / base64 畸形 → `Err`，不 panic。
fn unwrap_sync_envelope(reply: &[u8]) -> Result<Vec<u8>, ImError> {
    let env: serde_json::Value = serde_json::from_slice(reply)
        .map_err(|e| ImError::Parse(format!("sync envelope JSON parse error: {}", e)))?;
    let body_b64 = env
        .get("body")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ImError::Parse("sync envelope missing string `body` field".to_string()))?;
    crate::base64::decode(body_b64)
        .map_err(|e| ImError::Parse(format!("sync envelope body base64 decode error: {}", e)))
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

    /// 启动：读取 cursor + arm ping timer + 吐 Scan（§4.8 proactive resync 所需）
    ///
    /// Scan 吐 Persist{corr, ops:[Scan(channel_event_cursor)]}。
    /// channel_event_cursor 是 advance_cursor（acl/to_effect.rs）写入的实际 cursor 表，
    /// 列：channel_id TEXT PRIMARY KEY（= channel_id.as_str()）, last_event_seq INTEGER（= cursor seq）。
    /// PortReply 回来后 handle_scan_reply 解析 channel_id→ChannelId + last_event_seq，注册 channel，
    /// 然后对全部已知 channel 发 proactive sync Http（§4.8）。
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        // 1. 吐 Persist{corr, Scan(channel_event_cursor)} 载入全部 cursor
        // 注意：此表由 schema migrate 建（schema.rs），首次启动（无任何 cursor 写过）时为空。
        // driver scan 对 no such table 应降级返回 PortOutcome::Err，handle_scan_reply 处理降级。
        let scan_corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr: scan_corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "channel_event_cursor",
                limit: None,
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
        m.register_channel(ChannelId::from_test_u64(1), 0);
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
