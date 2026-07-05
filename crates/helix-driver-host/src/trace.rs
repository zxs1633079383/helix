use std::collections::VecDeque;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;
use serde::Deserialize;
use thiserror::Error;

use helix_core::effect::{Correlation, DomainEventBytes, HttpRequest, StorageOp, TransportId};
use helix_core::Tick;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceCarrier {
    pub traceparent: Option<String>,
    pub baggage: Option<String>,
    pub raw_json: Option<String>,
}

impl TraceCarrier {
    pub fn from_json_str(raw: &str) -> Result<Self, TraceCarrierError> {
        let wire: TraceCarrierWire =
            serde_json::from_str(raw).map_err(|e| TraceCarrierError::Parse(e.to_string()))?;
        Ok(Self {
            traceparent: wire.traceparent.filter(|v| !v.is_empty()),
            baggage: wire.baggage.filter(|v| !v.is_empty()),
            raw_json: Some(raw.to_string()),
        })
    }
}

#[derive(Debug, Error)]
pub enum TraceCarrierError {
    #[error("trace sidecar JSON parse failed: {0}")]
    Parse(String),
}

#[derive(Debug, Deserialize)]
struct TraceCarrierWire {
    #[serde(default)]
    traceparent: Option<String>,
    #[serde(default)]
    baggage: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CommandTraceQueue {
    inner: Arc<Mutex<VecDeque<Option<TraceCarrier>>>>,
}

impl CommandTraceQueue {
    pub fn push_slot(&self, carrier: Option<TraceCarrier>) {
        self.inner.lock().push_back(carrier);
    }

    pub fn rollback_last(&self) {
        let _ = self.inner.lock().pop_back();
    }

    pub fn pop_next(&self) -> Option<TraceCarrier> {
        self.inner.lock().pop_front().flatten()
    }

    #[cfg(test)]
    fn len_for_test(&self) -> usize {
        self.inner.lock().len()
    }
}

pub trait TraceHooksImpl {
    fn on_tick_start(&self, tick: &Tick, carrier: Option<TraceCarrier>) -> TraceScope;
    fn on_storage_dispatch(&self, corr: Option<Correlation>, ops: &[StorageOp]);
    fn on_http_dispatch(&self, corr: Option<Correlation>, req: &mut HttpRequest);
    fn on_ws_send(&self, transport: TransportId, frame: &mut Bytes);
    fn on_event_emit(&self, event: &DomainEventBytes);
}

pub struct TraceScope {
    end: Option<Box<dyn FnOnce() + Send>>,
}

impl TraceScope {
    pub fn noop() -> Self {
        Self { end: None }
    }

    pub fn new(end: impl FnOnce() + Send + 'static) -> Self {
        Self {
            end: Some(Box::new(end)),
        }
    }
}

impl Drop for TraceScope {
    fn drop(&mut self) {
        if let Some(end) = self.end.take() {
            end();
        }
    }
}

#[derive(Clone)]
pub struct TraceHooks {
    inner: Arc<dyn TraceHooksImpl + Send + Sync>,
    command_traces: Option<CommandTraceQueue>,
}

impl TraceHooks {
    pub fn new(inner: impl TraceHooksImpl + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(inner),
            command_traces: None,
        }
    }

    pub fn noop() -> Self {
        Self::new(NoopTraceHooks)
    }

    pub fn with_command_traces(mut self, command_traces: CommandTraceQueue) -> Self {
        self.command_traces = Some(command_traces);
        self
    }

    pub fn on_tick_start(&self, tick: &Tick) -> TraceScope {
        let carrier = match tick {
            Tick::Command(_) => self
                .command_traces
                .as_ref()
                .and_then(CommandTraceQueue::pop_next),
            _ => None,
        };
        self.inner.on_tick_start(tick, carrier)
    }

    pub fn on_storage_dispatch(&self, corr: Option<Correlation>, ops: &[StorageOp]) {
        self.inner.on_storage_dispatch(corr, ops);
    }

