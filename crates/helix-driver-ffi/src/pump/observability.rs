use helix_driver_host::engine::{TransportTraceEvent, TransportTraceSink, TransportTraceStats};
use helix_driver_host::{CommandTraceQueue, HostOtelRuntime, TraceDirection, TraceHooks};
use tokio::sync::mpsc;

pub(super) fn ffi_trace_hooks(
    command_traces: CommandTraceQueue,
    runtime: HostOtelRuntime,
) -> TraceHooks {
    TraceHooks::noop_without_otel()
        .with_otel(runtime)
        .with_command_traces(command_traces)
}

pub(super) fn spawn_transport_trace_exporter(
    runtime: &HostOtelRuntime,
) -> Option<TransportTraceSink> {
    if !runtime.is_enabled() {
        return None;
    }

    let (sink, rx) = TransportTraceSink::channel();
    let stats = sink.stats();
    let runtime = runtime.clone();
    match std::thread::Builder::new()
        .name("helix-ffi-trace-exporter".to_string())
        .spawn(move || export_transport_trace_events(rx, runtime, stats))
    {
        Ok(_) => Some(sink),
        Err(error) => {
            tracing::warn!(error = %error, "FFI pump 启动 trace exporter 线程失败，跳过 transport trace 导出");
            None
        }
    }
}

fn export_transport_trace_events(
    mut rx: mpsc::Receiver<TransportTraceEvent>,
    runtime: HostOtelRuntime,
    stats: TransportTraceStats,
) {
    while let Some(event) = rx.blocking_recv() {
        let attrs = transport_trace_attributes(&event);
        let _span = runtime.span_with_attributes(event.name, TraceDirection::Internal, None, attrs);
    }

    let transport_dropped = stats.dropped_count();
    let span_dropped = runtime.dropped_span_count();
    if transport_dropped > 0 || span_dropped > 0 {
        tracing::warn!(transport_dropped, span_dropped, "FFI OTel 有界队列发生丢弃");
    }
}

fn transport_trace_attributes(event: &TransportTraceEvent) -> Vec<(&'static str, String)> {
    let mut attrs = vec![
        ("transport.id", event.transport_id.raw().to_string()),
        ("ws.lifecycle.action", event.action.to_string()),
    ];
    if let Some(attempt) = event.attempt {
        attrs.push(("ws.reconnect.attempt", attempt.to_string()));
    }
    if let Some(delay_ms) = event.delay_ms {
        attrs.push(("ws.reconnect.delay_ms", delay_ms.to_string()));
    }
    if let Some(next_delay_ms) = event.next_delay_ms {
        attrs.push(("ws.reconnect.next_delay_ms", next_delay_ms.to_string()));
    }
    if let Some(reason) = event.reason {
        attrs.push(("ws.lifecycle.reason", reason.to_string()));
    }
    if let Some(error_class) = &event.error_class {
        attrs.push(("ws.error.class", error_class.clone()));
    }
    attrs
}

#[cfg(test)]
mod tests {
    use helix_core::effect::TransportId;
    use helix_driver_host::{CommandTraceQueue, HostOtelConfig, HostOtelRuntime};

    use super::{ffi_trace_hooks, spawn_transport_trace_exporter, transport_trace_attributes};
    use helix_driver_host::engine::TransportTraceEvent;

    #[test]
    fn disabled_runtime_keeps_ffi_trace_noop() {
        let runtime = HostOtelRuntime::new(HostOtelConfig::default());
        assert!(spawn_transport_trace_exporter(&runtime).is_none());
        assert!(!ffi_trace_hooks(CommandTraceQueue::default(), runtime).is_otel_enabled());
    }

    #[test]
    fn enabled_runtime_is_explicitly_installed_without_collector_io() {
        let runtime = HostOtelRuntime::new(HostOtelConfig {
            enabled: true,
            service_name: "helix-driver-ffi-test".to_string(),
            endpoint: "noop".to_string(),
            protocol: "noop".to_string(),
        });
        assert!(ffi_trace_hooks(CommandTraceQueue::default(), runtime).is_otel_enabled());
    }

    #[test]
    fn transport_trace_attributes_are_safe_and_complete() {
        let attrs = transport_trace_attributes(&TransportTraceEvent {
            transport_id: TransportId::from_raw(7),
            name: "helix.ws.reconnect.schedule",
            action: "reconnect_schedule",
            attempt: Some(2),
            delay_ms: Some(400),
            next_delay_ms: Some(800),
            reason: Some("reader_closed_or_error"),
            error_class: Some("transport".to_string()),
        });
        assert!(attrs.contains(&("transport.id", "7".to_string())));
        assert!(attrs.contains(&("ws.reconnect.attempt", "2".to_string())));
        assert!(attrs.contains(&("ws.reconnect.next_delay_ms", "800".to_string())));

        let serialized = format!("{attrs:?}").to_lowercase();
        for forbidden in ["cookie", "authorization", "token", "body", "raw_payload"] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
