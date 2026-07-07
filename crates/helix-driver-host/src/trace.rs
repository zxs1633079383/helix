use std::collections::VecDeque;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;
use serde::Deserialize;
use thiserror::Error;

use crate::otel::{HostOtelRuntime, TraceDirection};
use crate::storage::StorageTraceContext;
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TransportId,
};
use helix_core::Tick;

pub const TRACE_CARRIER_JSON_MAX_BYTES: usize = 16 * 1024;
pub const TRACEPARENT_BYTES: usize = 55;
pub const TRACE_BAGGAGE_MAX_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceCarrier {
    pub traceparent: Option<String>,
    pub baggage: Option<String>,
    pub raw_json: Option<String>,
}

impl TraceCarrier {
    pub fn from_json_str(raw: &str) -> Result<Self, TraceCarrierError> {
        if raw.len() > TRACE_CARRIER_JSON_MAX_BYTES {
            return Err(TraceCarrierError::TooLarge {
                field: "trace_json",
                max: TRACE_CARRIER_JSON_MAX_BYTES,
                actual: raw.len(),
            });
        }
        let wire: TraceCarrierWire =
            serde_json::from_str(raw).map_err(|e| TraceCarrierError::Parse(e.to_string()))?;
        let traceparent = match wire.traceparent.filter(|v| !v.is_empty()) {
            Some(value) if value.len() > TRACEPARENT_BYTES => {
                return Err(TraceCarrierError::TooLarge {
                    field: "traceparent",
                    max: TRACEPARENT_BYTES,
                    actual: value.len(),
                });
            }
            Some(value) if parse_trace_id(&value).is_none() => {
                return Err(TraceCarrierError::InvalidTraceparent);
            }
            value => value,
        };
        let baggage = match wire.baggage.filter(|v| !v.is_empty()) {
            Some(value) if value.len() > TRACE_BAGGAGE_MAX_BYTES => {
                return Err(TraceCarrierError::TooLarge {
                    field: "baggage",
                    max: TRACE_BAGGAGE_MAX_BYTES,
                    actual: value.len(),
                });
            }
            value => value,
        };
        Ok(Self {
            traceparent,
            baggage,
            raw_json: Some(raw.to_string()),
        })
    }

    pub fn from_headers(headers: &[(String, String)]) -> Option<Self> {
        let traceparent = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("traceparent"))
            .and_then(|(_, value)| bounded_traceparent(value));
        let baggage = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("baggage"))
            .and_then(|(_, value)| bounded_baggage(value));

        if traceparent.is_none() && baggage.is_none() {
            return None;
        }

        Some(Self {
            traceparent,
            baggage,
            raw_json: None,
        })
    }
}

#[derive(Debug, Error)]
pub enum TraceCarrierError {
    #[error("trace sidecar JSON parse failed: {0}")]
    Parse(String),
    #[error("{field} exceeds {max} bytes: {actual}")]
    TooLarge {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("traceparent is not valid W3C version 00 format")]
    InvalidTraceparent,
}

#[derive(Debug, Deserialize)]
struct TraceCarrierWire {
    #[serde(default)]
    traceparent: Option<String>,
    #[serde(default)]
    baggage: Option<String>,
}

pub(crate) fn parse_trace_id(traceparent: &str) -> Option<String> {
    if !is_valid_w3c_traceparent(traceparent) {
        return None;
    }
    Some(traceparent[3..35].to_string())
}

fn bounded_traceparent(value: &str) -> Option<String> {
    if value.len() > TRACEPARENT_BYTES || parse_trace_id(value).is_none() {
        return None;
    }
    Some(value.to_string())
}

fn bounded_baggage(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > TRACE_BAGGAGE_MAX_BYTES {
        return None;
    }
    Some(value.to_string())
}

fn is_valid_w3c_traceparent(value: &str) -> bool {
    if value.len() != TRACEPARENT_BYTES {
        return false;
    }
    let bytes = value.as_bytes();
    if &bytes[0..2] != b"00" || bytes[2] != b'-' || bytes[35] != b'-' || bytes[52] != b'-' {
        return false;
    }

    let trace_id = &value[3..35];
    let parent_id = &value[36..52];
    let flags = &value[53..55];
    all_lower_hex(trace_id)
        && all_lower_hex(parent_id)
        && all_lower_hex(flags)
        && !all_zero(trace_id)
        && !all_zero(parent_id)
}

