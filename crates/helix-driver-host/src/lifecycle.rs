//! Host 侧 lifecycle correlation 合同。
//!
//! 该模块只保存显式、不可变的 tick 上下文和有限的父子关联；它不依赖
//! task-local active carrier，也不把网络 exporter 的背压带回事件泵。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::owned_effect::OwnedEffect;
use crate::trace::TraceCarrier;
use helix_core::effect::{Correlation, TimerId};
use helix_core::Tick;

/// 每个 Tick 及其局部效果树使用的单调标识。
pub type TickId = u64;

/// lifecycle 的五个固定阶段；局部 HTTP/WS/Storage 效果均直接挂在 T1 根上。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleStage {
    T1,
    T2,
    T3,
    T4,
    T5,
}

impl LifecycleStage {
    /// 返回稳定的协议标签，避免把 Rust 调试格式写入遥测。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T1 => "T1",
            Self::T2 => "T2",
            Self::T3 => "T3",
            Self::T4 => "T4",
            Self::T5 => "T5",
        }
    }

    /// 将阶段映射到固定数组槽位，保持 O(1) 读取。
    const fn index(self) -> usize {
        match self {
            Self::T1 => 0,
            Self::T2 => 1,
            Self::T3 => 2,
            Self::T4 => 3,
            Self::T5 => 4,
        }
    }
}

/// 可选端口能力；能力缺失只能报告 not_applicable，不能伪造网络 span。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleCapability {
    Http,
    Ws,
    Persist,
    Effect,
}

impl LifecycleCapability {
    /// 返回稳定的协议标签，供属性和合同测试使用。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Ws => "ws",
            Self::Persist => "persist",
            Self::Effect => "effect",
        }
    }

    /// 将能力映射到固定数组槽位，保持 O(1) 读取。
    const fn index(self) -> usize {
        match self {
            Self::Http => 0,
            Self::Ws => 1,
            Self::Persist => 2,
            Self::Effect => 3,
        }
    }
}

/// lifecycle 观测状态；Skipped 与 NotApplicable 明确区分未执行和不适用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifecycleStatus {
    Pending,
    Started,
    Ok,
    Error,
    Skipped,
    NotApplicable,
}

impl LifecycleStatus {
    /// 返回稳定的小写状态标签，便于跨端对账。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Started => "started",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// `LifecycleState` 是状态合同的兼容别名，便于调用方按 state 语义读取。
pub type LifecycleState = LifecycleStatus;

/// 一次 Tick 的不可变 correlation 上下文。
///
/// `with_*` 方法均返回新值；调用方可以在 T1/T2/T3/T4/T5 间传递上下文，
/// 不需要全局 carrier 或 task-local 状态。`current_stage` 的父阶段恒为 T1，
/// 使同一 Tick 内的本地子树不会意外嵌套到前一个效果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleContext {
    tick_id: TickId,
    parent_tick_id: Option<TickId>,
    carrier: Option<TraceCarrier>,
    span_parent: Option<TraceCarrier>,
    current_stage: LifecycleStage,
    stage_status: [LifecycleStatus; 5],
    capabilities: [bool; 4],
    capability_status: [LifecycleStatus; 4],
}

impl LifecycleContext {
    /// 创建 T1 根上下文；网络能力默认关闭，避免缺能力时伪造 network span。
    pub fn new(
        tick_id: TickId,
        parent_tick_id: Option<TickId>,
        carrier: Option<TraceCarrier>,
    ) -> Self {
        Self {
            tick_id,
            parent_tick_id,
            carrier,
            span_parent: None,
            current_stage: LifecycleStage::T1,
            stage_status: [
                LifecycleStatus::Started,
                LifecycleStatus::Skipped,
                LifecycleStatus::Skipped,
                LifecycleStatus::Skipped,
                LifecycleStatus::Skipped,
            ],
            capabilities: [false; 4],
            capability_status: [LifecycleStatus::NotApplicable; 4],
        }
    }

    /// 创建带父 Tick 的 T1 上下文，作为 engine 的语义化构造别名。
    pub fn root(
        tick_id: TickId,
        parent_tick_id: Option<TickId>,
        carrier: Option<TraceCarrier>,
    ) -> Self {
        Self::new(tick_id, parent_tick_id, carrier)
    }