    pub fn on_http_dispatch(&self, corr: Option<Correlation>, req: &mut HttpRequest) {
        self.inner.on_http_dispatch(corr, req);
    }

    pub fn on_ws_send(&self, transport: TransportId, frame: &mut Bytes) {
        self.inner.on_ws_send(transport, frame);
    }

    pub fn on_event_emit(&self, event: &DomainEventBytes) {
        self.inner.on_event_emit(event);
    }
}

struct NoopTraceHooks;

impl TraceHooksImpl for NoopTraceHooks {
    fn on_tick_start(&self, _: &Tick, _: Option<TraceCarrier>) -> TraceScope {
        TraceScope::noop()
    }

    fn on_storage_dispatch(&self, _: Option<Correlation>, _: &[StorageOp]) {}

    fn on_http_dispatch(&self, _: Option<Correlation>, _: &mut HttpRequest) {}

    fn on_ws_send(&self, _: TransportId, _: &mut Bytes) {}

    fn on_event_emit(&self, _: &DomainEventBytes) {}
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use parking_lot::Mutex;

    use helix_core::effect::TransportId;
    use helix_core::tick::AppCommand;

    use super::*;

    #[test]
    fn trace_carrier_parses_w3c_json() {
        let carrier = TraceCarrier::from_json_str(
            r#"{"traceparent":"00-00000000000000000000000000000001-0000000000000002-01","baggage":"tenant=im"}"#,
        )
        .expect("valid carrier");
        assert_eq!(
            carrier.traceparent.as_deref(),
            Some("00-00000000000000000000000000000001-0000000000000002-01")
        );
        assert_eq!(carrier.baggage.as_deref(), Some("tenant=im"));
        assert!(carrier.raw_json.as_deref().unwrap().contains("traceparent"));
    }

    #[test]
    fn command_trace_queue_preserves_plain_and_traced_slots() {
        let queue = CommandTraceQueue::default();
        queue.push_slot(None);
        queue.push_slot(Some(
            TraceCarrier::from_json_str(r#"{"traceparent":"00-1-2-01"}"#).expect("carrier"),
        ));

        assert_eq!(queue.len_for_test(), 2);
        assert!(queue.pop_next().is_none());
        assert_eq!(queue.len_for_test(), 1);
        assert_eq!(
            queue.pop_next().and_then(|carrier| carrier.traceparent),
            Some("00-1-2-01".to_string())
        );
    }

    #[derive(Default)]
    struct RecordingHooks {
        seen_traceparents: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl TraceHooksImpl for RecordingHooks {
        fn on_tick_start(&self, _: &Tick, carrier: Option<TraceCarrier>) -> TraceScope {
            self.seen_traceparents
                .lock()
                .push(carrier.and_then(|c| c.traceparent));
            TraceScope::noop()
        }

        fn on_storage_dispatch(&self, _: Option<Correlation>, _: &[StorageOp]) {}

        fn on_http_dispatch(&self, _: Option<Correlation>, _: &mut HttpRequest) {}

        fn on_ws_send(&self, _: TransportId, _: &mut Bytes) {}

        fn on_event_emit(&self, _: &DomainEventBytes) {}
    }

    #[test]
    fn trace_hooks_consume_command_sidecars_in_tick_order() {
        let recording = RecordingHooks::default();
        let seen = Arc::clone(&recording.seen_traceparents);
        let queue = CommandTraceQueue::default();
        queue.push_slot(None);
        queue.push_slot(Some(
            TraceCarrier::from_json_str(r#"{"traceparent":"00-aa-bb-01"}"#).expect("carrier"),
        ));
        let hooks = TraceHooks::new(recording).with_command_traces(queue);

        let plain = Tick::Command(AppCommand::new("plain", Bytes::from_static(b"{}")));
        let traced = Tick::Command(AppCommand::new("traced", Bytes::from_static(b"{}")));

        let _plain_scope = hooks.on_tick_start(&plain);
        let _traced_scope = hooks.on_tick_start(&traced);

        assert_eq!(*seen.lock(), vec![None, Some("00-aa-bb-01".to_string())]);
    }
}
