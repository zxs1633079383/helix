use std::sync::{Arc, Mutex};

use helix_core::effect::{SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_host::{AsyncMetricSink, HostStorage, MetricEvent, MetricId, RecordOutcome};

#[derive(Clone, Default)]
struct RecordingMetricSink(Arc<Mutex<Vec<MetricEvent>>>);

impl AsyncMetricSink for RecordingMetricSink {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

#[tokio::test]
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
}
