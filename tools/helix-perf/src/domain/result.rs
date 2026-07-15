use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Pass,
    Partial,
    Blocked,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleStats {
    pub count: u64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub avg_ms: f64,
    pub operations_per_second: f64,
}

impl SampleStats {
    pub fn from_histogram(histogram: &Histogram<u64>, elapsed_seconds: f64) -> Self {
        let micros_to_ms = |value: u64| value as f64 / 1_000.0;
        Self {
            count: histogram.len(),
            p50_ms: micros_to_ms(histogram.value_at_quantile(0.50)),
            p90_ms: micros_to_ms(histogram.value_at_quantile(0.90)),
            p95_ms: micros_to_ms(histogram.value_at_quantile(0.95)),
            p99_ms: micros_to_ms(histogram.value_at_quantile(0.99)),
            avg_ms: histogram.mean() / 1_000.0,
            operations_per_second: histogram.len() as f64 / elapsed_seconds.max(0.000_001),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricPoint {
    pub schema_version: u32,
    pub run_id: String,
    pub ts_unix_ns: u128,
    pub name: String,
    pub kind: String,
    pub unit: String,
    pub value: f64,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub schema_version: u32,
    pub run_id: String,
    pub scenario_id: String,
    pub verdict: Verdict,
    pub latency: BTreeMap<String, SampleStats>,
    pub qps: BTreeMap<String, f64>,
    pub rps: BTreeMap<String, f64>,
    pub error_count: u64,
    pub error_rate: f64,
    pub error_ratio: f64,
    pub dropped: u64,
    pub correctness: BTreeMap<String, u64>,
    pub assertions: BTreeMap<String, bool>,
    pub errors: Vec<String>,
}

pub struct RunArtifacts {
    pub hdr: Vec<u8>,
    pub trace_index: Vec<Value>,
    pub sqlite: Value,
    pub cleanup: Value,
    pub profile: Value,
}
