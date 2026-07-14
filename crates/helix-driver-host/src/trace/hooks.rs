use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use super::carrier::{CommandTraceQueue, TraceCarrier};
use crate::otel::{HostOtelRuntime, TraceDirection};
use crate::storage::StorageTraceContext;
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TransportId,
};
use helix_core::Tick;

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
        Self::new(NoopTraceHooks)
    }

    pub fn noop_without_otel() -> Self {
        Self::noop()
    }

    pub fn with_command_traces(mut self, command_traces: CommandTraceQueue) -> Self {
        self.command_traces = Some(command_traces);
        self
    }

    pub fn with_otel(mut self, otel: HostOtelRuntime) -> Self {
        self.otel = if otel.is_enabled() { Some(otel) } else { None };
        self
    }

    /// composition root / 测试可用的装配状态；不暴露 exporter 或业务数据。
    pub fn is_otel_enabled(&self) -> bool {
        self.otel.is_some()
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

        let traced_scope = scope
            .chain(restore_scope)
            .chain(TraceScope::with_guard(runtime.span(
                "helix.core.step",
                TraceDirection::Internal,
                carrier.as_ref(),
            )));

        let Some((name, direction)) = (match tick {
            Tick::Command(_) => Some(("helix.command.accept", TraceDirection::Inbound)),
            Tick::Inbound(_) => Some(("helix.ws.recv", TraceDirection::Inbound)),
            _ => None,
        }) else {
            return traced_scope;
        };

        traced_scope.chain(TraceScope::with_guard(runtime.span(
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
