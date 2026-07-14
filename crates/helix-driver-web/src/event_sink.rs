//! Step-batched Web domain event sink.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;

pub const EVENT_BATCH_VERSION: u16 = 2;

#[derive(Debug, Clone)]
pub struct EventBatch {
    pub version: u16,
    pub events: Vec<Bytes>,
    /// Number of older, undrained batches evicted before this batch.
    pub dropped_batches_before: u64,
    /// Number of events omitted from this step after its event cap was reached.
    pub dropped_events: u64,
}

#[derive(Debug)]
struct EventState {
    pending: Vec<Bytes>,
    dropped_pending_events: u64,
    ready: VecDeque<EventBatch>,
}

/// Collects every `Emit` from one shell step and publishes exactly one batch.
#[derive(Debug, Clone)]
pub struct WebEventSink {
    state: Arc<Mutex<EventState>>,
    max_ready_batches: usize,
    max_events_per_batch: usize,
}

impl WebEventSink {
    pub fn new(max_ready_batches: usize, max_events_per_batch: usize) -> Result<Self, String> {
        if max_ready_batches == 0 || max_events_per_batch == 0 {
            return Err("Web event queue limits must be greater than zero".into());
        }
        Ok(Self {
            state: Arc::new(Mutex::new(EventState {
                pending: Vec::new(),
                dropped_pending_events: 0,
                ready: VecDeque::with_capacity(max_ready_batches.min(64)),
            })),
            max_ready_batches,
            max_events_per_batch,
        })
    }

    pub fn flush_step(&self) {
        let Ok(mut state) = self.state.lock() else {
            tracing::error!("Web event sink lock poisoned; step batch unavailable");
            return;
        };
        if state.pending.is_empty() && state.dropped_pending_events == 0 {
            return;
        }
        let events = std::mem::take(&mut state.pending);
        let dropped_events = std::mem::take(&mut state.dropped_pending_events);
        let mut batch = EventBatch {
            version: EVENT_BATCH_VERSION,
            events,
            dropped_batches_before: 0,
            dropped_events,
        };
        if state.ready.len() >= self.max_ready_batches {
            let evicted = state
                .ready
                .pop_front()
                .expect("bounded event queue is non-empty at capacity");
            let dropped = evicted.dropped_batches_before.saturating_add(1);
            if let Some(oldest) = state.ready.front_mut() {
                oldest.dropped_batches_before =
                    oldest.dropped_batches_before.saturating_add(dropped);
            } else {
                batch.dropped_batches_before = dropped;
            }
        }
        state.ready.push_back(batch);
    }

    pub fn take_batches(&self) -> Vec<EventBatch> {
        let Ok(mut state) = self.state.lock() else {
            tracing::error!("Web event sink lock poisoned; cannot drain batches");
            return Vec::new();
        };
        state.ready.drain(..).collect()
    }
}

impl EventSink for WebEventSink {
    fn emit(&self, event: DomainEventBytes) {
        if let Ok(mut state) = self.state.lock() {
            if state.pending.len() < self.max_events_per_batch {
                state.pending.push(event.0);
            } else {
                state.dropped_pending_events = state.dropped_pending_events.saturating_add(1);
            }
        } else {
            tracing::error!("Web event sink lock poisoned; event unavailable");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_step_is_one_versioned_batch_and_empty_flush_is_noop() {
        let sink = WebEventSink::new(4, 4).expect("sink");
        sink.emit(DomainEventBytes(Bytes::from_static(b"one")));
        sink.emit(DomainEventBytes(Bytes::from_static(b"two")));
        sink.flush_step();
        sink.flush_step();

        let batches = sink.take_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].version, EVENT_BATCH_VERSION);
        assert_eq!(batches[0].events[0].as_ref(), b"one");
        assert_eq!(batches[0].events[1].as_ref(), b"two");
        assert_eq!(batches[0].dropped_batches_before, 0);
        assert_eq!(batches[0].dropped_events, 0);
    }

    #[test]
    fn undrained_batches_and_oversized_steps_are_bounded_with_gap_markers() {
        let sink = WebEventSink::new(2, 1).expect("sink");
        for value in [&b"one"[..], &b"two"[..], &b"three"[..]] {
            sink.emit(DomainEventBytes(Bytes::from_static(value)));
            sink.emit(DomainEventBytes(Bytes::from_static(b"dropped")));
            sink.flush_step();
        }

        let batches = sink.take_batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].events[0].as_ref(), b"two");
        assert_eq!(batches[0].dropped_batches_before, 1);
        assert_eq!(batches[0].dropped_events, 1);
        assert_eq!(batches[1].events[0].as_ref(), b"three");
        assert_eq!(batches[1].dropped_batches_before, 0);
        assert_eq!(batches[1].dropped_events, 1);
    }
}
