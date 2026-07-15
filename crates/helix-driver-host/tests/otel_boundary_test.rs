use std::sync::{Arc, Mutex};

use bytes::Bytes;
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, HttpResponse, SqlValue,
    StorageOp, TimerId, TransportId, UpsertSpec,
};
use helix_core::ports::{EventSink, HttpRequester, Storage};
use helix_core::tick::AppCommand;
use helix_core::{PortError, Tick};
use helix_driver_host::storage::{with_storage_trace_context, StorageTraceContext};
use helix_driver_host::trace::{
    TraceHooksImpl, TraceScope, TRACEPARENT_BYTES, TRACE_BAGGAGE_MAX_BYTES,
};
use helix_driver_host::{
    AuthTokenRegistry, CommandTraceQueue, CrossCuttingHttp, HostOtelConfig, HostOtelRuntime,
    HostStorage, TraceCarrier, TraceDirection, TraceHooks,
};

#[test]
fn host_otel_runtime_extracts_traceparent_and_starts_child_span() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });
    let carrier = TraceCarrier::from_json_str(
        r#"{"traceparent":"00-00000000000000000000000000000001-0000000000000002-01","baggage":"client=test"}"#,
    )
    .expect("carrier");

    let scope = runtime.span(
        "helix.command.accept",
        TraceDirection::Inbound,
        Some(&carrier),
    );
    assert_eq!(
        scope.trace_id_for_test().as_deref(),
        Some("00000000000000000000000000000001")
    );
    assert_eq!(scope.name_for_test(), "helix.command.accept");
}

#[test]
fn host_otel_runtime_uses_non_colliding_span_ids() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });
    let carrier = TraceCarrier::from_json_str(
        r#"{"traceparent":"00-00000000000000000000000000000001-0000000000000001-01"}"#,
    )
    .expect("carrier");

    let _scope = runtime.span(
        "helix.command.accept",
        TraceDirection::Inbound,
        Some(&carrier),
    );
    let span = runtime.last_span_for_test().expect("span");

    assert_eq!(span.span_id.len(), 16);
    assert_ne!(span.span_id, "0000000000000001");
    assert_ne!(span.parent_span_id.as_deref(), Some(span.span_id.as_str()));
}

#[test]
fn trace_hooks_consume_command_trace_sidecar_at_host_boundary() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });
    let queue = CommandTraceQueue::default();
    queue.push_slot(Some(
        TraceCarrier::from_json_str(
            r#"{"traceparent":"00-00000000000000000000000000000002-0000000000000003-01"}"#,
        )
        .expect("carrier"),
    ));
    let hooks = TraceHooks::noop()
        .with_otel(runtime.clone())
        .with_command_traces(queue);

    let tick = Tick::Command(AppCommand::new("send_message", Bytes::from_static(b"{}")));
    let _scope = hooks.on_tick_start(&tick);

    let span = runtime.last_span_for_test().expect("span");
    assert_eq!(span.name, "helix.command.accept");
    assert_eq!(span.direction, TraceDirection::Inbound);
    assert_eq!(
        span.trace_id.as_deref(),
        Some("00000000000000000000000000000002")
    );
}

#[test]
fn nested_tick_scope_restores_outer_command_trace_for_http_dispatch() {
    let queue = CommandTraceQueue::default();
    let traceparent = "00-00000000000000000000000000000011-0000000000000011-01";
    queue.push_slot(Some(
        TraceCarrier::from_json_str(&format!(r#"{{"traceparent":"{traceparent}"}}"#))
            .expect("carrier"),
    ));
    let hooks = TraceHooks::noop().with_command_traces(queue);

    let command_tick = Tick::Command(AppCommand::new("im_send", Bytes::from_static(b"{}")));
    let background_tick = Tick::Timer(TimerId::from_raw(7));

    let command_scope = hooks.on_tick_start(&command_tick);
    let background_scope = hooks.on_tick_start(&background_tick);
    drop(background_scope);

    let mut req = HttpRequest {
        method: "GET".to_string(),
        url: "https://example.test/sync".to_string(),
        headers: Vec::new(),
        body: None,
    };
    hooks.on_http_dispatch(None, &mut req);
    drop(command_scope);

    assert_eq!(
        req.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("traceparent"))
            .map(|(_, value)| value.as_str()),
        Some(traceparent)
    );
}

#[derive(Default)]
struct RecordingTraceHooks {
    seen: Arc<Mutex<Vec<Option<String>>>>,
}

impl TraceHooksImpl for RecordingTraceHooks {
    fn on_tick_start(&self, _: &Tick, carrier: Option<TraceCarrier>) -> TraceScope {
        self.seen
            .lock()
            .expect("seen mutex")
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
fn command_trace_scope_does_not_leak_to_plain_tick() {
    let queue = CommandTraceQueue::default();
    queue.push_slot(Some(
        TraceCarrier::from_json_str(
            r#"{"traceparent":"00-00000000000000000000000000000012-0000000000000012-01"}"#,
        )
        .expect("carrier"),
    ));
    queue.push_slot(None);

    let recording = RecordingTraceHooks::default();
    let seen = Arc::clone(&recording.seen);
    let hooks = TraceHooks::new(recording).with_command_traces(queue);

    let traced_tick = Tick::Command(AppCommand::new("im_send", Bytes::from_static(b"{}")));
    let plain_tick = Tick::Command(AppCommand::new(
        "im_sync_channels",
        Bytes::from_static(b"{}"),
    ));

    let traced_scope = hooks.on_tick_start(&traced_tick);
    drop(traced_scope);
    let plain_scope = hooks.on_tick_start(&plain_tick);
    drop(plain_scope);

    let rows = seen.lock().expect("seen mutex");
    assert_eq!(
        rows.as_slice(),
        &[
            Some("00-00000000000000000000000000000012-0000000000000012-01".to_string()),
            None,
        ]
    );
}

#[derive(Clone, Default)]
struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _: DomainEventBytes) {}
}

#[derive(Clone)]
struct MockHttp;

#[async_trait::async_trait]
impl HttpRequester for MockHttp {
    async fn request(&self, _: HttpRequest) -> Result<HttpResponse, PortError> {
        Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::new(),
        })
    }
}

