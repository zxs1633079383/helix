use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use super::l2_completion::{CompletionOracle, FFI_DONE, HTTP_DONE, STORAGE_DONE};
use hdrhistogram::Histogram;
use serde_json::{json, Value};

pub struct L2Oracle {
    channels: usize,
    seen: Mutex<HashSet<(usize, u64)>>,
    last_seq: Mutex<HashMap<usize, u64>>,
    completion: CompletionOracle,
    port_started: Mutex<HashMap<u64, (Instant, u64, &'static str)>>,
    port_latency: Mutex<BTreeMap<&'static str, Histogram<u64>>>,
    event_started: Mutex<HashMap<u64, Instant>>,
    event_delivery: Mutex<Histogram<u64>>,
    ffi_callback: Mutex<Histogram<u64>>,
    trace: Mutex<Vec<Value>>,
    pub port_replies_ok: AtomicU64,
    pub port_replies_err: AtomicU64,
    pub native_events: AtomicU64,
    pub ffi_events: AtomicU64,
    pub duplicates: AtomicU64,
    pub order_violations: AtomicU64,
    pub cursor_regressions: AtomicU64,
    pub wrong_channel: AtomicU64,
    pub projection_mismatch: AtomicU64,
}

impl L2Oracle {
    pub fn new(channels: usize) -> Result<Self, String> {
        Ok(Self {
            channels,
            seen: Mutex::new(HashSet::new()),
            last_seq: Mutex::new(HashMap::new()),
            completion: CompletionOracle::new()?,
            port_started: Mutex::new(HashMap::new()),
            port_latency: Mutex::new(BTreeMap::from([
                (
                    "http_port_roundtrip",
                    Histogram::new(3).map_err(|e| e.to_string())?,
                ),
                (
                    "storage_port_roundtrip",
                    Histogram::new(3).map_err(|e| e.to_string())?,
                ),
            ])),
            event_started: Mutex::new(HashMap::new()),
            event_delivery: Mutex::new(Histogram::new(3).map_err(|e| e.to_string())?),
            ffi_callback: Mutex::new(Histogram::new(3).map_err(|e| e.to_string())?),
            trace: Mutex::new(Vec::new()),
            port_replies_ok: AtomicU64::new(0),
            port_replies_err: AtomicU64::new(0),
            native_events: AtomicU64::new(0),
            ffi_events: AtomicU64::new(0),
            duplicates: AtomicU64::new(0),
            order_violations: AtomicU64::new(0),
            cursor_regressions: AtomicU64::new(0),
            wrong_channel: AtomicU64::new(0),
            projection_mismatch: AtomicU64::new(0),
        })
    }

    pub fn mark_enqueued(&self, operation: u64) {
        let started = Instant::now();
        self.completion.mark_started(operation, started);
    }

    pub fn observe_dequeue(&self, operation: u64, channel: usize, seq: u64) {
        if channel >= self.channels {
            self.wrong_channel.fetch_add(1, Ordering::Relaxed);
        }
        let key = (channel, seq);
        if !self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key)
        {
            self.duplicates.fetch_add(1, Ordering::Relaxed);
        }
        let mut last = self.last_seq.lock().unwrap_or_else(|e| e.into_inner());
        let expected = last.get(&channel).copied().unwrap_or(0) + 1;
        if seq != expected {
            self.order_violations.fetch_add(1, Ordering::Relaxed);
        }
        if seq < expected {
            self.cursor_regressions.fetch_add(1, Ordering::Relaxed);
        }
        last.insert(channel, seq.max(expected.saturating_sub(1)));
        drop(last);
        self.trace
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(json!({
                "operation": operation, "channel": channel, "seq": seq,
                "hop": "core.dequeue", "elapsedUs": 0
            }));
    }

    pub fn mark_port(&self, correlation: u64, operation: u64, stage: &'static str) {
        self.port_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(correlation, (Instant::now(), operation, stage));
    }

