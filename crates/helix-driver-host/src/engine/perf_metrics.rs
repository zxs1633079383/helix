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
}

#[derive(Clone, Copy)]
struct PendingPort {
    corr: u64,
    started: Instant,
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
        }
    }

    pub(crate) fn sink(&self) -> &dyn AsyncMetricSink {
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

    pub(super) fn on_tick(
        &mut self,
        tick: &Tick,
        queue_wait: Option<Duration>,
    ) -> TickMetricContext {
        if !self.enabled {
            return TickMetricContext::disabled();
        }
        if let Some(queue_wait) = queue_wait {
            self.histogram(
                MetricId::TickQueueWaitSeconds,
                queue_wait.as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "core")
                    .with(LabelKey::TickKind, tick_kind(tick)),
            );
        }
        if let Tick::PortReply { corr, outcome } = tick {
            if let Some(started) = self.complete_port(corr.raw()) {
                let status = match outcome {
                    PortOutcome::Ok(_) => "ok",
                    PortOutcome::Err(_) => "error",
                };
                self.histogram(
                    MetricId::PortRoundtripSeconds,
                    started.elapsed().as_secs_f64(),
                    MetricLabels::one(LabelKey::Stage, "port_reply").with(LabelKey::Status, status),
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

    pub(crate) fn on_effect_dispatch(&mut self, effect: &OwnedEffect) {
        let corr = match effect {
            OwnedEffect::Persist { corr, .. }
            | OwnedEffect::PersistAtomic { corr, .. }
            | OwnedEffect::Http { corr, .. }
            | OwnedEffect::UploadFile { corr, .. }
            | OwnedEffect::Request { corr, .. } => Some(corr.raw()),
            _ => None,
        };
        if let Some(corr) = corr {
            self.remember_port_start(corr);
        }
    }

    fn remember_port_start(&mut self, corr: u64) {
        let slot = corr as usize % MAX_PENDING_PORTS;
        self.port_slots[slot] = Some(PendingPort {
            corr,
            started: Instant::now(),
        });
    }

    fn complete_port(&mut self, corr: u64) -> Option<Instant> {
        let slot = corr as usize % MAX_PENDING_PORTS;
        match self.port_slots[slot] {
            Some(pending) if pending.corr == corr => {
                self.port_slots[slot] = None;
                Some(pending.started)
            }
            _ => None,
        }
    }

    pub(super) fn start_timer(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
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

#[cfg(test)]
#[path = "perf_metrics_tests.rs"]
mod tests;