fn all_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn all_zero(value: &str) -> bool {
    value.bytes().all(|b| b == b'0')
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
    fn on_upload_dispatch(&self, corr: Option<Correlation>, req: &FileUploadRequest);
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

    pub fn with_guard<G>(guard: G) -> Self
    where
        G: Send + 'static,
    {
        Self::new(move || drop(guard))
    }

    pub fn chain(self, other: Self) -> Self {
        Self::new(move || {
            drop(other);
            drop(self);
        })
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
    otel: Option<HostOtelRuntime>,
    active_carrier: Arc<Mutex<Option<TraceCarrier>>>,
}

impl TraceHooks {
    pub fn new(inner: impl TraceHooksImpl + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::new(inner),
            command_traces: None,
            otel: None,
            active_carrier: Arc::new(Mutex::new(None)),
        }
    }

    pub fn noop() -> Self {
        Self::new(NoopTraceHooks).with_otel(HostOtelRuntime::from_env("helix-driver-host"))
    }

    pub fn with_command_traces(mut self, command_traces: CommandTraceQueue) -> Self {
        self.command_traces = Some(command_traces);
        self
    }

    pub fn with_otel(mut self, otel: HostOtelRuntime) -> Self {
        self.otel = if otel.is_enabled() { Some(otel) } else { None };
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
        let previous = self.active_carrier();
        *self.active_carrier.lock() = carrier.clone();
        let scope = self.inner.on_tick_start(tick, carrier.clone());
        let active_carrier = Arc::clone(&self.active_carrier);
        let restore_scope = TraceScope::new(move || {
            *active_carrier.lock() = previous;
        });
        let Some(runtime) = self.otel.as_ref() else {
            return scope.chain(restore_scope);
        };

        let (name, direction) = match tick {
            Tick::Command(_) => ("helix.command.accept", TraceDirection::Inbound),
            Tick::Inbound(_) => ("helix.ws.recv", TraceDirection::Inbound),
            _ => ("helix.core.step", TraceDirection::Internal),
        };

        scope
            .chain(restore_scope)
            .chain(TraceScope::with_guard(runtime.span(
                name,
                direction,
                carrier.as_ref(),
            )))
    }

    fn active_carrier(&self) -> Option<TraceCarrier> {
        self.active_carrier.lock().clone()
    }

    pub fn on_storage_dispatch(&self, corr: Option<Correlation>, ops: &[StorageOp]) {
        let _scope = self.otel.as_ref().and_then(|runtime| {
            (!ops.is_empty()).then(|| {
                let carrier = self.active_carrier();
                runtime.span(
                    "helix.storage.persist",
                    TraceDirection::Internal,
                    carrier.as_ref(),
                )
            })
        });
        self.inner.on_storage_dispatch(corr, ops);
    }

    pub fn storage_trace_context(&self) -> Option<StorageTraceContext> {
        self.otel
            .as_ref()
            .map(|runtime| StorageTraceContext::new(runtime.clone(), self.active_carrier()))
    }

    pub fn on_http_dispatch(&self, corr: Option<Correlation>, req: &mut HttpRequest) {
        let active = self.active_carrier();
        if let Some(traceparent) = active
            .as_ref()
            .and_then(|carrier| carrier.traceparent.clone())
        {
            if !req
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("traceparent"))
            {
                req.headers.push(("traceparent".to_string(), traceparent));
            }
        }
        let carrier = TraceCarrier::from_headers(&req.headers).or_else(|| active.clone());
        let _scope = self.otel.as_ref().map(|runtime| {
            runtime.span(
                "helix.http.dispatch",
                TraceDirection::Outbound,
                carrier.as_ref(),
            )
        });
        self.inner.on_http_dispatch(corr, req);
    }

    pub fn on_upload_dispatch(&self, corr: Option<Correlation>, req: &FileUploadRequest) {
        let carrier = self.active_carrier();
        let _scope = self.otel.as_ref().map(|runtime| {
            runtime.span(
                "helix.upload.dispatch",
                TraceDirection::Outbound,
                carrier.as_ref(),
            )
        });
        self.inner.on_upload_dispatch(corr, req);
    }

    pub fn on_ws_send(&self, transport: TransportId, frame: &mut Bytes) {
        let carrier = self.active_carrier();
        let _scope = self.otel.as_ref().map(|runtime| {
            runtime.span("helix.ws.send", TraceDirection::Outbound, carrier.as_ref())
        });
        self.inner.on_ws_send(transport, frame);
    }

    pub fn on_event_emit(&self, event: &DomainEventBytes) {
        let carrier = self.active_carrier();
        let _scope = self.otel.as_ref().map(|runtime| {
            runtime.span(
                "helix.event.emit",
                TraceDirection::Outbound,
                carrier.as_ref(),
            )
        });
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

    fn on_upload_dispatch(&self, _: Option<Correlation>, _: &FileUploadRequest) {}

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
    fn trace_carrier_rejects_invalid_w3c_traceparent_json() {
        for traceparent in [
            "00-1-2-01",
            "00-00000000000000000000000000000000-0000000000000001-01",
            "00-00000000000000000000000000000001-0000000000000000-01",
            "00-0000000000000000000000000000000A-0000000000000002-01",
        ] {
            let raw = format!(r#"{{"traceparent":"{traceparent}"}}"#);
            assert!(
                matches!(
                    TraceCarrier::from_json_str(&raw),
                    Err(TraceCarrierError::InvalidTraceparent)
                ),
                "{traceparent} should be rejected"
            );
        }

        let extra_segment =
            r#"{"traceparent":"00-00000000000000000000000000000001-0000000000000002-01-extra"}"#;
        assert!(matches!(
            TraceCarrier::from_json_str(extra_segment),
            Err(TraceCarrierError::TooLarge {
                field: "traceparent",
                ..
            })
        ));
        assert!(
            parse_trace_id("00-00000000000000000000000000000001-0000000000000002-01-extra")
                .is_none()
        );
    }

    #[test]
    fn trace_carrier_enforces_json_and_baggage_bounds() {
        let oversized_json = "x".repeat(TRACE_CARRIER_JSON_MAX_BYTES + 1);
        assert!(matches!(
            TraceCarrier::from_json_str(&oversized_json),
            Err(TraceCarrierError::TooLarge {
                field: "trace_json",
                ..
            })
        ));

        let raw = format!(
            r#"{{"baggage":"{}"}}"#,
            "a".repeat(TRACE_BAGGAGE_MAX_BYTES + 1)
        );
        assert!(matches!(
            TraceCarrier::from_json_str(&raw),
            Err(TraceCarrierError::TooLarge {
                field: "baggage",
                ..
            })
        ));
    }

    #[test]
    fn trace_carrier_drops_invalid_or_oversized_headers() {
        let headers = vec![
            (
                "traceparent".to_string(),
                "00-00000000000000000000000000000000-0000000000000001-01".to_string(),
            ),
            ("baggage".to_string(), "tenant=im".to_string()),
        ];
        let carrier = TraceCarrier::from_headers(&headers).expect("baggage survives");
        assert_eq!(carrier.traceparent, None);
        assert_eq!(carrier.baggage.as_deref(), Some("tenant=im"));

        let oversized = vec![
            ("traceparent".to_string(), "0".repeat(TRACEPARENT_BYTES + 1)),
            (
                "baggage".to_string(),
                "a".repeat(TRACE_BAGGAGE_MAX_BYTES + 1),
            ),
        ];
        assert_eq!(TraceCarrier::from_headers(&oversized), None);
    }

    #[test]
    fn command_trace_queue_preserves_plain_and_traced_slots() {
        let queue = CommandTraceQueue::default();
        queue.push_slot(None);
        queue.push_slot(Some(
            TraceCarrier::from_json_str(
                r#"{"traceparent":"00-00000000000000000000000000000001-0000000000000002-01"}"#,
            )
            .expect("carrier"),
        ));

        assert_eq!(queue.len_for_test(), 2);
        assert!(queue.pop_next().is_none());
        assert_eq!(queue.len_for_test(), 1);
        assert_eq!(
            queue.pop_next().and_then(|carrier| carrier.traceparent),
            Some("00-00000000000000000000000000000001-0000000000000002-01".to_string())
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

        fn on_upload_dispatch(&self, _: Option<Correlation>, _: &FileUploadRequest) {}

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
            TraceCarrier::from_json_str(
                r#"{"traceparent":"00-00000000000000000000000000000002-0000000000000003-01"}"#,
            )
            .expect("carrier"),
        ));
        let hooks = TraceHooks::new(recording).with_command_traces(queue);

        let plain = Tick::Command(AppCommand::new("plain", Bytes::from_static(b"{}")));
        let traced = Tick::Command(AppCommand::new("traced", Bytes::from_static(b"{}")));

        let _plain_scope = hooks.on_tick_start(&plain);
        let _traced_scope = hooks.on_tick_start(&traced);

        assert_eq!(
            *seen.lock(),
            vec![
                None,
                Some("00-00000000000000000000000000000002-0000000000000003-01".to_string())
            ]
        );
    }
}