    pub fn observe_port_reply(&self, correlation: u64) {
        let started = self
            .port_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&correlation);
        if let Some((started, operation, stage)) = started {
            let elapsed = started.elapsed();
            if let Some(histogram) = self
                .port_latency
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(stage)
            {
                let _ = histogram.record(elapsed.as_micros().max(1) as u64);
            }
            self.trace_hop(operation, stage, elapsed);
            let bit = match stage {
                "storage_port_roundtrip" => STORAGE_DONE,
                "http_port_roundtrip" => HTTP_DONE,
                _ => 0,
            };
            self.mark_completion(operation, bit);
        }
    }

    pub fn observe_ffi_complete(&self, operation: u64) {
        self.mark_completion(operation, FFI_DONE);
    }

    fn mark_completion(&self, operation: u64, bit: u8) {
        if bit == 0 {
            return;
        }
        let Some(elapsed) = self.completion.mark_stage(operation, bit) else {
            self.projection_mismatch.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if !elapsed.is_zero() {
            self.trace_hop(operation, "oracle.complete", elapsed);
        }
    }

    pub async fn wait_complete(&self, expected: u64, timeout: std::time::Duration) -> bool {
        self.completion.wait(expected, timeout).await
    }

    pub fn completed(&self) -> u64 {
        self.completion.completed()
    }

    pub fn mark_event(&self, operation: u64) {
        self.event_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(operation, Instant::now());
        self.trace_hop(operation, "event.emit", std::time::Duration::ZERO);
    }

    pub fn observe_event_delivery(&self, operation: u64) {
        if let Some(started) = self
            .event_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&operation)
        {
            let elapsed = started.elapsed();
            let _ = self
                .event_delivery
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record(elapsed.as_micros().max(1) as u64);
            self.trace_hop(operation, "event.dispatch", elapsed);
        }
    }

    pub fn correctness(
        &self,
        expected: u64,
        rows: u64,
        cleanup_residue: u64,
    ) -> BTreeMap<String, u64> {
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner()).len() as u64;
        BTreeMap::from([
            ("helix_lost_total".into(), expected.saturating_sub(seen)),
            (
                "helix_duplicate_total".into(),
                self.duplicates.load(Ordering::Relaxed),
            ),
            (
                "helix_order_violation_total".into(),
                self.order_violations.load(Ordering::Relaxed),
            ),
            (
                "helix_wrong_channel_total".into(),
                self.wrong_channel.load(Ordering::Relaxed),
            ),
            (
                "helix_cursor_regression_total".into(),
                self.cursor_regressions.load(Ordering::Relaxed),
            ),
            (
                "helix_projection_mismatch_total".into(),
                self.projection_mismatch.load(Ordering::Relaxed),
            ),
            (
                "helix_persist_remaining_total".into(),
                expected.saturating_sub(rows),
            ),
            ("helix_cleanup_residue_total".into(), cleanup_residue),
        ])
    }

    pub fn observe_ffi_callback(&self, elapsed: std::time::Duration) {
        let _ = self
            .ffi_callback
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(elapsed.as_micros().max(1) as u64);
    }

    pub fn attach_histograms(&self, histograms: &mut BTreeMap<String, Histogram<u64>>) {
        for (stage, histogram) in self
            .port_latency
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            histograms.insert((*stage).into(), histogram.clone());
        }
        histograms.insert(
            "event_dispatch".into(),
            self.event_delivery
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        );
        histograms.insert(
            "ffi_callback".into(),
            self.ffi_callback
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        );
        histograms.insert("end_to_end".into(), self.completion.histogram());
    }

    pub fn trace_hop(&self, operation: u64, hop: &'static str, elapsed: std::time::Duration) {
        self.trace
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(json!({
                "operation": operation,
                "hop": hop,
                "elapsedUs": elapsed.as_micros()
            }));
    }

    pub fn trace(&self) -> Vec<Value> {
        self.trace.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
