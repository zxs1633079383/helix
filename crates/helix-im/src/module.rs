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

mod chain;
mod trait_impl;

/// Driver 向 sans-I/O IM 模块同步当前 host 身份的内部命令。
///
/// 该命令不属于 public client API；driver 必须直接构造 `AppCommand`，避免移动端绕过
/// header/profile 边界向业务 payload 注入身份。
pub const RUNTIME_IDENTITY_COMMAND: &str = "__helix_runtime_identity";

/// 产生本地权威投影 ACK 的宿主平台，固定枚举避免指标标签基数失控。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClientPlatform {
    #[default]
    Native,
    Ffi,
    Web,
}

impl ClientPlatform {
    /// 返回 Go ACK 合同和指标共用的稳定低基数字符串。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Ffi => "ffi",
            Self::Web => "web",
        }
    }
}

/// IM 业务模块配置
pub struct ImConfig {
    /// WS endpoint（如 "wss://api.example.com/ws"）
    pub ws_url: String,
    /// Go IM HTTP API base URL（通常含 `/api/cses`）。
    pub api_base_url: String,
    /// Java base（host/前端显式注入 `env.restHost`，无 `/api/cses`）。
    ///
    /// 媒体 prepare/verify 与 `Gateway::Default` 命令使用；不得从 Go `api_base_url` 推导。
    pub default_api_base_url: String,
    /// 当前登录用户 Id26；host 注入，供未读 sender 豁免、可见性门控与 channel 归属列使用。
    pub auth_user_id: String,
    /// 当前用户所属公司/团队 Id；host 注入，供 `posts/create` 的 teamId/userSnapshot 使用。
    pub company_id: String,
    pub user_name: String,
    pub org_name: String,
    pub dept_name: String,
    /// 当前宿主平台；只用于三端 ACK 归属，不接受业务 payload 覆盖。
    pub client_platform: ClientPlatform,
    pub ping_interval_ms: u64,
    pub send_timeout_ms: u64,
    /// 由 host 注入的纯离线 sync 诊断配置，默认关闭。
    pub offline_sync_diagnostics: crate::sync::SyncDiagnosticsConfig,
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
            client_platform: ClientPlatform::Native,
            ping_interval_ms: 8_000,
            send_timeout_ms: 15_000,
            offline_sync_diagnostics: crate::sync::SyncDiagnosticsConfig::disabled(),
        }
    }
}

/// IM 业务模块（实现 helix_core::Module trait）
pub struct ImModule {
    pub(crate) failure_context: crate::failure_context::FailureContext,
    pub(crate) bookmark_count: crate::bookmark_count::CountCache,
    pub(crate) config: ImConfig,
    pub(crate) render_scope: crate::render_scope::RenderScope,
    pub(crate) scope_hydration: crate::scope_hydration::ScopeHydration,
    pub(crate) sync_timing: crate::sync_observation::Timing,
    pub(crate) diagnostics: crate::diagnostics::Session,
    pub(crate) state: ImState,
    /// 平台注入的本地存储能力；coverage 仍由 query 状态机逐次证明。
    pub(crate) local_store_mode: crate::query::LocalStoreMode,
    /// Correlation 分配器（自增）
    ///
    /// BLK-1b：模块从 1 开始，避免与 engine 的 Correlation 冲突。
    /// 多模块场景由各模块从不同起点分配；更健壮的方案是 core 统一 IdSource。
    next_corr: u64,
    /// TimerId 分配器（自增）
    ///
    /// BLK-1b：从 100 开始，避免与 engine 内部潜在分配冲突。
    next_timer: u64,
    /// 发送临时 ID 的 module 内单调序号；与 step 的 now_ms 共同形成确定性 ID。
    next_temporary_id: u64,
}

