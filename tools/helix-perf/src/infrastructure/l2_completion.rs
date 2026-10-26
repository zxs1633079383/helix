use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use tokio::sync::Notify;

pub const STORAGE_DONE: u8 = 1;
pub const HTTP_DONE: u8 = 2;
pub const FFI_DONE: u8 = 4;
const COMPLETE_MASK: u8 = STORAGE_DONE | HTTP_DONE | FFI_DONE;

pub struct CompletionOracle {
    started: Mutex<HashMap<u64, Instant>>,
    states: Mutex<HashMap<u64, u8>>,
    latency: Mutex<Histogram<u64>>,
    notify: Notify,
    completed: AtomicU64,
}

impl CompletionOracle {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            started: Mutex::new(HashMap::new()),
            states: Mutex::new(HashMap::new()),
            latency: Mutex::new(Histogram::new(3).map_err(|error| error.to_string())?),
            notify: Notify::new(),
            completed: AtomicU64::new(0),
        })
    }

    pub fn mark_started(&self, operation: u64, started: Instant) {
        self.started
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(operation, started);
        self.states
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(operation, 0);
    }

    pub fn mark_stage(&self, operation: u64, bit: u8) -> Option<Duration> {
        let completed_now = {
            let mut states = self
                .states
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let state = states.get_mut(&operation)?;
            *state |= bit;
            if *state == COMPLETE_MASK {
                states.remove(&operation);
                true
            } else {
                false
            }
        };
        if !completed_now {
            return Some(Duration::ZERO);
        }
        let started = self
            .started
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&operation)?;
        let elapsed = started.elapsed();
        let _ = self
            .latency
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .record(elapsed.as_micros().max(1) as u64);
        self.completed.fetch_add(1, Ordering::Release);
        self.notify.notify_waiters();
        Some(elapsed)
    }

    pub async fn wait(&self, expected: u64, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                let notified = self.notify.notified();
                if self.completed.load(Ordering::Acquire) >= expected {
                    return;
                }
                notified.await;
            }
        })
        .await
        .is_ok()
    }

    pub fn completed(&self) -> u64 {
        self.completed.load(Ordering::Acquire)
    }

    pub fn histogram(&self) -> Histogram<u64> {
        self.latency
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}
