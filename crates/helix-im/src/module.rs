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

pub(crate) mod action;
mod trait_impl;

/// Driver 向 sans-I/O IM 模块同步当前 host 身份的内部命令。
///
/// 该命令不属于 public client API；driver 必须直接构造 `AppCommand`，避免移动端绕过
/// header/profile 边界向业务 payload 注入身份。
pub const RUNTIME_IDENTITY_COMMAND: &str = "__helix_runtime_identity";

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
        let (result, timeline_refreshes) = {
            let mut ctx = crate::ws::ImWsContext::new(
                &mut self.state,
                now_ms,
                api_base_url,
                auth_user_id,
                &mut alloc_corr,
            );
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
        // producing the next typed Frame.
        for channel_id in timeline_refreshes {
            self.refresh_attached_latest_timeline(channel_id, None, out)
                .map_err(|e| CoreError::ModuleError {
                    module: module_name,
                    source: Box::new(e),
                })?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "module_tests.rs"]
mod handshake_tests;
