use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use helix_core::Tick;

use super::rss::process_resident_memory_bytes;
use crate::metrics::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};
use crate::owned_effect::OwnedEffect;

pub(super) struct EngineMetricRecorder {
    sink: Arc<dyn AsyncMetricSink>,
    enabled: bool,
    port_started: HashMap<u64, PendingPort>,
    port_slots: [Option<u64>; MAX_PENDING_PORTS],
    next_port_slot: usize,
}

#[derive(Clone, Copy)]
struct PendingPort {
    started: Instant,
    slot: usize,
}

const MAX_PENDING_PORTS: usize = 4096;

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
            port_slots: [None; MAX_PENDING_PORTS],
            next_port_slot: 0,
        }
    }

    pub(super) fn sink(&self) -> &dyn AsyncMetricSink {
        self.sink.as_ref()
    }

    pub(super) fn record_queue_snapshot(&self, depth: usize, capacity: usize) {
        if !self.enabled {
            return;
        }
        let labels = MetricLabels::one(LabelKey::Stage, "core");
        self.gauge(MetricId::TickQueueDepth, depth as f64, labels);
        self.gauge(MetricId::TickQueueCapacity, capacity as f64, labels);
    }

    pub(super) fn on_tick(&mut self, tick: &Tick) -> TickMetricContext {
        if !self.enabled {
            return TickMetricContext::disabled();
        }
        if let Tick::PortReply { corr, .. } = tick {
            if let Some(started) = self.complete_port(corr.raw()) {
                self.histogram(
                    MetricId::PortRoundtripSeconds,
                    started.elapsed().as_secs_f64(),
                    MetricLabels::one(LabelKey::Stage, "port_reply"),
                );
                self.counter(
                    MetricId::OperationsTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "port_reply")
                        .with(LabelKey::Operation, "reply")
                        .with(LabelKey::Status, "ok"),
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
            MetricLabels::one(LabelKey::Stage, "core")
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
        self.record_effect_batch(effects, context.tick_kind);
        self.remember_port_starts(effects);
        if let Some(started) = context.ingress_started {
            self.histogram(
                MetricId::WsIngressToEffectSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core"),
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
                MetricLabels::one(LabelKey::Stage, "event"),
            );
        }
        if let Some(started) = context.command_started {
            self.histogram(
                MetricId::CommandToProjectionSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "event"),
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
        if let Some(pending) = self.port_started.get_mut(&corr) {
            pending.started = Instant::now();
            return;
        }
        let slot = self.next_port_slot;
        self.next_port_slot = (slot + 1) % MAX_PENDING_PORTS;
        if let Some(evicted) = self.port_slots[slot].replace(corr) {
            self.port_started.remove(&evicted);
        }
        self.port_started.insert(
            corr,
            PendingPort {
                started: Instant::now(),
                slot,
            },
        );
    }

    fn complete_port(&mut self, corr: u64) -> Option<Instant> {
        let pending = self.port_started.remove(&corr)?;
        if self.port_slots[pending.slot] == Some(corr) {
            self.port_slots[pending.slot] = None;
        }
        Some(pending.started)
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
                    MetricLabels::one(LabelKey::Stage, "core"),
                );
            }
        }
        if let Some(started) = started {
            self.histogram(
                MetricId::ShutdownDrainSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core"),
            );
        }
    }

    fn record_effect_batch(&self, effects: &[OwnedEffect], tick_kind: &'static str) {
        let labels =
            MetricLabels::one(LabelKey::Stage, "effect").with(LabelKey::TickKind, tick_kind);
        let count = effects.len();
        let bytes = effects.iter().map(effect_bytes).sum::<usize>();
        self.histogram(MetricId::EffectsPerTick, count as f64, labels);
        self.histogram(MetricId::EffectBytesPerTick, bytes as f64, labels);
        self.histogram(MetricId::EffectAmplificationRatio, count as f64, labels);
        self.counter(
            MetricId::OperationsTotal,
            count as f64,
            labels
                .with(LabelKey::Operation, "produce")
                .with(LabelKey::Status, "ok"),
        );
        let event_count = effects
            .iter()
            .filter(|effect| matches!(effect, OwnedEffect::Emit { .. }))
            .count();
        self.histogram(
            MetricId::EventBatchSize,
            event_count as f64,
            MetricLabels::one(LabelKey::Stage, "event"),
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
#[path = "perf_metrics_tests.rs"]
mod tests;