#[test]
fn crosscutting_http_default_constructor_is_true_noop() {
    let http = CrossCuttingHttp::new(
        MockHttp,
        Arc::new(NoopSink),
        AuthTokenRegistry::new(),
        |_, _| None,
    );
    assert!(!http.is_otel_enabled());
}

#[tokio::test]
async fn crosscutting_http_starts_response_span_with_status() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });
    let http = CrossCuttingHttp::new(
        MockHttp,
        Arc::new(NoopSink),
        AuthTokenRegistry::new(),
        helix_im::http_outcome_event,
    )
    .with_otel(runtime.clone());

    let response = http
        .request(HttpRequest {
            method: "POST".to_string(),
            url: "https://example.test/im".to_string(),
            headers: vec![(
                "traceparent".to_string(),
                "00-00000000000000000000000000000004-0000000000000005-01".to_string(),
            )],
            body: Some(Bytes::from_static(br#"{"ok":true}"#)),
        })
        .await
        .expect("request ok");

    assert_eq!(response.status, 200);

    let span = runtime.last_span_for_test().expect("span");
    assert_eq!(span.name, "helix.http.response");
    assert_eq!(span.direction, TraceDirection::Inbound);
    assert_eq!(
        span.trace_id.as_deref(),
        Some("00000000000000000000000000000004")
    );
    assert!(span
        .attributes
        .iter()
        .any(|(k, v)| k == "http.response.status_code" && v == "200"));
}

#[tokio::test]
async fn sqlite_storage_span_carries_trace_context_statement_and_params() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });
    let carrier = TraceCarrier::from_json_str(
        r#"{"traceparent":"00-00000000000000000000000000000008-0000000000000009-01","baggage":"client=test"}"#,
    )
    .expect("carrier");
    let storage =
        HostStorage::open_sqlite_url("sqlite:file:helix_sql_trace?mode=memory&cache=shared")
            .await
            .expect("open sqlite");
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS messages \
             (id INTEGER PRIMARY KEY, body TEXT NOT NULL, channel_id TEXT NOT NULL);",
        )
        .await
        .expect("create table");

    with_storage_trace_context(
        StorageTraceContext::new(runtime.clone(), Some(carrier)),
        async {
            storage
                .batch_upsert(UpsertSpec {
                    table: "messages",
                    rows: vec![vec![
                        ("id".to_string(), SqlValue::Integer(7)),
                        (
                            "body".to_string(),
                            SqlValue::Text("hello trace".to_string()),
                        ),
                        ("channel_id".to_string(), SqlValue::Text("c-1".to_string())),
                    ]],
                    conflict_key: Some("id"),
                    exclude_from_update: Vec::new(),
                })
                .await
        },
    )
    .await
    .expect("upsert under trace context");

    let span = runtime.last_span_for_test().expect("sqlite span");
    assert_eq!(span.name, "helix.storage.sqlite.query");
    assert_eq!(span.direction, TraceDirection::Internal);
    assert_eq!(
        span.trace_id.as_deref(),
        Some("00000000000000000000000000000008")
    );
    assert!(span
        .attributes
        .iter()
        .any(|(k, v)| k == "db.statement" && v.contains("INSERT INTO messages")));
    assert!(span
        .attributes
        .iter()
        .any(|(k, v)| k == "db.parameters" && v.contains("hello trace")));
}

#[test]
fn trace_headers_drop_malformed_and_oversized_fields() {
    let malformed = vec![
        (
            "traceparent".to_string(),
            "00-00000000000000000000000000000000-0000000000000001-01".to_string(),
        ),
        ("baggage".to_string(), "client=test".to_string()),
    ];
    let carrier = TraceCarrier::from_headers(&malformed).expect("valid baggage remains");
    assert_eq!(carrier.traceparent, None);
    assert_eq!(carrier.baggage.as_deref(), Some("client=test"));

    let oversized = vec![
        ("traceparent".to_string(), "0".repeat(TRACEPARENT_BYTES + 1)),
        (
            "baggage".to_string(),
            "a".repeat(TRACE_BAGGAGE_MAX_BYTES + 1),
        ),
    ];
    assert_eq!(TraceCarrier::from_headers(&oversized), None);
}
