use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::lifecycle::{
    LifecycleCapability, LifecycleContext, LifecycleStage, LifecycleStatus, LifecycleTraceSink,
};
use crate::otel::{HostOtelConfig, HostOtelRuntime};
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TransportId,
};
use helix_core::tick::AppCommand;
use helix_core::Tick;

use super::*;

#[test]
fn noop_trace_hooks_do_not_enable_otel() {
    assert!(!TraceHooks::noop().is_otel_enabled());
}

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
        parse_trace_id("00-00000000000000000000000000000001-0000000000000002-01-extra").is_none()
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

/// T4 缺少 WS 能力时只记录状态，不创建网络 Span。
#[test]
fn absent_ws_reports_not_applicable_without_creating_network_span() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });
    let (sink, mut observations) = LifecycleTraceSink::channel();
    let hooks = TraceHooks::noop()
        .with_otel(runtime.clone())
        .with_lifecycle_sink(sink);
    let context = LifecycleContext::new(9, None, None);

    hooks.emit_lifecycle_status(
        &context,
        LifecycleStage::T4,
        Some(LifecycleCapability::Ws),
        LifecycleStatus::NotApplicable,
    );

    let observation = observations.try_recv().expect("status observation");
    assert_eq!(observation.status, LifecycleStatus::NotApplicable);
    assert_eq!(observation.capability, Some(LifecycleCapability::Ws));
    assert_eq!(
        runtime.last_span_for_test().map(|span| span.name),
        Some("helix.lifecycle.status".to_string())
    );
}
