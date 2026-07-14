use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use helix_core::Tick;

use crate::metrics::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};
use crate::owned_effect::OwnedEffect;

pub(super) struct EngineMetricRecorder {
    sink: Arc<dyn AsyncMetricSink>,
    enabled: bool,
    port_started: HashMap<u64, Instant>,
    port_order: VecDeque<u64>,
}

pub(super) struct TickMetricContext {
    tick_kind: &'static str,
    ingress_started: Option<Instant>,
    command_started: Option<Instant>,
}

impl EngineMetricRecorder {
    pub(super) fn new(sink: Arc<dyn AsyncMetricSink>) -> Self {
        let enabled = sink.is_enabled();
        Self {
            sink,
            enabled,
            port_started: HashMap::new(),
            port_order: VecDeque::new(),
        }
    }

    pub(super) fn sink(&self) -> &dyn AsyncMetricSink {
        self.sink.as_ref()
    }

    pub(super) fn record_queue_snapshot(&self, depth: usize, capacity: usize) {
        if !self.enabled {
            return;
        }
        let labels = MetricLabels::one(LabelKey::Stage, "tick_queue");
        self.gauge(MetricId::TickQueueDepth, depth as f64, labels);
        self.gauge(MetricId::TickQueueCapacity, capacity as f64, labels);
    }

    pub(super) fn on_tick(&mut self, tick: &Tick) -> TickMetricContext {
        if !self.enabled {
            return TickMetricContext::disabled();
        }
        if let Tick::PortReply { corr, .. } = tick {
            if let Some(started) = self.port_started.remove(&corr.raw()) {
                self.histogram(
                    MetricId::PortRoundtripSeconds,
                    started.elapsed().as_secs_f64(),
                    MetricLabels::one(LabelKey::Stage, "port_reply"),
                );
            }
        }
        TickMetricContext {
            tick_kind: tick_kind(tick),
            ingress_started: matches!(tick, Tick::Inbound(_)).then(Instant::now),
            command_started: matches!(tick, Tick::Command(_)).then(Instant::now),
        }
    }

    pub(super) fn on_step_error(&self, context: &TickMetricContext) {
        if !self.enabled {
            return;
        }
        self.counter(
            MetricId::ErrorsTotal,
            1.0,
            MetricLabels::one(LabelKey::Stage, "core_step")
                .with(LabelKey::TickKind, context.tick_kind)
                .with(LabelKey::ErrorKind, "step_failed"),
        );
    }

    pub(super) fn on_step_complete(
        &mut self,
        context: &TickMetricContext,
        started: Option<Instant>,
        effects: &[OwnedEffect],
    ) {
        let Some(started) = started else {
            return;
        };
        let labels = MetricLabels::one(LabelKey::Layer, "L2")
            .with(LabelKey::Stage, "core_step")
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
        self.record_effect_batch(effects, context.tick_kind);
        self.remember_port_starts(effects);
        if let Some(started) = context.ingress_started {
            self.histogram(
                MetricId::WsIngressToEffectSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core_effect"),
            );
        }
    }

