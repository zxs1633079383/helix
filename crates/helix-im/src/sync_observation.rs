//! 同步时序的只读旁路：单轮和单页 O(1) 状态，指标出口由宿主提供。
use crate::{diagnostics::Observation, ImModule};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStage {
    Recovery,
    InventoryPage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncResult {
    Started,
    Success,
    Failed,
    Cancelled,
}
impl SyncResult {
    /// 固定枚举同时约束日志和低基数标签。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}
/// 仅传标量给宿主，实体与关联 ID 不进入指标端口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncRecord {
    pub stage: SyncStage,
    pub result: SyncResult,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub total_pages: Option<u64>,
}
pub trait SyncObserver: Send + Sync {
    /// 实现必须非阻塞；丢弃观测不得影响业务结果。
    fn record(&self, record: SyncRecord);
}
impl<F: Fn(SyncRecord) + Send + Sync> SyncObserver for F {
    /// 装配层可用闭包转接标量，无需让通用driver反向依赖IM。
    fn record(&self, record: SyncRecord) {
        self(record);
    }
}
#[derive(Default)]
pub(crate) struct Timing {
    observer: Option<Arc<dyn SyncObserver>>,
    epoch: u64,
    corr_floor: u64,
    pending_operations: u64,
    started: Option<u64>,
    session: String,
    page_started: Option<u64>,
    pub page_index: u64,
    pub total_pages: Option<u64>,
    pub inventory_committed: bool,
}
impl ImModule {
    /// 注入独立指标出口，诊断日志关闭时仍记录同步时间。
    pub fn with_sync_observer(mut self, observer: Arc<dyn SyncObserver>) -> Self {
        self.sync_timing.observer = Some(observer);
        self
    }
    /// 每次业务 Tick 后读现有门槛；不修改恢复状态、队列或持久化语义。
    pub(crate) fn observe_sync_tick(&mut self, effects: &[helix_core::Effect], corr_floor: u64) {
        let epoch = self.state.recovery_session.session_epoch;
        if epoch != 0 && epoch != self.sync_timing.epoch {
            self.finish_sync_observation(SyncResult::Cancelled);
            self.sync_timing.epoch = epoch;
            self.sync_timing.corr_floor = corr_floor;
            self.sync_timing.pending_operations = 0;
            self.sync_timing.started = Some(self.diagnostics.now_ms);
            self.sync_timing.session = self.state.connection_id.clone().unwrap_or_default();
            self.sync_timing.page_index = 0;
            self.sync_timing.total_pages = None;
            self.sync_timing.inventory_committed = false;
            self.emit_sync_observation(
                SyncStage::Recovery,
                SyncResult::Started,
                self.diagnostics.now_ms,
            );
        }
        if self.sync_timing.started.is_none() {
            return;
        }
        // 只看本Tick新增Effect及其既有相关上下文：O(1)/Effect，额外状态始终O(1)。
        for effect in effects {
            let corr = match effect {
                helix_core::Effect::Http { corr, .. }
                | helix_core::Effect::Persist { corr, .. }
                | helix_core::Effect::PersistAtomic { corr, .. } => *corr,
                _ => continue,
            };
            if corr.raw() >= self.sync_timing.corr_floor
                && self
                    .state
                    .corr_map
                    .get(&corr)
                    .is_some_and(is_recovery_operation)
            {
                self.sync_timing.pending_operations += 1;
            }
        }
        if matches!(
            self.state.recovery_session.phase,
            crate::sync_session::RecoveryPhase::Failed
                | crate::sync_session::RecoveryPhase::Blocked
        ) {
            self.finish_sync_observation(SyncResult::Failed);
        } else if self.state.startup_channel_projection_ready
            && self.sync_timing.pending_operations == 0
            && self.sync_timing.inventory_committed
            && self.state.increment_pull.is_none()
            && self.state.channel_sync_persist_inflight == 0
            && !self.state.channel_sync_batch_pending
            && self.state.sync_scheduler.is_idle()
            && !self.state.recovery_session.has_pending_commits()
        {
            self.finish_sync_observation(SyncResult::Success);
        }
    }
    /// 在业务移除corr前消费匹配回执；旧轮corr低于单调分配水位，不能关闭新轮屏障。
    pub(crate) fn observe_sync_reply(&mut self, tick: &helix_core::Tick) -> bool {
        let helix_core::Tick::PortReply { corr, outcome } = tick else {
            return false;
        };
        if self.sync_timing.started.is_none() || corr.raw() < self.sync_timing.corr_floor {
            return false;
        }
        let Some(context) = self.state.corr_map.get(corr) else {
            return false;
        };
        if !is_recovery_operation(context) {
            return false;
        }
        self.sync_timing.pending_operations = self.sync_timing.pending_operations.saturating_sub(1);
        if matches!(outcome, helix_core::tick::PortOutcome::Err(_)) {
            self.finish_sync_observation(SyncResult::Failed);
        } else if matches!(
            context,
            crate::state::CorrelationContext::IncrementBatchPersist { .. }
        ) {
            self.sync_timing.inventory_committed = true;
        }
        true
    }
    /// 单页起点是发出 HTTP Effect；后续 HTTP 返回不能提前关闭本页。
    pub(crate) fn start_page_observation(&mut self) {
        if self.sync_timing.started.is_none() {
            return;
        }
        self.sync_timing.page_index += 1;
        self.sync_timing.page_started = Some(self.diagnostics.now_ms);
        self.emit_sync_observation(
            SyncStage::InventoryPage,
            SyncResult::Started,
            self.diagnostics.now_ms,
        );
    }
    /// 仅匹配的持久化回执或经过验证的空页可以成功关闭页面。
    pub(crate) fn finish_page_observation(&mut self, result: SyncResult) {
        if let Some(started) = self.sync_timing.page_started.take() {
            self.emit_sync_observation(SyncStage::InventoryPage, result, started);
        }
    }
    /// take 保证错误、断线、停止和重复回执都只产生一次终态。
    pub(crate) fn finish_sync_observation(&mut self, result: SyncResult) {
        self.finish_page_observation(result);
        if let Some(started) = self.sync_timing.started.take() {
            self.emit_sync_observation(SyncStage::Recovery, result, started);
        }
    }
    /// 指标走 Copy 端口，诊断走既有固定字段旁路；不序列化业务对象。
    fn emit_sync_observation(&self, stage: SyncStage, result: SyncResult, started: u64) {
        let now = self.diagnostics.now_ms;
        let record = SyncRecord {
            stage,
            result,
            started_at_ms: started,
            completed_at_ms: now,
            total_pages: self.sync_timing.total_pages,
        };
        if let Some(observer) = &self.sync_timing.observer {
            observer.record(record);
        }
        self.diagnose(Observation {
            event: match (stage, result) {
                (SyncStage::Recovery, SyncResult::Started) => "sync_recovery_started",
                (SyncStage::Recovery, _) => "sync_recovery_terminal",
                (SyncStage::InventoryPage, SyncResult::Started) => "sync_inventory_page_started",
                (SyncStage::InventoryPage, _) => "sync_inventory_page_terminal",
            },
            stage: "client",
            result: result.as_str(),
            sync_session_id: &self.sync_timing.session,
            page_index: (stage == SyncStage::InventoryPage).then_some(self.sync_timing.page_index),
            total_pages: self.sync_timing.total_pages,
            started_at_ms: Some(started),
            completed_at_ms: (result != SyncResult::Started).then_some(now),
            elapsed: now.saturating_sub(started) as f64 / 1000.0,
            ..Default::default()
        });
    }
}

/// 仅列恢复请求与durable提交，不把UI查询/遥测或独立业务命令计入恢复。
fn is_recovery_operation(context: &crate::state::CorrelationContext) -> bool {
    use crate::state::CorrelationContext::*;
    matches!(
        context,
        ScanCursors
            | ScanChannelProjections
            | IncrementMessageTimestampScan { .. }
            | IncrementPullHttp
            | IncrementPullPersist
            | IncrementBatchPersist { .. }
            | SyncPull { .. }
            | ChannelPersist { .. }
            | ChannelTerminalPersist { .. }
            | MemberProjectionPersist { .. }
            | TooLongReload { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::CorrelationContext;
    use helix_core::{Correlation, Effect, Tick};

    /// 已发布旧恢复事件也不能越过后续真实持久化；观察不依赖completion_published。
    #[test]
    fn continuation_persist_remains_a_terminal_barrier() {
        let mut module = ImModule::new(Default::default());
        module.state.recovery_session.begin("actor");
        module.observe_sync_tick(&[], 10);
        module.sync_timing.inventory_committed = true;
        module.state.startup_channel_projection_ready = true;
        module.state.channel_sync_batch_pending = false;
        module.state.recovery_session.mark_completion_published();
        let corr = Correlation::from_raw(11);
        module.state.corr_map.insert(
            corr,
            CorrelationContext::IncrementBatchPersist {
                projections: vec![],
                batch_id: None,
            },
        );
        module.observe_sync_tick(&[Effect::PersistAtomic { corr, ops: vec![] }], 12);
        assert!(module.sync_timing.started.is_some());
        module.observe_sync_reply(&Tick::PortReply {
            corr,
            outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                bytes::Bytes::new(),
            )),
        });
        module.state.corr_map.remove(&corr);
        module.observe_sync_tick(&[], 12);
        assert!(module.sync_timing.started.is_none());
    }

    /// 前连接的最终屏障不能为新连接认证成功或失败。
    #[test]
    fn old_inventory_receipt_does_not_complete_new_epoch() {
        let mut module = ImModule::new(Default::default());
        module.state.recovery_session.begin("actor");
        module.observe_sync_tick(&[], 10);
        let old = Correlation::from_raw(5);
        module.state.corr_map.insert(
            old,
            CorrelationContext::IncrementBatchPersist {
                projections: vec![],
                batch_id: None,
            },
        );
        module.observe_sync_reply(&Tick::PortReply {
            corr: old,
            outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                bytes::Bytes::new(),
            )),
        });
        assert!(!module.sync_timing.inventory_committed);
        assert!(module.sync_timing.started.is_some());
    }
}
