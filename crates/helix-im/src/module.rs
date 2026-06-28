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
use crate::state::{ChannelId, ConnState, CorrelationContext, ImState, SendStatus, TemporaryId};
use helix_core::effect::TimerId;
use helix_core::{CoreError, EffectSink, Module, Tick};

/// IM 业务模块配置
pub struct ImConfig {
    /// WS endpoint（如 "wss://api.example.com/ws"）
    pub ws_url: String,
    /// HTTP API base URL（如 "https://api.example.com"）
    pub api_base_url: String,
    /// 第二业务网关 base（host 注入 `env.restHost`，无 `/api/cses`）；仅 `Gateway::Default` 命令使用。
    pub default_api_base_url: String,
    /// 当前登录用户 Id26；host 注入，供未读 sender 豁免、可见性门控与 channel 归属列使用。
    pub auth_user_id: String,
    /// 当前用户所属公司/团队 Id；host 注入，供 `posts/create` 的 teamId/userSnapshot 使用。
    pub company_id: String,
    pub user_name: String,
    pub org_name: String,
    pub dept_name: String,
    pub ping_interval_ms: u64,
    pub send_timeout_ms: u64,
}

impl ImConfig {
    /// 当前登录用户身份视图（userSnapshot 注入源，posts/create body §1）。
    pub fn user_identity(&self) -> crate::outbound::send_build::UserIdentity<'_> {
        crate::outbound::send_build::UserIdentity {
            user_id: &self.auth_user_id,
            team_id: &self.company_id,
            user_name: &self.user_name,
            org_name: &self.org_name,
            dept_name: &self.dept_name,
        }
    }
}

impl Default for ImConfig {
    fn default() -> Self {
        Self {
            ws_url: String::new(),
            api_base_url: String::new(),
            default_api_base_url: String::new(),
            auth_user_id: String::new(),
            company_id: String::new(),
            user_name: String::new(),
            org_name: String::new(),
            dept_name: String::new(),
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

    /// B4：全局在途 sync 数 + 待 dispatch 队列长（HX-C011 证伪性质测试结构化断言用）。
    pub fn sync_inflight(&self) -> usize {
        self.state.sync_scheduler.inflight()
    }
    pub fn sync_pending_len(&self) -> usize {
        self.state.sync_scheduler.pending_len()
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

    /// 内部 alloc_corr（不借用 self.state，仅借用分配器）
    pub(crate) fn alloc_corr_internal(&mut self) -> helix_core::Correlation {
        let c = helix_core::Correlation::from_raw(self.next_corr);
        self.next_corr += 1;
        c
    }

    /// 拆借 state 与 correlation 分配器，供 sibling module 复用私有 `next_corr` 的顺序分配。
    pub(crate) fn with_state_and_corr_allocator<R>(
        &mut self,
        f: impl FnOnce(&mut ImState, &mut dyn FnMut() -> helix_core::Correlation) -> R,
    ) -> R {
        let next_corr = &mut self.next_corr;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(*next_corr);
            *next_corr += 1;
            corr
        };
        f(&mut self.state, &mut alloc_corr)
    }

    /// `Tick::Inbound` 解析后的统一 dispatch：`WsFrame::parse` → WS registry by `action`（hello /
    /// increment_channel(_end) / post / 19 事件全集 / dead-action no-op / 无 action 的 legacy event）。
    /// 未知 action → `UnsupportedWsAction`（可观测）。各 handler 在 `ImWsContext` 上操作 state，副作用
    /// 经 `out.push(Effect::…)`，保持 handle 同步纯函数。
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
                // 连接控制（04 文档表 B `IMWS:reconnect`）：helix-im 是 sans-IO，不持 transport——
                // 只 emit `im:net:reconnect_requested` 控制信号，由 driver 观察后重连 NativeTransport
                // （连接生命周期归 driver）。payload 无须解析（无参控制信号）。
                "im_reconnect" => {
                    out.push(crate::acl::to_effect::emit_reconnect_requested());
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
                            let ctx = if name == "im_channels_members_by_ids" {
                                CorrelationContext::OutboundMembersByIds { req_id }
                            } else if name == "im_get_replies" || name == "im_get_reply_branch" {
                                // S8：回复族 → 双 emit（im:read:result 冻结 + im:channel:replies render-ready）。
                                CorrelationContext::OutboundReplies { req_id }
                            } else {
                                CorrelationContext::OutboundReadReply { req_id }
                            };
                            self.state.corr_map.insert(corr, ctx);
                        }
                    }
                    for eff in effects {
                        out.push(eff);
                    }
                }
                // P6 投影/状态命令（im_query_messages_by_channel / im_delete_all_dialogs）：
                // 本地读/清，零 HTTP；契约在 crate::query（薄壳，与 outbound registry 解耦）。
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

#[cfg(test)]
#[path = "module_tests.rs"]
mod handshake_tests;
