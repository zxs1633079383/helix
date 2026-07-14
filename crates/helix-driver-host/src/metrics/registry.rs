#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricAvailability {
    Runtime,
    /// 仅由显式 allocator/profile harness 采集，不冒充常驻进程指标。
    ProfileOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetricDescriptor {
    pub id: MetricId,
    pub name: &'static str,
    pub kind: MetricKind,
    pub unit: &'static str,
}

macro_rules! metric_registry {
    ($( $variant:ident => ($name:literal, $kind:ident, $unit:literal) ),+ $(,)?) => {
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum MetricId { $( $variant ),+ }

        impl MetricId {
            pub const fn descriptor(self) -> &'static MetricDescriptor {
                &ALL_METRICS[self as usize]
            }

            pub const fn availability(self) -> MetricAvailability {
                match self {
                    Self::AllocCountPerOp | Self::AllocBytesPerOp => MetricAvailability::ProfileOnly,
                    _ => MetricAvailability::Runtime,
                }
            }
        }

        pub const ALL_METRIC_IDS: &[MetricId] = &[$(MetricId::$variant),+];
        const ALL_METRICS: &[MetricDescriptor] = &[
            $(MetricDescriptor {
                id: MetricId::$variant,
                name: $name,
                kind: MetricKind::$kind,
                unit: $unit,
            }),+
        ];
    };
}

metric_registry! {
    OperationsTotal => ("helix_operations_total", Counter, "operations"),
    ErrorsTotal => ("helix_errors_total", Counter, "errors"),
    CoreStepDurationSeconds => ("helix_core_step_duration_seconds", Histogram, "s"),
    TickQueueWaitSeconds => ("helix_tick_queue_wait_seconds", Histogram, "s"),
    TickQueueDepth => ("helix_tick_queue_depth", Gauge, "count"),
    TickQueueCapacity => ("helix_tick_queue_capacity", Gauge, "count"),
    EffectsPerTick => ("helix_effects_per_tick", Histogram, "count"),
    EffectBytesPerTick => ("helix_effect_bytes_per_tick", Histogram, "bytes"),
    EffectAmplificationRatio => ("helix_effect_amplification_ratio", Histogram, "ratio"),
    HttpQueueWaitSeconds => ("helix_http_queue_wait_seconds", Histogram, "s"),
    HttpRequestDurationSeconds => ("helix_http_request_duration_seconds", Histogram, "s"),
    HttpInflight => ("helix_http_inflight", Gauge, "count"),
    PersistQueueWaitSeconds => ("helix_persist_queue_wait_seconds", Histogram, "s"),
    StorageTxDurationSeconds => ("helix_storage_tx_duration_seconds", Histogram, "s"),
    StorageRowsTotal => ("helix_storage_rows_total", Counter, "rows"),
    PortRoundtripSeconds => ("helix_port_roundtrip_seconds", Histogram, "s"),
    WsIngressToEffectSeconds => ("helix_ws_ingress_to_effect_seconds", Histogram, "s"),
    WsIngressToEventSeconds => ("helix_ws_ingress_to_event_seconds", Histogram, "s"),
    CommandToProjectionSeconds => ("helix_command_to_projection_seconds", Histogram, "s"),
    EventLaggedTotal => ("helix_event_lagged_total", Counter, "count"),
    EventNoReceiverTotal => ("helix_event_no_receiver_total", Counter, "count"),
    EventBatchSize => ("helix_event_batch_size", Histogram, "count"),
    FfiBatchEvents => ("helix_ffi_batch_events", Histogram, "count"),
    FfiBatchBytes => ("helix_ffi_batch_bytes", Histogram, "bytes"),
    FfiCallbackDurationSeconds => ("helix_ffi_callback_duration_seconds", Histogram, "s"),
    FfiBusyTotal => ("helix_ffi_busy_total", Counter, "count"),
    AllocCountPerOp => ("helix_alloc_count_per_op", Histogram, "count"),
    AllocBytesPerOp => ("helix_alloc_bytes_per_op", Histogram, "bytes"),
    ProcessResidentMemoryBytes => ("helix_process_resident_memory_bytes", Gauge, "bytes"),
    ShutdownDrainSeconds => ("helix_shutdown_drain_seconds", Histogram, "s"),
    MetricsQueueDepth => ("helix_metrics_queue_depth", Gauge, "count"),
    MetricsQueueCapacity => ("helix_metrics_queue_capacity", Gauge, "count"),
    MetricsDroppedTotal => ("helix_metrics_dropped_total", Counter, "count"),
    MetricsExportBatchSize => ("helix_metrics_export_batch_size", Histogram, "count"),
    MetricsExportDurationSeconds => ("helix_metrics_export_duration_seconds", Histogram, "s"),
    MetricsExportErrorsTotal => ("helix_metrics_export_errors_total", Counter, "count"),
    MetricsExporterState => ("helix_metrics_exporter_state", Gauge, "state"),
    MetricsLastSuccessAgeSeconds => ("helix_metrics_last_success_age_seconds", Gauge, "s"),
    LostTotal => ("helix_lost_total", Counter, "count"),
    DuplicateTotal => ("helix_duplicate_total", Counter, "count"),
    OrderViolationTotal => ("helix_order_violation_total", Counter, "count"),
    WrongChannelTotal => ("helix_wrong_channel_total", Counter, "count"),
    CursorRegressionTotal => ("helix_cursor_regression_total", Counter, "count"),
    ProjectionMismatchTotal => ("helix_projection_mismatch_total", Counter, "count"),
    PersistRemainingTotal => ("helix_persist_remaining_total", Counter, "count"),
    CleanupResidueTotal => ("helix_cleanup_residue_total", Counter, "count"),
    TelemetryCanaryTotal => ("helix_telemetry_canary_total", Counter, "count"),
}

