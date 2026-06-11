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
use helix_core::effect::TimerId;
use crate::state::{ImState, ConnState};
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
    config: ImConfig,
    state: ImState,
    /// Correlation 分配器（自增）
    next_corr: u64,
    /// TimerId 分配器（自增）
    next_timer: u64,
}

impl ImModule {
    pub fn new(config: ImConfig) -> Self {
        Self {
            config,
            state: ImState::new(),
            next_corr: 1,
            next_timer: 100, // 从 100 开始，避免与 engine 内部分配冲突
        }
    }

    fn alloc_corr(&mut self) -> helix_core::Correlation {
        // BLK-1b: 使用 from_raw 构造，模块内部自增保证模块级单调唯一性。
        // 多模块场景的全局唯一性由各模块从不同起点（或 next_timer 偏移）保证；
        // 更健壮的方案是 core 统一分配，此处先用偏移策略作骨架实现。
        let c = helix_core::Correlation::from_raw(self.next_corr);
        self.next_corr += 1;
        c
    }

    fn alloc_timer(&mut self) -> TimerId {
        let t = TimerId::from_raw(self.next_timer);
        self.next_timer += 1;
        t
    }

    /// 处理 PortReply（Persist/Http 的异步结果回报）
    ///
    /// ## MAJ-1 修复
    ///
    /// 原实现在 `state.pending_commits` 中查找 `(channel_id, target_seq)`，
    /// 但 `channel.pending_commits` 才是真正持有 `corr → Seq` 的结构，
    /// 两份 pending_commits 是独立的 source of truth，导致路由分裂。
    ///
    /// 修复：
    /// 1. `state.corr_to_channel`（`Correlation → ChannelId` 路由索引）替代原 `state.pending_commits`
    /// 2. 查到 channel_id 后，委托 `channel.on_persist_ok(corr, fx)` 处理 cursor 推进
    /// 3. 删除 `ImState.pending_commits`，Channel 成为 cursor 推进的唯一聚合根
    fn handle_port_reply(
        &mut self,
        corr: helix_core::Correlation,
        outcome: &helix_core::tick::PortOutcome,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        use helix_core::tick::PortOutcome;

        // MAJ-1: 从路由索引取 channel_id，然后委托给 channel 聚合根
        if let Some(channel_id) = self.state.corr_to_channel.remove(&corr) {
            match outcome {
                PortOutcome::Ok(_) => {
                    if let Some(ch) = self.state.channels.get_mut(&channel_id) {
                        ch.on_persist_ok(corr, out)?;
                    }
                }
                PortOutcome::Err(e) => {
                    // 写失败：不推 cursor，下次 sync 重拉
                    tracing::warn!(
                        channel_id = channel_id.0,
                        corr = corr.raw(),
                        error = ?e,
                        "persist failed, cursor not advanced, will re-sync"
                    );
                }
            }
        }

        // TODO: 处理 Http PortReply（sync/notify 响应）

        Ok(())
    }

    /// 处理 Timer 触发
    fn handle_timer(
        &mut self,
        timer_id: TimerId,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // ping timer
        if self.state.ping_timer == Some(timer_id) {
            if self.state.conn == ConnState::Connected {
                // TODO: 吐 Send{ping 帧}
                // 重新 arm（self-arm 实现无 tick() 的周期心跳）
                let new_timer = self.alloc_timer();
                out.push(Effect::ScheduleTimer {
                    id: new_timer,
                    after_ms: self.config.ping_interval_ms,
                });
                self.state.ping_timer = Some(new_timer);
            }
            return Ok(());
        }

        // backoff timer（重连）
        if self.state.backoff_timer == Some(timer_id) {
            self.state.backoff_timer = None;
            // TODO: 吐 Transport.connect() → Effect::Send{hello}
            self.state.conn = ConnState::Connecting;
            return Ok(());
        }

        // 15s 发送超时 timer
        // TODO: 查 pending_sends，找匹配的 timer_id，调用 on_timeout

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
                // TODO: 处理 im_send_message / im_read_channel 等命令
                tracing::debug!("im: command '{}' received", cmd.name);
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

            Tick::Connected(transport) => {
                // TODO(Phase B): 重连业务策略——cursor reset 判定 + proactive resync（§4.7/4.8）
                tracing::debug!(transport = transport.raw(), "im: transport connected");
            }

            Tick::Disconnected(transport) => {
                // TODO(Phase B): 标记离线态（重连退避是 driver 的事，模块只感知状态）
                tracing::debug!(transport = transport.raw(), "im: transport disconnected");
            }
        }
        Ok(())
    }

    /// 启动：读取 cursor + 连接 WS + arm ping timer
    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        // TODO: 吐 Persist{get cursors for all channels, corr}
        // TODO: 吐 Effect::Send{hello 帧} or Transport connect signal

        // arm ping timer（self-arm 实现周期心跳，无 tick() 方法）
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
        // TODO: 吐 Effect::Send{WS close 帧}
        self.state.conn = ConnState::Closing;
        Ok(())
    }
}
