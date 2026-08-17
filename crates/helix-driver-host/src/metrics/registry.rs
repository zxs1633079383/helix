#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricAvailability {
    Runtime,
    /// 名称与标签合同已冻结，但缺少可诚实闭合的当前平台 seam，禁止 dashboard 当成缺数。
    Planned,
    /// 仅由显式 allocator/profile harness 采集，不冒充常驻进程指标。
    ProfileOnly,
    /// 仅由有限时长压测在收尾阶段发布，避免抓取周期吞掉突发吞吐率。
    HarnessOnly,
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
                    Self::RunThroughputPerSecond
                    | Self::LostTotal
                    | Self::DuplicateTotal
                    | Self::OrderViolationTotal
                    | Self::WrongChannelTotal
                    | Self::CursorRegressionTotal
                    | Self::ProjectionMismatchTotal
                    | Self::PersistRemainingTotal
                    | Self::CleanupResidueTotal
                    | Self::TerminalProjectionMissingTotal
                    | Self::OptimisticProjectionStuckTotal => MetricAvailability::HarnessOnly,
                    Self::HttpCancelledTotal
                    | Self::TickQueueDroppedTotal
                    | Self::StoragePoolWaitSeconds
                    | Self::StorageWalBytes
                    | Self::StorageCheckpointDurationSeconds
                    | Self::StorageCheckpointErrorsTotal
                    | Self::PortReplyLateTotal
                    | Self::WsConnectDurationSeconds
                    | Self::WsReconnectTotal
                    | Self::WsReconnectDelaySeconds
                    | Self::WsHeartbeatRoundtripSeconds
                    | Self::WsDecodeDurationSeconds
                    | Self::WsDecodeErrorsTotal
                    | Self::CommandToProjectionSeconds
                    | Self::CommandToOptimisticPersistSeconds
                    | Self::CommandToOptimisticProjectionSeconds
                    | Self::CommandToWsEchoSeconds
                    | Self::CommandToAuthoritativePersistSeconds
                    | Self::CommandToProjectionConsumeSeconds
                    | Self::CorrelationTrackerOverflowTotal
                    | Self::CorrelationTrackerTimeoutTotal
                    | Self::CorrelationTrackerDuplicateTotal
                    | Self::CorrelationTrackerLateTotal
                    | Self::EventEmitToConsumeSeconds
                    | Self::EventConsumerDurationSeconds
                    | Self::EventBridgeDurationSeconds
                    | Self::EventBridgeErrorsTotal
                    | Self::FfiCommandsTotal
                    | Self::FfiCommandDurationSeconds
                    | Self::WebBridgeDurationSeconds
                    | Self::WebBridgeErrorsTotal
                    | Self::WebEventBatchSize
                    | Self::BuildInfo
                    | Self::ProcessStartTimeSeconds
                    | Self::ProcessUptimeSeconds
                    | Self::RuntimeTasks
                    | Self::TaskPanicsTotal
                    | Self::ShutdownPendingJobs
                    | Self::ShutdownAbortedJobsTotal
                    | Self::ShutdownTimeoutTotal => MetricAvailability::Planned,
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
    CommandAcceptedTotal => ("helix_command_accepted_total", Counter, "1"),
    CommandRejectedTotal => ("helix_command_rejected_total", Counter, "1"),
    CommandAdmissionDurationSeconds => ("helix_command_admission_duration_seconds", Histogram, "s"),
    TicksTotal => ("helix_ticks_total", Counter, "1"),
    CoreStepDurationSeconds => ("helix_core_step_duration_seconds", Histogram, "s"),
    CoreStepErrorsTotal => ("helix_core_step_errors_total", Counter, "1"),
    CoreEmptyEffectTotal => ("helix_core_empty_effect_total", Counter, "1"),
    TickQueueWaitSeconds => ("helix_tick_queue_wait_seconds", Histogram, "s"),
    TickQueueDepth => ("helix_tick_queue_depth", Gauge, ""),
    TickQueueCapacity => ("helix_tick_queue_capacity", Gauge, ""),
    TickQueueFullTotal => ("helix_tick_queue_full_total", Counter, "1"),
    TickQueueDroppedTotal => ("helix_tick_queue_dropped_total", Counter, "1"),
    TickInflight => ("helix_tick_inflight", Gauge, "1"),
    ReplyFairnessForcedTotal => ("helix_reply_fairness_forced_total", Counter, "1"),
    EngineLoopIterationSeconds => ("helix_engine_loop_iteration_seconds", Histogram, "s"),
    EngineState => ("helix_engine_state", Gauge, "state"),
    EffectsTotal => ("helix_effects_total", Counter, "1"),
    EffectsPerTick => ("helix_effects_per_tick", Histogram, "1"),
    EffectBytesPerTick => ("helix_effect_bytes_per_tick", Histogram, "bytes"),
    EffectAmplificationRatio => ("helix_effect_amplification_ratio", Histogram, "ratio"),
    EffectDispatchDurationSeconds => ("helix_effect_dispatch_duration_seconds", Histogram, "s"),
    EffectDispatchErrorsTotal => ("helix_effect_dispatch_errors_total", Counter, "1"),
    PoolQueueDepth => ("helix_pool_queue_depth", Gauge, "1"),
    PoolQueueCapacity => ("helix_pool_queue_capacity", Gauge, "1"),
    PoolEnqueueBlockSeconds => ("helix_pool_enqueue_block_seconds", Histogram, "s"),
    PoolQueueResidencySeconds => ("helix_pool_queue_residency_seconds", Histogram, "s"),
    PoolExecutionSeconds => ("helix_pool_execution_seconds", Histogram, "s"),
    PoolInflight => ("helix_pool_inflight", Gauge, "1"),
    PoolWorkers => ("helix_pool_workers", Gauge, "1"),
    PoolDroppedTotal => ("helix_pool_dropped_total", Counter, "1"),
    PoolClosedTotal => ("helix_pool_closed_total", Counter, "1"),
    PoolJobPanicsTotal => ("helix_pool_job_panics_total", Counter, "1"),
    HttpQueueWaitSeconds => ("helix_http_queue_wait_seconds", Histogram, "s"),
    HttpRequestDurationSeconds => ("helix_http_request_duration_seconds", Histogram, "s"),
    HttpInflight => ("helix_http_inflight", Gauge, ""),
    HttpRequestBytes => ("helix_http_request_bytes", Histogram, "bytes"),
    HttpResponseBytes => ("helix_http_response_bytes", Histogram, "bytes"),
    HttpTimeoutTotal => ("helix_http_timeout_total", Counter, "1"),
    HttpCancelledTotal => ("helix_http_cancelled_total", Counter, "1"),
    HttpStatusTotal => ("helix_http_status_total", Counter, "1"),
    PersistQueueWaitSeconds => ("helix_persist_queue_wait_seconds", Histogram, "s"),
    StorageTxDurationSeconds => ("helix_storage_tx_duration_seconds", Histogram, "s"),
    StorageRowsTotal => ("helix_storage_rows_total", Counter, "rows"),
    StorageInflight => ("helix_storage_inflight", Gauge, "1"),
    StoragePoolWaitSeconds => ("helix_storage_pool_wait_seconds", Histogram, "s"),
    StorageBusyTotal => ("helix_storage_busy_total", Counter, "1"),
    StorageTimeoutTotal => ("helix_storage_timeout_total", Counter, "1"),
    StorageRollbackTotal => ("helix_storage_rollback_total", Counter, "1"),
    StorageBatchRows => ("helix_storage_batch_rows", Histogram, "rows"),
    StorageWalBytes => ("helix_storage_wal_bytes", Gauge, "bytes"),
    StorageCheckpointDurationSeconds => ("helix_storage_checkpoint_duration_seconds", Histogram, "s"),
    StorageCheckpointErrorsTotal => ("helix_storage_checkpoint_errors_total", Counter, "1"),
    PortRoundtripSeconds => ("helix_port_roundtrip_seconds", Histogram, "s"),
    PortReplyQueueDepth => ("helix_port_reply_queue_depth", Gauge, "1"),
    PortReplyQueueWaitSeconds => ("helix_port_reply_queue_wait_seconds", Histogram, "s"),
    PortReplyTotal => ("helix_port_reply_total", Counter, "1"),
    PortReplyOrphanTotal => ("helix_port_reply_orphan_total", Counter, "1"),
    PortReplyLateTotal => ("helix_port_reply_late_total", Counter, "1"),
    PortCorrelationCollisionTotal => ("helix_port_correlation_collision_total", Counter, "1"),
    PortPending => ("helix_port_pending", Gauge, "1"),
    PortPendingCapacity => ("helix_port_pending_capacity", Gauge, "1"),
    UnknownPortReplyTotal => ("helix_unknown_port_reply_total", Counter, "1"),
    WsConnectionState => ("helix_ws_connection_state", Gauge, "state"),
    WsConnectTotal => ("helix_ws_connect_total", Counter, "1"),
    WsConnectDurationSeconds => ("helix_ws_connect_duration_seconds", Histogram, "s"),
    WsDisconnectTotal => ("helix_ws_disconnect_total", Counter, "1"),
    WsReconnectTotal => ("helix_ws_reconnect_total", Counter, "1"),
    WsReconnectDelaySeconds => ("helix_ws_reconnect_delay_seconds", Histogram, "s"),
    WsFramesTotal => ("helix_ws_frames_total", Counter, "1"),
    WsFrameBytes => ("helix_ws_frame_bytes", Histogram, "bytes"),
    WsSendDurationSeconds => ("helix_ws_send_duration_seconds", Histogram, "s"),
    WsSendErrorsTotal => ("helix_ws_send_errors_total", Counter, "1"),
    WsInboundLastSeenAgeSeconds => ("helix_ws_inbound_last_seen_age_seconds", Gauge, "s"),
    WsHeartbeatRoundtripSeconds => ("helix_ws_heartbeat_roundtrip_seconds", Histogram, "s"),
    WsDecodeDurationSeconds => ("helix_ws_decode_duration_seconds", Histogram, "s"),
    WsDecodeErrorsTotal => ("helix_ws_decode_errors_total", Counter, "1"),
    WsIngressToEffectSeconds => ("helix_ws_ingress_to_effect_seconds", Histogram, "s"),
    WsIngressToEventSeconds => ("helix_ws_ingress_to_event_seconds", Histogram, "s"),
    CommandToProjectionSeconds => ("helix_command_to_projection_seconds", Histogram, "s"),
    CommandToPersistReplySeconds => ("helix_command_to_persist_reply_seconds", Histogram, "s"),
    CommandToImmediateProjectionSeconds => ("helix_command_to_immediate_projection_seconds", Histogram, "s"),
    CommandToOptimisticPersistSeconds => ("helix_command_to_optimistic_persist_seconds", Histogram, "s"),
    CommandToOptimisticProjectionSeconds => ("helix_command_to_optimistic_projection_seconds", Histogram, "s"),
    CommandToHttpDispatchSeconds => ("helix_command_to_http_dispatch_seconds", Histogram, "s"),
    CommandToHttpResponseSeconds => ("helix_command_to_http_response_seconds", Histogram, "s"),
    CommandToWsEchoSeconds => ("helix_command_to_ws_echo_seconds", Histogram, "s"),
    CommandToAuthoritativePersistSeconds => ("helix_command_to_authoritative_persist_seconds", Histogram, "s"),
    CommandToProjectionEmitSeconds => ("helix_command_to_projection_emit_seconds", Histogram, "s"),
    CommandToProjectionConsumeSeconds => ("helix_command_to_projection_consume_seconds", Histogram, "s"),
    CorrelationTrackerOverflowTotal => ("helix_correlation_tracker_overflow_total", Counter, "1"),
    CorrelationTrackerTimeoutTotal => ("helix_correlation_tracker_timeout_total", Counter, "1"),
    CorrelationTrackerDuplicateTotal => ("helix_correlation_tracker_duplicate_total", Counter, "1"),
    CorrelationTrackerLateTotal => ("helix_correlation_tracker_late_total", Counter, "1"),
    EventLaggedTotal => ("helix_event_lagged_total", Counter, "1"),
    EventNoReceiverTotal => ("helix_event_no_receiver_total", Counter, "1"),
    EventBatchSize => ("helix_event_batch_size", Histogram, "1"),
    EventEmittedTotal => ("helix_event_emitted_total", Counter, "1"),
    EventEmitDurationSeconds => ("helix_event_emit_duration_seconds", Histogram, "s"),
    EventEmitToConsumeSeconds => ("helix_event_emit_to_consume_seconds", Histogram, "s"),
    EventReceiverCount => ("helix_event_receiver_count", Gauge, "1"),
    EventConsumerDurationSeconds => ("helix_event_consumer_duration_seconds", Histogram, "s"),
    EventConsumerClosedTotal => ("helix_event_consumer_closed_total", Counter, "1"),
    EventBridgeDurationSeconds => ("helix_event_bridge_duration_seconds", Histogram, "s"),
    EventBridgeErrorsTotal => ("helix_event_bridge_errors_total", Counter, "1"),
    TimerActive => ("helix_timer_active", Gauge, "1"),
    TimerScheduledTotal => ("helix_timer_scheduled_total", Counter, "1"),
    TimerCancelledTotal => ("helix_timer_cancelled_total", Counter, "1"),
    TimerFiredTotal => ("helix_timer_fired_total", Counter, "1"),
    TimerLatenessSeconds => ("helix_timer_lateness_seconds", Histogram, "s"),
    TimerDeliveryWaitSeconds => ("helix_timer_delivery_wait_seconds", Histogram, "s"),
    TimerDeliveryFailedTotal => ("helix_timer_delivery_failed_total", Counter, "1"),
    FfiBatchEvents => ("helix_ffi_batch_events", Histogram, "1"),
    FfiBatchBytes => ("helix_ffi_batch_bytes", Histogram, "bytes"),
    FfiCallbackDurationSeconds => ("helix_ffi_callback_duration_seconds", Histogram, "s"),
    FfiBusyTotal => ("helix_ffi_busy_total", Counter, "1"),
    FfiCommandsTotal => ("helix_ffi_commands_total", Counter, "1"),
    FfiCommandDurationSeconds => ("helix_ffi_command_duration_seconds", Histogram, "s"),
    FfiCallbackErrorsTotal => ("helix_ffi_callback_errors_total", Counter, "1"),
    FfiEventsDroppedTotal => ("helix_ffi_events_dropped_total", Counter, "1"),
    FfiBoundaryBytes => ("helix_ffi_boundary_bytes", Histogram, "bytes"),
    WebBridgeDurationSeconds => ("helix_web_bridge_duration_seconds", Histogram, "s"),
    WebBridgeErrorsTotal => ("helix_web_bridge_errors_total", Counter, "1"),
    WebEventBatchSize => ("helix_web_event_batch_size", Histogram, "1"),
    AllocCountPerOp => ("helix_alloc_count_per_op", Histogram, "1"),
    AllocBytesPerOp => ("helix_alloc_bytes_per_op", Histogram, "bytes"),
    BuildInfo => ("helix_build_info", Gauge, "1"),
    ProcessStartTimeSeconds => ("helix_process_start_time_seconds", Gauge, "s"),
    ProcessUptimeSeconds => ("helix_process_uptime_seconds", Gauge, "s"),
    ProcessResidentMemoryBytes => ("helix_process_resident_memory_bytes", Gauge, "bytes"),
    RuntimeTasks => ("helix_runtime_tasks", Gauge, "1"),
    TaskPanicsTotal => ("helix_task_panics_total", Counter, "1"),
    TransportCount => ("helix_transport_count", Gauge, "1"),
    ShutdownDrainSeconds => ("helix_shutdown_drain_seconds", Histogram, "s"),
    ShutdownPendingJobs => ("helix_shutdown_pending_jobs", Gauge, "1"),
    ShutdownAbortedJobsTotal => ("helix_shutdown_aborted_jobs_total", Counter, "1"),
    ShutdownTimeoutTotal => ("helix_shutdown_timeout_total", Counter, "1"),
    MetricsQueueDepth => ("helix_metrics_queue_depth", Gauge, ""),
    MetricsQueueCapacity => ("helix_metrics_queue_capacity", Gauge, ""),
    MetricsDroppedTotal => ("helix_metrics_dropped_total", Counter, "1"),
    MetricsExportBatchSize => ("helix_metrics_export_batch_size", Histogram, "1"),
    MetricsExportDurationSeconds => ("helix_metrics_export_duration_seconds", Histogram, "s"),
    MetricsExportErrorsTotal => ("helix_metrics_export_errors_total", Counter, "1"),
    MetricsExporterState => ("helix_metrics_exporter_state", Gauge, "state"),
    MetricsLastSuccessAgeSeconds => ("helix_metrics_last_success_age_seconds", Gauge, "s"),
    TracesQueueDepth => ("helix_traces_queue_depth", Gauge, "1"),
    TracesQueueCapacity => ("helix_traces_queue_capacity", Gauge, "1"),
    TracesDroppedTotal => ("helix_traces_dropped_total", Counter, "1"),
    TracesExportBatchSize => ("helix_traces_export_batch_size", Histogram, "1"),
    TracesExportDurationSeconds => ("helix_traces_export_duration_seconds", Histogram, "s"),
    TracesExportErrorsTotal => ("helix_traces_export_errors_total", Counter, "1"),
    TracesExporterState => ("helix_traces_exporter_state", Gauge, "state"),
    TracesLastSuccessAgeSeconds => ("helix_traces_last_success_age_seconds", Gauge, "s"),
    SpansCreatedTotal => ("helix_spans_created_total", Counter, "1"),
    SpansSampledTotal => ("helix_spans_sampled_total", Counter, "1"),
    SlowSpansTotal => ("helix_slow_spans_total", Counter, "1"),
    LostTotal => ("helix_lost_total", Counter, "1"),
    DuplicateTotal => ("helix_duplicate_total", Counter, "1"),
    OrderViolationTotal => ("helix_order_violation_total", Counter, "1"),
    WrongChannelTotal => ("helix_wrong_channel_total", Counter, "1"),
    CursorRegressionTotal => ("helix_cursor_regression_total", Counter, "1"),
    ProjectionMismatchTotal => ("helix_projection_mismatch_total", Counter, "1"),
    PersistRemainingTotal => ("helix_persist_remaining_total", Counter, "1"),
    CleanupResidueTotal => ("helix_cleanup_residue_total", Counter, "1"),
    TerminalProjectionMissingTotal => ("helix_terminal_projection_missing_total", Counter, "1"),
    OptimisticProjectionStuckTotal => ("helix_optimistic_projection_stuck_total", Counter, "1"),
    TelemetryCanaryTotal => ("helix_telemetry_canary_total", Counter, "1"),
    RunThroughputPerSecond => ("helix_run_throughput_per_second", Gauge, ""),
}

