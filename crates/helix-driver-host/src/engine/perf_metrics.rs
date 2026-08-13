use std::sync::Arc;
use std::time::{Duration, Instant};

use helix_core::tick::PortOutcome;
use helix_core::Tick;

use super::rss::process_resident_memory_bytes;
use crate::metrics::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};
use crate::owned_effect::{EffectSummary, OwnedEffect};

pub(crate) struct EngineMetricRecorder {
    sink: Arc<dyn AsyncMetricSink>,
    enabled: bool,
    port_slots: [Option<PendingPort>; MAX_PENDING_PORTS],
    pending_ports: usize,
    connected_transports: usize,
    active_command_started: Option<Instant>,
    active_from_port_reply: bool,
}

#[derive(Clone, Copy)]
struct PendingPort {
    corr: u64,
    started: Instant,
    effect_kind: &'static str,
    command_started: Option<Instant>,
}

const MAX_PENDING_PORTS: usize = 4096;
const RSS_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

pub(super) struct TickMetricContext {
    tick_kind: &'static str,
    ingress_started: Option<Instant>,
    command_started: Option<Instant>,
}

impl EngineMetricRecorder {
    pub(crate) fn new(sink: Arc<dyn AsyncMetricSink>) -> Self {
        let enabled = sink.is_enabled();
        Self {
            sink,
            enabled,
            port_slots: [None; MAX_PENDING_PORTS],
            pending_ports: 0,
            connected_transports: 0,
            active_command_started: None,
            active_from_port_reply: false,
        }
    }

    pub(crate) fn sink(&self) -> &dyn AsyncMetricSink {
        self.sink.as_ref()
    }

    /// 克隆共享 sink 给工作池，保持所有 host hop 使用同一有界出口。
    pub(crate) fn sink_handle(&self) -> Arc<dyn AsyncMetricSink> {
        Arc::clone(&self.sink)
    }

    /// 发布 Engine 已进入运行态以及固定 Port correlation 容量。
    pub(super) fn on_engine_start(&self) {
        if !self.enabled {
            return;
        }
        self.gauge(
            MetricId::EngineState,
            1.0,
            MetricLabels::one(LabelKey::Stage, "core").with(LabelKey::LifecycleState, "running"),
        );
        self.gauge(
            MetricId::PortPendingCapacity,
            MAX_PENDING_PORTS as f64,
            MetricLabels::one(LabelKey::Stage, "port_reply"),
        );
    }

    /// 发布 Engine 停止态，供 dashboard 区分无流量与进程已停。
    pub(super) fn on_engine_stopped(&self) {
        if !self.enabled {
            return;
        }
        self.gauge(
            MetricId::EngineState,
            0.0,
            MetricLabels::one(LabelKey::Stage, "core").with(LabelKey::LifecycleState, "stopped"),
        );
    }

    pub(super) fn record_queue_snapshot(&self, depth: usize, capacity: usize) {
        if !self.enabled {
            return;
        }
        let labels = MetricLabels::one(LabelKey::Stage, "core");
        self.gauge(MetricId::TickQueueDepth, depth as f64, labels);
        self.gauge(MetricId::TickQueueCapacity, capacity as f64, labels);
    }

