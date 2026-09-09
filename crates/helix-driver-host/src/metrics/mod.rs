//! Host-only asynchronous metrics boundary.
//!
//! Callers only perform a bounded `try_send`. OTLP, aggregation, retries and
//! shutdown live on the dedicated worker and never enter `helix-core`.

mod business;
mod event;
mod labels;
mod otlp;
mod registry;
mod runtime;
mod sink;
mod sync;
pub use sync::record_sync_timing;
mod worker;

pub(crate) use business::record_im_command_terminal_event;
pub use business::{
    record_gap_duration, record_im_business_event, record_message_client_terminal,
    record_message_e2e_view_updated, record_seq_observation, record_tick_stage_duration,
    ClientTerminalObservation,
};
pub use event::MetricEvent;
pub use labels::{LabelKey, MetricLabel, MetricLabels, ALLOWED_LABEL_KEYS};
pub use registry::{
    production_metric_ids, MetricAvailability, MetricDescriptor, MetricId, MetricKind,
    ALL_METRIC_IDS, CANARY_METRIC_IDS, REQUIRED_METRIC_IDS,
};
pub use runtime::{
    BatchMetricExporter, BoundedMetricRuntime, MetricExportError, MetricExporterState,
    MetricRuntimeConfig, MetricRuntimeError, MetricRuntimeSnapshot, ShutdownOutcome,
};
pub use sink::{AsyncMetricSink, NoopMetricSink, RecordOutcome};

pub(crate) use otlp::OtlpMetricExporter;
