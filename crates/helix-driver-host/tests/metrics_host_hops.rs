use std::sync::{Arc, Mutex};

use helix_core::effect::{HttpRequest, SqlValue, UpsertSpec};
use helix_core::ports::{HttpRequester, Storage};
use helix_driver_host::{
    AsyncMetricSink, HostNetworkConfig, HostStorage, LabelKey, MetricEvent, MetricId,
    RecordOutcome, SharedHttpClient,
};

#[derive(Clone, Default)]
struct RecordingMetricSink(Arc<Mutex<Vec<MetricEvent>>>);

impl AsyncMetricSink for RecordingMetricSink {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

#[tokio::test]
/// Verify metrics are recorded by the real storage boundary.
async fn host_storage_records_duration_and_rows_at_real_port_boundary() {
    let storage = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .expect("open sqlite");
    storage
        .execute_raw("CREATE TABLE metric_rows (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
        .await
        .expect("create table");
    let metrics = RecordingMetricSink::default();
    let recorded = Arc::clone(&metrics.0);
    let storage = storage.with_metric_sink(Arc::new(metrics));
    storage
        .batch_upsert(UpsertSpec {
            table: "metric_rows",
            rows: vec![vec![
                ("id".to_string(), SqlValue::Integer(1)),
                ("value".to_string(), SqlValue::Text("ok".to_string())),
            ]],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("upsert");
    let events = recorded.lock().unwrap();
    assert!(events
        .iter()
        .any(|event| event.id == MetricId::StorageTxDurationSeconds));
    assert!(events
        .iter()
        .any(|event| event.id == MetricId::StorageRowsTotal && event.value == 1.0));
    assert!(events.iter().any(|event| {
        event.id == MetricId::OperationsTotal
            && has_label(event, LabelKey::Status, "ok")
            && has_label(event, LabelKey::Operation, "batch_upsert")
    }));
}

#[tokio::test]
/// Verify one failed storage operation increments the error counter once.
async fn storage_failure_records_one_error_operation_and_error_counter() {
    let storage = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .expect("open sqlite");
    let metrics = RecordingMetricSink::default();
    let recorded = Arc::clone(&metrics.0);
    let storage = storage.with_metric_sink(Arc::new(metrics));
    let result = storage
        .batch_upsert(UpsertSpec {
            table: "missing_table",
            rows: vec![vec![("id".to_string(), SqlValue::Integer(1))]],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await;
    assert!(result.is_err(), "破坏表名必须令 storage 失败");
    let events = recorded.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.id == MetricId::OperationsTotal && has_label(event, LabelKey::Status, "error")
            })
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.id == MetricId::ErrorsTotal)
            .count(),
        1
    );
}

#[tokio::test]
async fn http_prepare_failure_records_one_error_operation_and_error_counter() {
    let metrics = RecordingMetricSink::default();
    let recorded = Arc::clone(&metrics.0);
    let client = SharedHttpClient::new(HostNetworkConfig::new(
        "http://127.0.0.1:1",
        "ws://127.0.0.1:1",
    ))
    .expect("valid config")
    .with_metric_sink(Arc::new(metrics));
    let result = client
        .request(HttpRequest {
            method: "bad method".to_string(),
            url: "/test".to_string(),
            headers: Vec::new(),
            body: None,
        })
        .await;
    assert!(result.is_err(), "非法 method 必须在真实 HTTP port 边界失败");
    let events = recorded.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.id == MetricId::OperationsTotal && has_label(event, LabelKey::Status, "error")
            })
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.id == MetricId::ErrorsTotal)
            .count(),
        1
    );
}

fn has_label(event: &MetricEvent, key: LabelKey, value: &str) -> bool {
    event
        .labels
        .iter()
        .any(|label| label.key == key && label.value == value)
}