impl ImModule {
    /// 排队一个原子业务写，并把 MessageV3 terminal events 绑定到 matching PersistOk。
    #[doc(hidden)]
    pub fn queue_message_v3_commit(
        &mut self,
        ops: Vec<helix_core::effect::StorageOp>,
        terminal_events: Vec<crate::event::MessageV3Event>,
        out: &mut EffectSink,
    ) -> helix_core::Correlation {
        let corr = self.alloc_corr_internal();
        self.render_scope.stage_ops(corr, &ops);
        let terminal_events = terminal_events
            .into_iter()
            .map(crate::event::MessageV3Event::into_bytes)
            .collect();
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageV3Commit { terminal_events },
        );
        out.push(helix_core::Effect::PersistAtomic { corr, ops });
        corr
    }

    /// PC/Flutter 默认使用 durable host store。
    pub fn new(config: ImConfig) -> Self {
        Self::new_with_local_store(config, crate::query::LocalStoreMode::Durable)
    }

    /// Web 可注入 Session/Disabled；三端业务 query API 保持一致。
    pub fn new_with_local_store(
        config: ImConfig,
        local_store_mode: crate::query::LocalStoreMode,
    ) -> Self {
        Self {
            config,
            failure_context: Default::default(),
            bookmark_count: Default::default(),
            render_scope: Default::default(),
            scope_hydration: Default::default(),
            diagnostics: crate::diagnostics::Session::default(),
            sync_timing: Default::default(),
            state: ImState::new(),
            local_store_mode,
            next_corr: 1,
            next_timer: 100,
            next_temporary_id: 1,
        }
    }

    pub fn local_store_mode(&self) -> crate::query::LocalStoreMode {
        self.local_store_mode
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

    pub(crate) fn alloc_temporary_id(
        &mut self,
        now_ms: u64,
    ) -> Result<TemporaryId, crate::error::ImError> {
        let sequence = self.next_temporary_id;
        self.next_temporary_id = sequence.checked_add(1).ok_or_else(|| {
            crate::error::ImError::Parse("temporary_id sequence exhausted".to_string())
        })?;
        Ok(TemporaryId::mint(now_ms, sequence))
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

    /// 查询已确认的 type7 terminal tombstone 水位（测试 / 诊断用）。
    pub fn terminal_event_seq_for(&self, channel_id: ChannelId) -> Option<u64> {
        self.state
            .channels
            .get(&channel_id)
            .and_then(|channel| channel.terminal_event_seq())
            .map(|seq| seq.0)
    }

    /// 查询 pending_sends 数量（测试用）
    pub fn pending_send_count(&self) -> usize {
        self.state.pending_sends.len()
    }

    /// 查询文字接龙 mutation 的本地状态，供 bridge 映射稳定状态而非重算业务事实。
    pub fn chain_mutation_state(
        &self,
        client_mutation_id: &str,
    ) -> Option<crate::chain::ChainMutationState> {
        self.state
            .chain_mutations
            .get(client_mutation_id)
            .map(|mutation| mutation.state)
    }

    /// 查询由 Helix 生成的稳定 operationId，供 reconcile 继续使用同一业务操作。
    pub fn chain_operation_id(&self, client_mutation_id: &str) -> Option<&str> {
        self.state
            .chain_mutations
            .get(client_mutation_id)
            .map(|mutation| mutation.operation_id.as_str())
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

    /// B-rest 测试 / driver 入口：直接喂 `increment_channel_end` 信号。
    /// `Some(ch)`=子 topic 结束；`None`=global end（触发增量群 sync）。
    pub fn ingest_increment_end(&mut self, ch: Option<ChannelId>, out: &mut EffectSink) {
        let start = out.as_slice().len();
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
        self.render_scope.guard_effects(&self.config, start, out);
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
        self.failure_context.websocket(frame);
        self.diagnose_inbound(frame);
        let module_name = self.name();
        let api_base_url = self.config.api_base_url.as_str();
        let auth_user_id = self.config.auth_user_id.as_str();
        // Only hello needs the local watermark scope.  Do not rescan the channel
        // ownership map for every ordinary WS frame.
        let hello_scope = (frame.action().ok() == Some("hello"))
            .then(|| self.render_scope.scoped_channel_ids(&self.config));
        let next_corr = &mut self.next_corr;
        let mut alloc_corr = || {
            let corr = helix_core::Correlation::from_raw(*next_corr);
            *next_corr += 1;
            corr
        };
        let (result, timeline_refreshes) = {
            let mut ctx = crate::ws::ImWsContext::new(
                &mut self.state,
                now_ms,
                api_base_url,
                auth_user_id,
                &mut alloc_corr,
            );
            if let Some(channel_ids) = hello_scope.as_deref() {
                ctx.set_scoped_channel_ids(channel_ids);
            }
            let result = crate::ws::dispatch_ws(&mut ctx, frame, out);
            let timeline_refreshes = ctx.take_attached_timeline_refreshes();
            (result, timeline_refreshes)
        };
        result.map_err(|e| CoreError::ModuleError {
            module: module_name,
            source: Box::new(e),
        })?;

        // The post handler has already appended its PersistFire write. The host
        // driver serializes Persist/PersistFire by the same `message` table key,
        // so this local-first query observes that accepted durable write before
        // producing the next MessageV3 event.
        for (channel_id, causation_id) in timeline_refreshes {
            self.refresh_attached_latest_timeline(channel_id, causation_id, out)
                .map_err(|e| CoreError::ModuleError {
                    module: module_name,
                    source: Box::new(e),
                })?;
        }
        Ok(())
    }
}
