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
    pending_reported_at_ms: Option<u64>,
    started: Option<u64>,
    session: String,
    page_started: Option<u64>,
    pub page_index: u64,
    pub total_pages: Option<u64>,
    pub inventory_committed: bool,
}

/// `barrier_mask` 是诊断快照，不参与完成判定；bit=1 表示该屏障仍未闭合。
/// `pending_operations` 单独保留，避免把异步 Effect 数量塞进业务 `count`。
pub(crate) const RECOVERY_BARRIER_STARTUP_PROJECTION: u64 = 1 << 0;
pub(crate) const RECOVERY_BARRIER_INVENTORY_COMMIT: u64 = 1 << 1;
pub(crate) const RECOVERY_BARRIER_INCREMENT_PULL: u64 = 1 << 2;
pub(crate) const RECOVERY_BARRIER_BATCH_PERSIST: u64 = 1 << 3;
pub(crate) const RECOVERY_BARRIER_BATCH_PENDING: u64 = 1 << 4;
pub(crate) const RECOVERY_BARRIER_SCHEDULER: u64 = 1 << 5;
pub(crate) const RECOVERY_BARRIER_PENDING_COMMITS: u64 = 1 << 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RecoverySnapshot {
    pending_operations: u64,
    barrier_mask: u64,
    scheduler_inflight: u64,
    scheduler_pending: u64,
    batch_persist_inflight: u64,
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
        let mut recovery_started = false;
        if epoch != 0 && epoch != self.sync_timing.epoch {
            self.finish_sync_observation(SyncResult::Cancelled);
            self.sync_timing.epoch = epoch;
            self.sync_timing.corr_floor = corr_floor;
            self.sync_timing.pending_operations = 0;
            self.sync_timing.pending_reported_at_ms = None;
            self.sync_timing.started = Some(self.diagnostics.now_ms);
            self.sync_timing.session = self.state.connection_id.clone().unwrap_or_default();
            self.sync_timing.page_index = 0;
            self.sync_timing.total_pages = None;
            self.sync_timing.inventory_committed = false;
            recovery_started = true;
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
        // Started 快照放在本Tick新增 Effect 已计数之后，才能记录真实的首个待处理屏障。
        if recovery_started {
            let now = self.diagnostics.now_ms;
            self.emit_sync_observation(SyncStage::Recovery, SyncResult::Started, now);
            self.sync_timing.pending_reported_at_ms = Some(now);
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
        } else {
            self.emit_recovery_pending_if_due();
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
            self.sync_timing.pending_reported_at_ms = None;
        }
    }

    /// 仅在恢复仍活跃且距上次快照至少一秒时记录旁路状态；不读取时钟或执行I/O。
    fn emit_recovery_pending_if_due(&mut self) {
        let now = self.diagnostics.now_ms;
        if self
            .sync_timing
            .pending_reported_at_ms
            .is_some_and(|last| now.saturating_sub(last) < 1_000)
        {
            return;
        }
        self.sync_timing.pending_reported_at_ms = Some(now);
        let snapshot = self.recovery_snapshot();
        self.diagnose(Observation {
            event: "sync_recovery_pending",
            stage: "client",
            result: "pending",
            sync_session_id: &self.sync_timing.session,
            pending_operations: Some(snapshot.pending_operations),
            barrier_mask: Some(snapshot.barrier_mask),
            scheduler_inflight: Some(snapshot.scheduler_inflight),
            scheduler_pending: Some(snapshot.scheduler_pending),
            batch_persist_inflight: Some(snapshot.batch_persist_inflight),
            ..Default::default()
        });
    }

    /// 只读取既有状态字段，所有计数均为诊断快照，不参与业务状态推进。
    fn recovery_snapshot(&self) -> RecoverySnapshot {
        RecoverySnapshot {
            pending_operations: self.sync_timing.pending_operations,
            barrier_mask: self.recovery_barrier_mask(),
            scheduler_inflight: self.state.sync_scheduler.inflight() as u64,
            scheduler_pending: self.state.sync_scheduler.pending_len() as u64,
            batch_persist_inflight: self.state.channel_sync_persist_inflight as u64,
        }
    }

    /// bit=1 表示对应 durable 屏障仍未闭合；该 mask 只用于解释 pending/cancelled。
    fn recovery_barrier_mask(&self) -> u64 {
        let mut mask = 0;
        if !self.state.startup_channel_projection_ready {
            mask |= RECOVERY_BARRIER_STARTUP_PROJECTION;
        }
        if !self.sync_timing.inventory_committed {
            mask |= RECOVERY_BARRIER_INVENTORY_COMMIT;
        }
        if self.state.increment_pull.is_some() {
            mask |= RECOVERY_BARRIER_INCREMENT_PULL;
        }
        if self.state.channel_sync_persist_inflight != 0 {
            mask |= RECOVERY_BARRIER_BATCH_PERSIST;
        }
        if self.state.channel_sync_batch_pending {
            mask |= RECOVERY_BARRIER_BATCH_PENDING;
        }
        if !self.state.sync_scheduler.is_idle() {
            mask |= RECOVERY_BARRIER_SCHEDULER;
        }
        if self.state.recovery_session.has_pending_commits() {
            mask |= RECOVERY_BARRIER_PENDING_COMMITS;
        }
        mask
    }
    /// 指标走 Copy 端口，诊断走既有固定字段旁路；不序列化业务对象。
    fn emit_sync_observation(&self, stage: SyncStage, result: SyncResult, started: u64) {
        let now = self.diagnostics.now_ms;
        let snapshot = (stage == SyncStage::Recovery).then(|| self.recovery_snapshot());
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
            pending_operations: snapshot.map(|value| value.pending_operations),
            barrier_mask: snapshot.map(|value| value.barrier_mask),
            scheduler_inflight: snapshot.map(|value| value.scheduler_inflight),
            scheduler_pending: snapshot.map(|value| value.scheduler_pending),
            batch_persist_inflight: snapshot.map(|value| value.batch_persist_inflight),
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
    use helix_core::{Correlation, Effect, EffectSink, Tick};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event, Id, Metadata, Subscriber};

    #[derive(Clone, Debug)]
    struct CapturedEvent {
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct Recorder {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    struct FieldRecorder<'a> {
        fields: &'a mut BTreeMap<String, String>,
    }

    impl Visit for FieldRecorder<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_f64(&mut self, field: &Field, value: f64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    impl Subscriber for Recorder {
        fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
            if metadata.target() == "helix::diagnostics" {
                Interest::always()
            } else {
                Interest::never()
            }
        }

        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.target() == "helix::diagnostics"
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            if event.metadata().target() != "helix::diagnostics" {
                return;
            }
            let mut fields = BTreeMap::new();
            event.record(&mut FieldRecorder {
                fields: &mut fields,
            });
            self.events
                .lock()
                .expect("diagnostic recorder lock")
                .push(CapturedEvent { fields });
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    fn captured(recorder: &Recorder, event: &str) -> Vec<CapturedEvent> {
        recorder
            .events
            .lock()
            .expect("diagnostic recorder lock")
            .iter()
            .filter(|record| record.fields.get("event").map(String::as_str) == Some(event))
            .cloned()
            .collect()
    }

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

    #[test]
    fn recovery_pending_snapshot_is_throttled_and_carries_barriers() {
        let recorder = Recorder::default();
        let dispatch = tracing::Dispatch::new(recorder.clone());
        let mut module = ImModule::new(Default::default())
            .with_diagnostic_session("login-a".into(), "device-a".into());
        module.state.connection_id = Some("sync-a".into());
        module.state.recovery_session.begin("actor-a");

        tracing::dispatcher::with_default(&dispatch, || {
            module.diagnostics.now_ms = 1_000;
            module.observe_sync_tick(&[], 10);
            module.diagnostics.now_ms = 1_500;
            module.observe_sync_tick(&[], 10);
            module.diagnostics.now_ms = 2_000;
            module.observe_sync_tick(&[], 10);
        });

        let pending = captured(&recorder, "sync_recovery_pending");
        assert_eq!(
            pending.len(),
            1,
            "pending snapshots are at most one per second"
        );
        let fields = &pending[0].fields;
        assert_eq!(
            fields.get("login_attempt_id").map(String::as_str),
            Some("login-a")
        );
        assert_eq!(
            fields.get("device_session_id").map(String::as_str),
            Some("device-a")
        );
        assert_eq!(
            fields.get("sync_session_id").map(String::as_str),
            Some("sync-a")
        );
        assert_eq!(
            fields.get("pending_operations").map(String::as_str),
            Some("0")
        );
        assert_eq!(fields.get("barrier_mask").map(String::as_str), Some("19"));
        assert_eq!(
            fields.get("scheduler_inflight").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            fields.get("scheduler_pending").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            fields.get("batch_persist_inflight").map(String::as_str),
            Some("0")
        );

        module.state.startup_channel_projection_ready = true;
        module.sync_timing.inventory_committed = true;
        module.state.channel_sync_batch_pending = false;
        module.state.recovery_session.phase = crate::sync_session::RecoveryPhase::Recovered;
        module.diagnostics.now_ms = 2_001;
        // 终态也必须在同一测试订阅器作用域中发出，才能核验真实导出的字段。
        tracing::dispatcher::with_default(&dispatch, || module.observe_sync_tick(&[], 10));
        let terminal = captured(&recorder, "sync_recovery_terminal");
        assert_eq!(terminal.len(), 1);
        assert_eq!(
            terminal[0].fields.get("result").map(String::as_str),
            Some("success")
        );
        assert_eq!(
            terminal[0].fields.get("barrier_mask").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            terminal[0]
                .fields
                .get("pending_operations")
                .map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn recovery_barrier_mask_has_fixed_pending_bits() {
        let channel = crate::state::ChannelId::from_str("chfixx00000000000000000001")
            .expect("test channel id");
        let mut module = ImModule::new(Default::default());
        module.state.connection_id = Some("sync-a".into());
        module.state.recovery_session.begin("actor-a");
        module.state.sync_scheduler.enqueue(channel);
        module
            .state
            .recovery_session
            .await_commit(channel, crate::state::Seq(1));
        module.state.channel_sync_persist_inflight = 1;
        let mut effects = EffectSink::new();
        module.start_increment_pull(1, vec![], &mut effects);

        let all = RECOVERY_BARRIER_STARTUP_PROJECTION
            | RECOVERY_BARRIER_INVENTORY_COMMIT
            | RECOVERY_BARRIER_INCREMENT_PULL
            | RECOVERY_BARRIER_BATCH_PERSIST
            | RECOVERY_BARRIER_BATCH_PENDING
            | RECOVERY_BARRIER_SCHEDULER
            | RECOVERY_BARRIER_PENDING_COMMITS;
        assert_eq!(module.recovery_barrier_mask(), all);

        module.state.startup_channel_projection_ready = true;
        module.sync_timing.inventory_committed = true;
        module.state.increment_pull = None;
        module.state.channel_sync_persist_inflight = 0;
        module.state.channel_sync_batch_pending = false;
        module.state.sync_scheduler.reset();
        module.state.recovery_session.pending_commits.clear();
        assert_eq!(module.recovery_barrier_mask(), 0);
    }

    #[test]
    fn recovery_pending_resets_between_epochs_and_keeps_old_corr_out() {
        let recorder = Recorder::default();
        let dispatch = tracing::Dispatch::new(recorder.clone());
        let mut module = ImModule::new(Default::default())
            .with_diagnostic_session("login-a".into(), "device-a".into());
        module.state.connection_id = Some("sync-a".into());
        module.state.recovery_session.begin("actor-a");
        let old_corr = Correlation::from_raw(11);
        module.state.corr_map.insert(
            old_corr,
            CorrelationContext::IncrementBatchPersist {
                projections: vec![],
                batch_id: None,
            },
        );

        tracing::dispatcher::with_default(&dispatch, || {
            module.diagnostics.now_ms = 1_000;
            module.observe_sync_tick(
                &[Effect::PersistAtomic {
                    corr: old_corr,
                    ops: vec![],
                }],
                10,
            );
            module.state.connection_id = Some("sync-b".into());
            module.state.recovery_session.begin("actor-b");
            module.diagnostics.now_ms = 2_000;
            module.observe_sync_tick(&[], 12);
        });

        let started = captured(&recorder, "sync_recovery_started");
        assert_eq!(started.len(), 2);
        assert_eq!(
            started[0].fields.get("sync_session_id").map(String::as_str),
            Some("sync-a")
        );
        assert_eq!(
            started[0]
                .fields
                .get("pending_operations")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            started[1].fields.get("sync_session_id").map(String::as_str),
            Some("sync-b")
        );
        assert_eq!(
            started[1]
                .fields
                .get("pending_operations")
                .map(String::as_str),
            Some("0")
        );
        let cancelled = captured(&recorder, "sync_recovery_terminal");
        assert_eq!(cancelled.len(), 1);
        assert_eq!(
            cancelled[0].fields.get("result").map(String::as_str),
            Some("cancelled")
        );
        assert_eq!(
            cancelled[0]
                .fields
                .get("sync_session_id")
                .map(String::as_str),
            Some("sync-a")
        );
    }
}