/// 所有注册指标都是 schema 合同的一部分；availability 决定常驻、profile 或 harness 出口。
pub const REQUIRED_METRIC_IDS: &[MetricId] = ALL_METRIC_IDS;

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
    MetricId::TracesQueueDepth,
    MetricId::TracesQueueCapacity,
    MetricId::TracesDroppedTotal,
    MetricId::TracesExportBatchSize,
    MetricId::TracesExportDurationSeconds,
    MetricId::TracesExportErrorsTotal,
    MetricId::TracesExporterState,
    MetricId::TracesLastSuccessAgeSeconds,
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
    /// required registry 必须覆盖 catalog 且名称全局唯一。
    fn required_registry_contains_every_metric_exactly_once() {
        assert_eq!(REQUIRED_METRIC_IDS.len(), ALL_METRIC_IDS.len());
        let names: HashSet<_> = REQUIRED_METRIC_IDS
            .iter()
            .map(|id| id.descriptor().name)
            .collect();
        assert_eq!(names.len(), REQUIRED_METRIC_IDS.len());
    }

    #[test]
    fn total_registry_includes_correctness_and_canary() {
        assert!(ALL_METRIC_IDS.len() > 100);
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
        assert!(production_metric_ids().count() > 80);
        assert_eq!(
            MetricId::RunThroughputPerSecond.availability(),
            MetricAvailability::HarnessOnly
        );
    }

    #[test]
    fn canary_coverage_contains_real_worker_heartbeat() {
        assert!(CANARY_METRIC_IDS.contains(&MetricId::TelemetryCanaryTotal));
        assert!(CANARY_METRIC_IDS.contains(&MetricId::MetricsExportErrorsTotal));
        assert!(CANARY_METRIC_IDS.contains(&MetricId::TracesExportErrorsTotal));
        assert!(CANARY_METRIC_IDS
            .iter()
            .all(|id| id.availability() == MetricAvailability::Runtime));
    }

    #[test]
    fn dimensionless_metrics_use_ucum_one_without_prometheus_name_suffix() {
        for id in ALL_METRIC_IDS {
            assert_ne!(
                id.descriptor().unit,
                "count",
                "{} must use UCUM 1 instead of count",
                id.descriptor().name
            );
        }
        for id in [
            MetricId::TickQueueDepth,
            MetricId::TickQueueCapacity,
            MetricId::HttpInflight,
            MetricId::MetricsQueueDepth,
            MetricId::MetricsQueueCapacity,
        ] {
            assert_eq!(id.descriptor().unit, "");
        }
    }
}