    pub(super) fn on_tick(
        &mut self,
        tick: &Tick,
        queue_wait: Option<Duration>,
    ) -> TickMetricContext {
        if !self.enabled {
            return TickMetricContext::disabled();
        }
        self.active_command_started = None;
        self.active_from_port_reply = false;
        if let Some(queue_wait) = queue_wait {
            self.histogram(
                MetricId::TickQueueWaitSeconds,
                queue_wait.as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core")
                    .with(LabelKey::TickKind, tick_kind(tick)),
            );
        }
        let tick_labels =
            MetricLabels::one(LabelKey::Stage, "core").with(LabelKey::TickKind, tick_kind(tick));
        self.counter(MetricId::TicksTotal, 1.0, tick_labels);
        self.gauge(MetricId::TickInflight, 1.0, tick_labels);
        match tick {
            Tick::Inbound(frame) => {
                let labels =
                    MetricLabels::one(LabelKey::Stage, "ws").with(LabelKey::Direction, "inbound");
                self.counter(MetricId::WsFramesTotal, 1.0, labels);
                self.histogram(MetricId::WsFrameBytes, frame.0.len() as f64, labels);
                self.gauge(
                    MetricId::WsInboundLastSeenAgeSeconds,
                    0.0,
                    MetricLabels::one(LabelKey::Stage, "ws"),
                );
            }
            Tick::Connected(_) => {
                self.connected_transports = self.connected_transports.saturating_add(1);
                self.counter(
                    MetricId::WsConnectTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "ws").with(LabelKey::Status, "ok"),
                );
                self.record_transport_state();
            }
            Tick::Disconnected(_) => {
                self.connected_transports = self.connected_transports.saturating_sub(1);
                self.counter(
                    MetricId::WsDisconnectTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "ws"),
                );
                self.record_transport_state();
            }
            _ => {}
        }
        if let Tick::PortReply { corr, outcome } = tick {
            if let Some(pending) = self.complete_port(corr.raw()) {
                let status = match outcome {
                    PortOutcome::Ok(_) => "ok",
                    PortOutcome::Err(_) => "error",
                };
                self.histogram(
                    MetricId::PortRoundtripSeconds,
                    pending.started.elapsed().as_secs_f64(),
                    MetricLabels::one(LabelKey::Stage, "port_reply").with(LabelKey::Status, status),
                );
                self.counter(
                    MetricId::PortReplyTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "port_reply").with(LabelKey::Status, status),
                );
                if let Some(command_started) = pending.command_started {
                    let stage_metric = match pending.effect_kind {
                        "http" | "upload" | "request" => {
                            Some(MetricId::CommandToHttpResponseSeconds)
                        }
                        "persist" | "persist_atomic" => {
                            Some(MetricId::CommandToPersistReplySeconds)
                        }
                        _ => None,
                    };
                    if let Some(stage_metric) = stage_metric {
                        self.histogram(
                            stage_metric,
                            command_started.elapsed().as_secs_f64(),
                            MetricLabels::one(LabelKey::Stage, "command_lifecycle")
                                .with(LabelKey::Status, status),
                        );
                    }
                    self.active_command_started = Some(command_started);
                    self.active_from_port_reply = true;
                }
            } else {
                self.counter(
                    MetricId::PortReplyOrphanTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "port_reply"),
                );
                self.counter(
                    MetricId::UnknownPortReplyTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "port_reply"),
                );
            }
            self.record_pending_ports();
        }
        TickMetricContext {
            tick_kind: tick_kind(tick),
            ingress_started: matches!(tick, Tick::Inbound(_)).then(Instant::now),
            command_started: matches!(tick, Tick::Command(_)).then(Instant::now),
        }
        .tap_command(self)
    }

    pub(super) fn on_step_error(&self, context: &TickMetricContext) {
        if !self.enabled {
            return;
        }
        self.counter(
            MetricId::ErrorsTotal,
            1.0,
            MetricLabels::one(LabelKey::Stage, "core")
                .with(LabelKey::TickKind, context.tick_kind)
                .with(LabelKey::ErrorKind, "step_failed"),
        );
        self.counter(
            MetricId::CoreStepErrorsTotal,
            1.0,
            MetricLabels::one(LabelKey::Stage, "core").with(LabelKey::TickKind, context.tick_kind),
        );
        self.gauge(
            MetricId::TickInflight,
            0.0,
            MetricLabels::one(LabelKey::Stage, "core").with(LabelKey::TickKind, context.tick_kind),
        );
        self.counter(
            MetricId::OperationsTotal,
            1.0,
            MetricLabels::one(LabelKey::Stage, "core")
                .with(LabelKey::TickKind, context.tick_kind)
                .with(LabelKey::Operation, "step")
                .with(LabelKey::Status, "error"),
        );
    }

    pub(super) fn on_step_complete(
        &mut self,
        context: &TickMetricContext,
        started: Option<Instant>,
        summary: EffectSummary,
    ) {
        let Some(started) = started else {
            return;
        };
        let labels = MetricLabels::one(LabelKey::Layer, "L2")
            .with(LabelKey::Stage, "core")
            .with(LabelKey::TickKind, context.tick_kind)
            .with(LabelKey::Status, "ok");
        self.histogram(
            MetricId::CoreStepDurationSeconds,
            started.elapsed().as_secs_f64(),
            labels,
        );
        self.counter(
            MetricId::OperationsTotal,
            1.0,
            labels.with(LabelKey::Operation, "step"),
        );
        self.record_effect_batch(summary, context.tick_kind);
        if summary.count == 0 {
            self.counter(
                MetricId::CoreEmptyEffectTotal,
                1.0,
                MetricLabels::one(LabelKey::Stage, "core")
                    .with(LabelKey::TickKind, context.tick_kind),
            );
        }
        self.gauge(
            MetricId::TickInflight,
            0.0,
            MetricLabels::one(LabelKey::Stage, "core").with(LabelKey::TickKind, context.tick_kind),
        );
        if let Some(started) = context.ingress_started {
            self.histogram(
                MetricId::WsIngressToEffectSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core"),
            );
        }
    }

    /// 闭合本轮 dispatch，并清除仅在当前 Tick 有效的 command 根上下文。
    pub(super) fn on_dispatch_complete(&mut self, context: &TickMetricContext) {
        if !self.enabled {
            return;
        }
        if let Some(started) = context.ingress_started {
            self.histogram(
                MetricId::WsIngressToEventSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "event"),
            );
        }
        self.active_command_started = None;
        self.active_from_port_reply = false;
    }

    pub(crate) fn on_effect_dispatch(&mut self, effect: &OwnedEffect) {
        if let Some(command_started) = self.active_command_started {
            match effect {
                OwnedEffect::Http { .. } | OwnedEffect::UploadFile { .. } => self.histogram(
                    MetricId::CommandToHttpDispatchSeconds,
                    command_started.elapsed().as_secs_f64(),
                    MetricLabels::one(LabelKey::Stage, "command_lifecycle"),
                ),
                OwnedEffect::Emit { .. } => self.histogram(
                    if self.active_from_port_reply {
                        MetricId::CommandToProjectionEmitSeconds
                    } else {
                        MetricId::CommandToImmediateProjectionSeconds
                    },
                    command_started.elapsed().as_secs_f64(),
                    MetricLabels::one(LabelKey::Stage, "command_lifecycle"),
                ),
                _ => {}
            }
        }
        self.counter(
            MetricId::EffectsTotal,
            1.0,
            MetricLabels::one(LabelKey::Stage, "effect")
                .with(LabelKey::EffectKind, effect_kind(effect)),
        );
        let corr = match effect {
            OwnedEffect::Persist { corr, .. }
            | OwnedEffect::PersistAtomic { corr, .. }
            | OwnedEffect::Http { corr, .. }
            | OwnedEffect::UploadFile { corr, .. }
            | OwnedEffect::Request { corr, .. } => Some(corr.raw()),
            _ => None,
        };
        if let Some(corr) = corr {
            self.remember_port_start(corr, effect_kind(effect));
        }
    }

    /// 在固定槽内保存 Port 起点及可选 command 根，容量冲突时显式计数。
    fn remember_port_start(&mut self, corr: u64, effect_kind: &'static str) {
        let slot = corr as usize % MAX_PENDING_PORTS;
        if self.port_slots[slot].is_some() {
            self.counter(
                MetricId::PortCorrelationCollisionTotal,
                1.0,
                MetricLabels::one(LabelKey::Stage, "port_reply"),
            );
        } else {
            self.pending_ports = self.pending_ports.saturating_add(1);
        }
        self.port_slots[slot] = Some(PendingPort {
            corr,
            started: Instant::now(),
            effect_kind,
            command_started: self.active_command_started,
        });
        self.record_pending_ports();
    }

    /// 按 corr O(1) 取回并释放 Port 槽；不匹配保持 orphan 语义。
    fn complete_port(&mut self, corr: u64) -> Option<PendingPort> {
        let slot = corr as usize % MAX_PENDING_PORTS;
        match self.port_slots[slot] {
            Some(pending) if pending.corr == corr => {
                self.port_slots[slot] = None;
                self.pending_ports = self.pending_ports.saturating_sub(1);
                Some(pending)
            }
            _ => None,
        }
    }

    /// 仅在指标启用时读取单调时钟，disabled 热路径保持零时钟开销。
    pub(crate) fn start_timer(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    /// 记录一条 feedback 在独立回灌 channel 中的真实驻留时间与队列快照。
    pub(super) fn on_feedback_dequeued(&self, tick: &Tick, wait: Duration, depth: usize) {
        if !self.enabled {
            return;
        }
        self.histogram(
            MetricId::PortReplyQueueWaitSeconds,
            wait.as_secs_f64(),
            MetricLabels::one(LabelKey::Stage, "port_reply")
                .with(LabelKey::TickKind, tick_kind(tick)),
        );
        self.gauge(
            MetricId::PortReplyQueueDepth,
            depth as f64,
            MetricLabels::one(LabelKey::Stage, "port_reply"),
        );
    }

    /// 标记 PortReply 洪峰下的 ingress 公平闸真实触发。
    pub(super) fn on_reply_fairness_forced(&self) {
        if self.enabled {
            self.counter(
                MetricId::ReplyFairnessForcedTotal,
                1.0,
                MetricLabels::one(LabelKey::Stage, "port_reply"),
            );
        }
    }

    /// 闭合 Engine 单轮耗时，覆盖 step、dispatch 与 event flush。
    pub(super) fn on_loop_iteration(&self, started: Option<Instant>) {
        if let Some(started) = started {
            self.histogram(
                MetricId::EngineLoopIterationSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core"),
            );
        }
    }

    pub(super) fn spawn_rss_sampler(&self) -> Option<tokio::task::JoinHandle<()>> {
        if !self.enabled {
            return None;
        }
        let sink = Arc::clone(&self.sink);
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(RSS_SAMPLE_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let sample = tokio::task::spawn_blocking(process_resident_memory_bytes)
                    .await
                    .ok()
                    .flatten();
                if let Some(rss_bytes) = sample {
                    let _ = sink.try_record(MetricEvent::gauge(
                        MetricId::ProcessResidentMemoryBytes,
                        rss_bytes as f64,
                        MetricLabels::one(LabelKey::Stage, "telemetry"),
                    ));
                }
            }
        }))
    }

    pub(super) fn on_shutdown(&self, started: Option<Instant>) {
        if let Some(started) = started {
            self.histogram(
                MetricId::ShutdownDrainSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core"),
            );
        }
    }

    fn record_effect_batch(&self, summary: EffectSummary, tick_kind: &'static str) {
        let labels =
            MetricLabels::one(LabelKey::Stage, "effect").with(LabelKey::TickKind, tick_kind);
        let count = summary.count;
        let bytes = summary.bytes;
        self.histogram(MetricId::EffectsPerTick, count as f64, labels);
        self.histogram(MetricId::EffectBytesPerTick, bytes as f64, labels);
        self.histogram(MetricId::EffectAmplificationRatio, count as f64, labels);
        self.histogram(
            MetricId::EventBatchSize,
            summary.event_count as f64,
            MetricLabels::one(LabelKey::Stage, "event"),
        );
    }

    /// O(1) 发布固定 correlation 表当前占用，不扫描热路径槽位。
    fn record_pending_ports(&self) {
        self.gauge(
            MetricId::PortPending,
            self.pending_ports as f64,
            MetricLabels::one(LabelKey::Stage, "port_reply"),
        );
    }

    /// 发布 transport 数与“至少一条连接可用”状态，不使用 transport id 标签。
    fn record_transport_state(&self) {
        let labels = MetricLabels::one(LabelKey::Stage, "ws");
        self.gauge(
            MetricId::TransportCount,
            self.connected_transports as f64,
            labels,
        );
        self.gauge(
            MetricId::WsConnectionState,
            f64::from(self.connected_transports > 0),
            labels,
        );
    }

    fn counter(&self, id: MetricId, value: f64, labels: MetricLabels) {
        let _ = self
            .sink
            .try_record(MetricEvent::counter(id, value, labels));
    }

    fn gauge(&self, id: MetricId, value: f64, labels: MetricLabels) {
        let _ = self.sink.try_record(MetricEvent::gauge(id, value, labels));
    }

    fn histogram(&self, id: MetricId, value: f64, labels: MetricLabels) {
        let _ = self
            .sink
            .try_record(MetricEvent::histogram(id, value, labels));
    }
}