pub const REQUIRED_METRIC_IDS: &[MetricId] = &[
    MetricId::OperationsTotal,
    MetricId::ErrorsTotal,
    MetricId::CoreStepDurationSeconds,
    MetricId::TickQueueWaitSeconds,
    MetricId::TickQueueDepth,
    MetricId::TickQueueCapacity,
    MetricId::EffectsPerTick,
    MetricId::EffectBytesPerTick,
    MetricId::EffectAmplificationRatio,
    MetricId::HttpQueueWaitSeconds,
    MetricId::HttpRequestDurationSeconds,
    MetricId::HttpInflight,
    MetricId::PersistQueueWaitSeconds,
    MetricId::StorageTxDurationSeconds,
    MetricId::StorageRowsTotal,
    MetricId::PortRoundtripSeconds,
    MetricId::WsIngressToEffectSeconds,
    MetricId::WsIngressToEventSeconds,
    MetricId::CommandToProjectionSeconds,
    MetricId::EventLaggedTotal,
    MetricId::EventNoReceiverTotal,
    MetricId::EventBatchSize,
    MetricId::FfiBatchEvents,
    MetricId::FfiBatchBytes,
    MetricId::FfiCallbackDurationSeconds,
    MetricId::FfiBusyTotal,
    MetricId::AllocCountPerOp,
    MetricId::AllocBytesPerOp,
    MetricId::ProcessResidentMemoryBytes,
    MetricId::ShutdownDrainSeconds,
    MetricId::MetricsQueueDepth,
    MetricId::MetricsQueueCapacity,
    MetricId::MetricsDroppedTotal,
    MetricId::MetricsExportBatchSize,
    MetricId::MetricsExportDurationSeconds,
    MetricId::MetricsExportErrorsTotal,
    MetricId::MetricsExporterState,
    MetricId::MetricsLastSuccessAgeSeconds,
];

/// Collector canary 必须出现的最小闭环：业务吞吐/错误 + exporter 自观测 + 心跳。
pub const CANARY_METRIC_IDS: &[MetricId] = &[
    MetricId::OperationsTotal,
    MetricId::ErrorsTotal,
    MetricId::MetricsQueueDepth,
    MetricId::MetricsQueueCapacity,
    MetricId::MetricsDroppedTotal,
    MetricId::MetricsExportBatchSize,
    MetricId::MetricsExportDurationSeconds,
    MetricId::MetricsExportErrorsTotal,
    MetricId::MetricsExporterState,
    MetricId::MetricsLastSuccessAgeSeconds,
    MetricId::TelemetryCanaryTotal,
];

pub fn production_metric_ids() -> impl Iterator<Item = MetricId> {
    REQUIRED_METRIC_IDS
        .iter()
        .copied()
        .filter(|id| id.availability() == MetricAvailability::Runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn required_registry_has_exactly_38_unique_metrics() {
        assert_eq!(REQUIRED_METRIC_IDS.len(), 38);
        let names: HashSet<_> = REQUIRED_METRIC_IDS
            .iter()
            .map(|id| id.descriptor().name)
            .collect();
        assert_eq!(names.len(), 38);
    }

    #[test]
    fn total_registry_includes_correctness_and_canary() {
        assert_eq!(ALL_METRIC_IDS.len(), 47);
        assert_eq!(
            MetricId::TelemetryCanaryTotal.descriptor().name,
            "helix_telemetry_canary_total"
        );
    }

    #[test]
    fn allocation_metrics_are_explicitly_profile_only() {
        let profile_only: Vec<_> = REQUIRED_METRIC_IDS
            .iter()
            .copied()
            .filter(|id| id.availability() == MetricAvailability::ProfileOnly)
            .collect();
        assert_eq!(
            profile_only,
            vec![MetricId::AllocCountPerOp, MetricId::AllocBytesPerOp]
        );
        assert_eq!(production_metric_ids().count(), 36);
    }

    #[test]
    fn canary_coverage_contains_real_worker_heartbeat() {
        assert!(CANARY_METRIC_IDS.contains(&MetricId::TelemetryCanaryTotal));
        assert!(CANARY_METRIC_IDS.contains(&MetricId::MetricsExportErrorsTotal));
        assert!(CANARY_METRIC_IDS
            .iter()
            .all(|id| id.availability() == MetricAvailability::Runtime));
    }
}