    pub(super) fn on_dispatch_complete(&self, context: &TickMetricContext) {
        if !self.enabled {
            return;
        }
        if let Some(started) = context.ingress_started {
            self.histogram(
                MetricId::WsIngressToEventSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "event_flush"),
            );
        }
        if let Some(started) = context.command_started {
            self.histogram(
                MetricId::CommandToProjectionSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "event_flush"),
            );
        }
    }

    fn remember_port_starts(&mut self, effects: &[OwnedEffect]) {
        for effect in effects {
            let corr = match effect {
                OwnedEffect::Persist { corr, .. }
                | OwnedEffect::Http { corr, .. }
                | OwnedEffect::UploadFile { corr, .. }
                | OwnedEffect::Request { corr, .. } => Some(corr.raw()),
                _ => None,
            };
            if let Some(corr) = corr {
                self.remember_port_start(corr);
            }
        }
    }

    fn remember_port_start(&mut self, corr: u64) {
        const MAX_PENDING: usize = 4096;
        if self.port_started.contains_key(&corr) {
            self.port_started.insert(corr, Instant::now());
            return;
        }
        while self.port_started.len() >= MAX_PENDING {
            let Some(evicted) = self.port_order.pop_front() else {
                break;
            };
            if self.port_started.remove(&evicted).is_some() {
                break;
            }
        }
        self.port_started.insert(corr, Instant::now());
        self.port_order.push_back(corr);
    }

    pub(super) fn start_timer(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    pub(super) fn on_shutdown(&self, started: Option<Instant>) {
        if self.enabled {
            if let Some(rss_bytes) = process_resident_memory_bytes() {
                self.gauge(
                    MetricId::ProcessResidentMemoryBytes,
                    rss_bytes as f64,
                    MetricLabels::one(LabelKey::Stage, "engine_shutdown"),
                );
            }
        }
        if let Some(started) = started {
            self.histogram(
                MetricId::ShutdownDrainSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "engine_shutdown"),
            );
        }
    }

    fn record_effect_batch(&self, effects: &[OwnedEffect], tick_kind: &'static str) {
        let labels = MetricLabels::one(LabelKey::Stage, "effect_produce")
            .with(LabelKey::TickKind, tick_kind);
        let count = effects.len();
        let bytes = effects.iter().map(effect_bytes).sum::<usize>();
        self.histogram(MetricId::EffectsPerTick, count as f64, labels);
        self.histogram(MetricId::EffectBytesPerTick, bytes as f64, labels);
        self.histogram(MetricId::EffectAmplificationRatio, count as f64, labels);
        let event_count = effects
            .iter()
            .filter(|effect| matches!(effect, OwnedEffect::Emit { .. }))
            .count();
        self.histogram(
            MetricId::EventBatchSize,
            event_count as f64,
            MetricLabels::one(LabelKey::Stage, "event_egress"),
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

#[cfg(unix)]
fn process_resident_memory_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage 在成功返回前完整初始化调用方提供的 rusage 指针；指针来自有效栈内存。
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    // SAFETY: 仅在 getrusage 返回 0 后读取，POSIX 契约保证结构体已初始化。
    let max_rss = unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64;
    #[cfg(target_vendor = "apple")]
    return Some(max_rss);
    #[cfg(not(target_vendor = "apple"))]
    return max_rss.checked_mul(1024);
}

#[cfg(not(unix))]
fn process_resident_memory_bytes() -> Option<u64> {
    None
}

impl TickMetricContext {
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
        Tick::Timer(_) => "timer",
        Tick::Command(_) => "command",
        Tick::Connected(_) => "connected",
        Tick::Disconnected(_) => "disconnected",
    }
}

fn effect_bytes(effect: &OwnedEffect) -> usize {
    match effect {
        OwnedEffect::Send { frame, .. } => frame.len(),
        OwnedEffect::Persist { ops, .. } | OwnedEffect::PersistFire { ops } => {
            ops.len() * std::mem::size_of::<helix_core::effect::StorageOp>()
        }
        OwnedEffect::Http { req, .. } | OwnedEffect::HttpFire { req } => {
            req.body.as_ref().map_or(0, bytes::Bytes::len)
        }
        OwnedEffect::UploadFile { req, .. } => req.size.unwrap_or(0) as usize,
        OwnedEffect::Request { payload, .. } => payload.len(),
        OwnedEffect::Emit { event } => event.0.len(),
        OwnedEffect::ScheduleTimer { .. } | OwnedEffect::CancelTimer { .. } => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::NoopMetricSink;

    #[test]
    fn pending_port_correlation_is_strictly_bounded() {
        let mut recorder = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
        for corr in 0..5000 {
            recorder.remember_port_start(corr);
        }
        assert_eq!(recorder.port_started.len(), 4096);
        assert_eq!(recorder.port_order.len(), 4096);
    }
}