impl TickMetricContext {
    /// 把 command 根起点同步进跨 Tick correlation recorder，再返回原 context。
    fn tap_command(self, recorder: &mut EngineMetricRecorder) -> Self {
        if self.command_started.is_some() {
            recorder.active_command_started = self.command_started;
        }
        self
    }

    fn disabled() -> Self {
        Self {
            tick_kind: "disabled",
            ingress_started: None,
            command_started: None,
        }
    }
}

fn tick_kind(tick: &Tick) -> &'static str {
    match tick {
        Tick::Inbound(_) => "inbound",
        Tick::PortReply { .. } => "port_reply",
        Tick::PortProgress { .. } => "port_progress",
        Tick::Timer(_) => "timer",
        Tick::Command(_) => "command",
        Tick::Connected(_) => "connected",
        Tick::Disconnected(_) => "disconnected",
    }
}

/// 将 Effect enum 映射成冻结的低基数标签。
fn effect_kind(effect: &OwnedEffect) -> &'static str {
    match effect {
        OwnedEffect::Persist { .. } => "persist",
        OwnedEffect::PersistAtomic { .. } => "persist_atomic",
        OwnedEffect::PersistFire { .. } => "persist_fire",
        OwnedEffect::Http { .. } => "http",
        OwnedEffect::HttpFire { .. } => "http_fire",
        OwnedEffect::UploadFile { .. } => "upload",
        OwnedEffect::Send { .. } => "send",
        OwnedEffect::Request { .. } => "request",
        OwnedEffect::Emit { .. } => "emit",
        OwnedEffect::ScheduleTimer { .. } => "schedule_timer",
        OwnedEffect::CancelTimer { .. } => "cancel_timer",
    }
}

#[cfg(test)]
#[path = "perf_metrics_tests.rs"]
mod tests;