    /// 返回当前 Tick 的独立 ID。
    pub const fn tick_id(&self) -> TickId {
        self.tick_id
    }

    /// 返回上游 Tick ID；没有可证明的上游时保持 None。
    pub const fn parent_tick_id(&self) -> Option<TickId> {
        self.parent_tick_id
    }

    /// 返回显式 carrier 的只读引用。
    pub fn carrier(&self) -> Option<&TraceCarrier> {
        self.carrier.as_ref()
    }

    /// 返回当前 Tick 内部 T1 span 的显式 parent carrier；没有 OTel 时保持 None。
    pub fn span_parent(&self) -> Option<&TraceCarrier> {
        self.span_parent.as_ref()
    }

    /// 返回新的上下文并绑定本 Tick 的内部 span parent，不写入全局或 task-local 状态。
    pub fn with_span_parent(&self, span_parent: Option<TraceCarrier>) -> Self {
        let mut next = self.clone();
        next.span_parent = span_parent;
        next
    }

    /// 选择异步子节点的显式 parent；内部 T1 优先，外部 W3C carrier 仅作根回退。
    pub fn otel_parent(&self) -> Option<&TraceCarrier> {
        self.span_parent.as_ref().or(self.carrier.as_ref())
    }

    /// 返回当前生命周期阶段。
    pub const fn current_stage(&self) -> LifecycleStage {
        self.current_stage
    }

    /// 返回当前阶段在 T1 根下的挂载点。
    pub const fn parent_stage(&self) -> LifecycleStage {
        LifecycleStage::T1
    }

    /// 读取五阶段状态，不会修改上下文。
    pub const fn stage_status(&self, stage: LifecycleStage) -> LifecycleStatus {
        self.stage_status[stage.index()]
    }

    /// 设置阶段状态并返回新上下文。
    pub fn with_stage_status(&self, stage: LifecycleStage, status: LifecycleStatus) -> Self {
        let mut next = self.clone();
        next.stage_status[stage.index()] = status;
        next.current_stage = stage;
        next
    }

    /// 读取能力是否存在。
    pub const fn has_capability(&self, capability: LifecycleCapability) -> bool {
        self.capabilities[capability.index()]
    }

    /// 读取能力状态；未提供能力时固定返回 NotApplicable。
    pub const fn capability_status(&self, capability: LifecycleCapability) -> LifecycleStatus {
        self.capability_status[capability.index()]
    }

    /// 设置能力存在性；关闭能力时状态自动收敛到 NotApplicable。
    pub fn with_capability(&self, capability: LifecycleCapability, enabled: bool) -> Self {
        let mut next = self.clone();
        let index = capability.index();
        next.capabilities[index] = enabled;
        next.capability_status[index] = if enabled {
            LifecycleStatus::Skipped
        } else {
            LifecycleStatus::NotApplicable
        };
        next
    }

    /// 设置能力状态并返回新上下文；NotApplicable 会同时关闭该能力。
    pub fn with_capability_status(
        &self,
        capability: LifecycleCapability,
        status: LifecycleStatus,
    ) -> Self {
        let mut next = self.clone();
        let index = capability.index();
        next.capabilities[index] = status != LifecycleStatus::NotApplicable;
        next.capability_status[index] = status;
        next
    }

    /// 构造同一 Tick 的局部阶段上下文，所有局部节点直接挂到 T1。
    pub fn local_stage(&self, stage: LifecycleStage) -> Self {
        let mut next = self.clone();
        next.current_stage = stage;
        next
    }
}

impl Default for LifecycleContext {
    /// 创建 synthetic root，供无 Tick 的测试/启动 effect 使用。
    fn default() -> Self {
        Self::new(0, None, None)
    }
}

/// 生命周期状态出口的结构化记录；不携带请求正文或用户数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleObservation {
    pub tick_id: TickId,
    pub parent_tick_id: Option<TickId>,
    pub stage: LifecycleStage,
    pub capability: Option<LifecycleCapability>,
    pub status: LifecycleStatus,
    /// 固定诊断原因；不承载请求正文或动态错误文本。
    pub reason: Option<&'static str>,
}

/// lifecycle 诊断出口的固定有界容量。
pub const LIFECYCLE_TRACE_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct LifecycleTraceStats {
    dropped: Arc<AtomicU64>,
}

impl LifecycleTraceStats {
    /// 返回因队列满或接收端关闭而丢弃的观测数。
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// 不参与业务正确性的有界 lifecycle 观测 sink。
#[derive(Clone, Debug)]
pub struct LifecycleTraceSink {
    tx: mpsc::Sender<LifecycleObservation>,
    stats: LifecycleTraceStats,
}

impl LifecycleTraceSink {
    /// 创建固定容量的生命周期观测通道。
    pub fn channel() -> (Self, mpsc::Receiver<LifecycleObservation>) {
        let (tx, rx) = mpsc::channel(LIFECYCLE_TRACE_QUEUE_CAPACITY);
        (
            Self {
                tx,
                stats: LifecycleTraceStats::default(),
            },
            rx,
        )
    }

    /// 非阻塞投递生命周期观测；队满时丢弃并累计，不影响 engine。
    pub fn try_emit(&self, observation: LifecycleObservation) -> bool {
        match self.tx.try_send(observation) {
            Ok(()) => true,
            Err(_) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// 返回共享的丢弃统计。
    pub fn stats(&self) -> LifecycleTraceStats {
        self.stats.clone()
    }
}

/// 父 Tick 关联表容量上限；超限仅淘汰诊断链接，不影响业务回灌。
pub const LIFECYCLE_LINK_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LinkKey {
    Correlation(u64),
    Timer(u64),
}

#[derive(Clone, Debug)]
struct LifecycleLink {
    generation: u64,
    parent_tick_id: TickId,
    carrier: Option<TraceCarrier>,
    span_parent: Option<TraceCarrier>,
}

#[derive(Default)]
struct TrackerState {
    links: HashMap<LinkKey, LifecycleLink>,
    order: VecDeque<(LinkKey, u64)>,
    next_generation: u64,
}

/// engine 私有、有限容量的父 Tick 关联表。
///
/// 关联表只保存 correlation/timer 的标量和 bounded carrier，过载时淘汰最旧项；
/// 它不是 active carrier，也不承担业务正确性。
#[derive(Clone, Default)]
pub struct LifecycleTracker {
    next_tick_id: Arc<AtomicU64>,
    state: Arc<Mutex<TrackerState>>,
}

impl LifecycleTracker {
    /// 分配下一个独立 Tick ID，0 保留给 engine start 的 synthetic root。
    pub fn next_tick_id(&self) -> TickId {
        self.next_tick_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }

    /// 解析 Tick 的上游父关联，并在终态 Tick 到达后释放一次性链接。
    pub fn parent_for_tick(
        &self,
        tick: &Tick,
    ) -> (Option<TickId>, Option<TraceCarrier>, Option<TraceCarrier>) {
        let (key, terminal) = match tick {
            Tick::PortReply { corr, .. } => (Some(LinkKey::Correlation(corr.raw())), true),
            Tick::PortProgress { corr, .. } => (Some(LinkKey::Correlation(corr.raw())), false),
            Tick::Timer(id) => (Some(LinkKey::Timer(id.raw())), true),
            _ => (None, false),
        };
        let Some(key) = key else {
            return (None, None, None);
        };
        let mut state = self.state.lock();
        let link = if terminal {
            state.links.remove(&key)
        } else {
            state.links.get(&key).cloned()
        };
        link.map_or((None, None, None), |link| {
            (Some(link.parent_tick_id), link.carrier, link.span_parent)
        })
    }

    /// 记录效果产生的未来 Tick 关联；超出容量时淘汰最旧诊断链接。
    pub fn remember_effect(&self, effect: &OwnedEffect, context: &LifecycleContext) {
        let (key, carrier, span_parent) = match effect {
            OwnedEffect::Persist { corr, .. }
            | OwnedEffect::PersistAtomic { corr, .. }
            | OwnedEffect::Http { corr, .. }
            | OwnedEffect::UploadFile { corr, .. }
            | OwnedEffect::Request { corr, .. } => (
                Some(LinkKey::Correlation(corr.raw())),
                context.carrier.clone(),
                context.span_parent.clone(),
            ),
            OwnedEffect::ScheduleTimer { id, .. } => (
                Some(LinkKey::Timer(id.raw())),
                context.carrier.clone(),
                context.span_parent.clone(),
            ),
            OwnedEffect::CancelTimer { id } => {
                self.forget(LinkKey::Timer(id.raw()));
                (None, None, None)
            }
            _ => (None, None, None),
        };
        let Some(key) = key else {
            return;
        };
        let mut state = self.state.lock();
        let generation = state
            .links
            .get(&key)
            .map(|link| link.generation)
            .unwrap_or_else(|| {
                state.next_generation = state.next_generation.wrapping_add(1);
                let generation = state.next_generation;
                state.order.push_back((key, generation));
                generation
            });
        state.links.insert(
            key,
            LifecycleLink {
                generation,
                parent_tick_id: context.tick_id,
                carrier,
                span_parent,
            },
        );
        compact_tracker_order_if_needed(&mut state);
        while state.links.len() > LIFECYCLE_LINK_CAPACITY {
            let Some((oldest, generation)) = state.order.pop_front() else {
                break;
            };
            if state
                .links
                .get(&oldest)
                .is_some_and(|link| link.generation == generation)
            {
                state.links.remove(&oldest);
            }
        }
    }

    /// 删除被取消的 timer 或替换前的旧关联。
    fn forget(&self, key: LinkKey) {
        let mut state = self.state.lock();
        state.links.remove(&key);
    }
}

/// 偶发压缩已消费的顺序标记，保证 stale tombstone 不会无界增长；正常热路径不扫描全表。
fn compact_tracker_order_if_needed(state: &mut TrackerState) {
    const COMPACTION_FACTOR: usize = 4;
    if state.order.len() <= LIFECYCLE_LINK_CAPACITY * COMPACTION_FACTOR {
        return;
    }
    state.order.retain(|(key, generation)| {
        state
            .links
            .get(key)
            .is_some_and(|link| link.generation == *generation)
    });
}

/// 保留端口类型在该模块的可见性，方便调用方构造矩阵测试。
pub type LifecycleCorrelation = Correlation;
/// 保留 timer 类型在该模块的可见性，方便调用方构造矩阵测试。
pub type LifecycleTimer = TimerId;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use helix_core::effect::HttpRequest;

    #[test]
    fn context_tree_has_independent_ticks_and_t1_rooted_local_stages() {
        let root = LifecycleContext::new(7, None, None)
            .with_capability(LifecycleCapability::Ws, false)
            .with_capability(LifecycleCapability::Http, true);
        let local = root
            .local_stage(LifecycleStage::T4)
            .with_stage_status(LifecycleStage::T4, LifecycleStatus::Started);
        let next = LifecycleContext::new(8, Some(7), None);

        assert_eq!(root.tick_id(), 7);
        assert_eq!(next.tick_id(), 8);
        assert_eq!(next.parent_tick_id(), Some(7));
        assert_eq!(local.parent_stage(), LifecycleStage::T1);
        assert_eq!(
            local.stage_status(LifecycleStage::T4),
            LifecycleStatus::Started
        );
        assert_eq!(
            root.capability_status(LifecycleCapability::Ws),
            LifecycleStatus::NotApplicable
        );
        assert_eq!(
            root.capability_status(LifecycleCapability::Http),
            LifecycleStatus::Skipped
        );
    }

    #[test]
    fn tracker_keeps_parent_and_carrier_isolated_across_correlations() {
        let tracker = LifecycleTracker::default();
        let carrier = TraceCarrier::from_headers(&[(
            "traceparent".to_string(),
            "00-00000000000000000000000000000001-0000000000000002-01".to_string(),
        )]);
        let first = LifecycleContext::new(11, None, carrier);
        let second = LifecycleContext::new(12, None, None);
        let first_effect = OwnedEffect::Http {
            corr: Correlation::from_raw(1),
            req: HttpRequest {
                method: "GET".to_string(),
                url: "https://example.test".to_string(),
                headers: Vec::new(),
                body: None,
            },
        };
        let second_effect = OwnedEffect::Http {
            corr: Correlation::from_raw(2),
            req: HttpRequest {
                method: "GET".to_string(),
                url: "https://example.test".to_string(),
                headers: Vec::new(),
                body: None,
            },
        };
        tracker.remember_effect(&first_effect, &first);
        tracker.remember_effect(&second_effect, &second);
        let (first_parent, first_carrier, first_span_parent) =
            tracker.parent_for_tick(&Tick::PortReply {
                corr: Correlation::from_raw(1),
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    Bytes::new(),
                )),
            });
        let (second_parent, second_carrier, second_span_parent) =
            tracker.parent_for_tick(&Tick::PortReply {
                corr: Correlation::from_raw(2),
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    Bytes::new(),
                )),
            });
        assert_eq!(first_parent, Some(11));
        assert_eq!(second_parent, Some(12));
        assert!(first_carrier.is_some());
        assert!(second_carrier.is_none());
        assert!(first_span_parent.is_none());
        assert!(second_span_parent.is_none());
    }

    #[test]
    fn lifecycle_sink_is_non_blocking_and_bounded() {
        let (sink, mut rx) = LifecycleTraceSink::channel();
        let context = LifecycleContext::new(1, None, None);
        for _ in 0..LIFECYCLE_TRACE_QUEUE_CAPACITY {
            assert!(sink.try_emit(LifecycleObservation {
                tick_id: context.tick_id(),
                parent_tick_id: context.parent_tick_id(),
                stage: LifecycleStage::T1,
                capability: None,
                status: LifecycleStatus::Started,
                reason: None,
            }));
        }
        assert!(!sink.try_emit(LifecycleObservation {
            tick_id: 1,
            parent_tick_id: None,
            stage: LifecycleStage::T4,
            capability: Some(LifecycleCapability::Http),
            status: LifecycleStatus::NotApplicable,
            reason: Some("transport_absent"),
        }));
        assert_eq!(sink.stats().dropped_count(), 1);
        rx.close();
    }

    #[test]
    fn capability_matrix_marks_http_only_ws_only_and_no_network_explicitly() {
        let no_network = LifecycleContext::new(1, None, None);
        let http_only = no_network.with_capability(LifecycleCapability::Http, true);
        let ws_only = no_network.with_capability(LifecycleCapability::Ws, true);

        assert_eq!(
            no_network.capability_status(LifecycleCapability::Http),
            LifecycleStatus::NotApplicable
        );
        assert_eq!(
            no_network.capability_status(LifecycleCapability::Ws),
            LifecycleStatus::NotApplicable
        );
        assert_eq!(
            http_only.capability_status(LifecycleCapability::Http),
            LifecycleStatus::Skipped
        );
        assert_eq!(
            http_only.capability_status(LifecycleCapability::Ws),
            LifecycleStatus::NotApplicable
        );
        assert_eq!(
            ws_only.capability_status(LifecycleCapability::Ws),
            LifecycleStatus::Skipped
        );
        assert_eq!(
            ws_only.capability_status(LifecycleCapability::Http),
            LifecycleStatus::NotApplicable
        );
    }

    #[test]
    fn concurrent_trackers_keep_parent_links_isolated() {
        let tracker = LifecycleTracker::default();
        let first_tracker = tracker.clone();
        let second_tracker = tracker.clone();
        let first = std::thread::spawn(move || {
            let context = LifecycleContext::new(101, None, None);
            let effect = OwnedEffect::Http {
                corr: Correlation::from_raw(101),
                req: HttpRequest {
                    method: "GET".to_string(),
                    url: "https://example.test/one".to_string(),
                    headers: Vec::new(),
                    body: None,
                },
            };
            first_tracker.remember_effect(&effect, &context);
        });
        let second = std::thread::spawn(move || {
            let context = LifecycleContext::new(202, None, None);
            let effect = OwnedEffect::Http {
                corr: Correlation::from_raw(202),
                req: HttpRequest {
                    method: "GET".to_string(),
                    url: "https://example.test/two".to_string(),
                    headers: Vec::new(),
                    body: None,
                },
            };
            second_tracker.remember_effect(&effect, &context);
        });
        assert!(first.join().is_ok());
        assert!(second.join().is_ok());

        let (first_parent, _, _) = tracker.parent_for_tick(&Tick::PortReply {
            corr: Correlation::from_raw(101),
            outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Timeout),
        });
        let (second_parent, _, _) = tracker.parent_for_tick(&Tick::PortReply {
            corr: Correlation::from_raw(202),
            outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Timeout),
        });
        assert_eq!(first_parent, Some(101));
        assert_eq!(second_parent, Some(202));
    }
}
