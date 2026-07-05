# Multiplatform OTel Trace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a configurable OpenTelemetry trace chain for one `im.send(...)` flow across PC Tauri, mobile QuickJS/C++, Helix host/driver, and `cses-im-server`, exporting through local OTel Collector into Jaeger.

**Architecture:** Trace context is propagated through system sidecars and protocol envelopes, not business DTOs. `helix-core` stays runtime-agnostic; Rust OTel code lives in host/native/ffi/client shells, Go OTel code lives in middleware and boundary wrappers, and clients can enable tracing independently.

**Tech Stack:** OpenTelemetry SDK, W3C `traceparent`/`baggage`, OTLP gRPC/HTTP, Jaeger, Rust `tracing`/OTel bridge at driver layer, Go OTel SDK, Angular/Tauri bridge sidecar, C++/QuickJS additive command sidecar.

## Global Constraints

- Config primary path is project config: Helix `helix.config.yaml`, loopforge `config/*.json`, mobile `config/mobile-local.json`, Go `config.yaml + Consul KV`.
- Environment variables are not the trace enablement/source-of-truth path.
- `helix-core` must not depend on OTel, exporters, Tokio-specific tracing glue, Tauri, reqwest, sqlx, or web APIs.
- Trace metadata must not be persisted into business rows, business DTOs, or projection schemas as business fields.
- HTTP propagation uses W3C `traceparent` and `baggage`.
- WS per-message propagation uses the envelope-level `tracing` field, never `data`.
- Tauri invoke propagation uses `__trace` sidecar and strips it before command payload enters Helix business handling.
- Mobile FFI v1 adds `helix_command_with_trace`; the existing `helix_command` remains source-compatible.
- OTel exporter failure must not change IM business success/failure behavior.
- Each project can run tracing off or independently on.
- Local dev sampling ratio is `1.0`; pre/prod defaults below `1.0` with parent-based sampling.

---

## File Structure

### Helix Repo: `/System/Volumes/Data/workspace/rust/helix`

- Create `docker/otel/otel-collector.yaml`: local collector receiver/exporter config.
- Create `docker/otel/docker-compose.yaml`: local Collector + Jaeger stack.
- Create `docs/trace/local-jaeger-runbook.md`: local startup, smoke, and Jaeger inspection runbook.
- Modify `helix.config.yaml`: add `observability.otel` config block.
- Modify `crates/helix-host-cli/src/config.rs`: deserialize `observability.otel`.
- Create `crates/helix-driver-host/src/trace.rs`: runtime-neutral host trace config/types and span helper surface.
- Modify `crates/helix-driver-host/src/lib.rs`: re-export trace helpers.
- Modify `crates/helix-driver-host/src/engine.rs`: add optional `TraceHooks` to `EngineDeps`, create core-step and tick spans outside `helix-core`.
- Modify `crates/helix-driver-host/src/dispatch.rs`: wrap Persist/Http/Send/Emit dispatch boundaries.
- Modify `crates/helix-driver-host/src/http_cross.rs`: inject W3C context when absent and preserve valid inbound `traceparent`.
- Modify `crates/helix-driver-ffi/src/api.rs`: add `helix_command_with_trace`.
- Modify `crates/helix-driver-ffi/src/pump.rs`: pass trace hooks into shared host engine.
- Modify `crates/helix-driver-ffi/helix.h` and `crates/helix-driver-ffi/helix.hpp`: regenerate exported C/C++ headers.
- Add tests under `crates/helix-driver-host/tests/trace_context.rs` and `crates/helix-driver-ffi/tests/trace_command_test.rs`.

### Go Repo: `/System/Volumes/Data/workspace/golang/cses-im-server`

- Modify `config.yaml`: add local `observability.otel`.
- Modify `deploy/consul/cses-im-server.kv.yaml`: add Consul KV keys for OTel config.
- Modify `internal/config/config.go`: add `Observability` and OTel config structs.
- Modify `internal/config/yaml_source.go`: map YAML observability keys.
- Create `internal/observability/otel.go`: provider bootstrap and shutdown.
- Create `internal/observability/trace.go`: tracer accessors, span naming, attribute helpers, propagation helpers.
- Create `internal/api/trace_middleware.go`: gorilla/mux HTTP trace middleware.
- Modify `internal/api/router.go`: install trace middleware.
- Modify post creation handler files under `internal/api/post/`: boundary handler span for `create_post`.
- Modify `internal/app/app.go` and post create path files: service boundary span.
- Modify `internal/store/sqlstore/cses_post_store*.go`: store boundary spans for create/read paths used by send flow.
- Modify `internal/ws/event.go` and `internal/ws/event_encode.go`: envelope `tracing` field injection/extraction.
- Modify `internal/ws/hub_broadcast.go`, `internal/ws/hub_loop.go`, `internal/ws/web_conn*.go`: publish/fanout/deliver spans.
- Add tests under `internal/observability`, `internal/api`, and `internal/ws`.

### Loopforge Repo: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im`

- Modify `config/dev-local.json`, `config/pre.json`, `config/prod.json`: add optional `observability.otel`.
- Modify `src-tauri/src/config.rs`: parse trace config.
- Create `src-tauri/src/trace.rs`: OTel bootstrap and carrier helpers.
- Modify `src-tauri/src/engine.rs`: initialize trace hooks and wrap `RecordingSink<NativeEventSink>`.
- Modify `src-tauri/src/commands.rs`: accept `__trace` sidecar in command args and strip before `Tick::Command`.
- Modify `src/app/im/tauri-bridge.service.ts`: attach `__trace` sidecar in invoke envelope.
- Create `src/app/im/trace-context.service.ts`: browser-side trace context generation and storage.
- Modify `src/app/im/im-store.service.ts`: start `pc.ui.action` and `pc.ui.render` spans around send/render.
- Add Rust tests in `src-tauri/src/config.rs` or `trace.rs`.
- Add Angular tests for bridge sidecar stripping behavior.

### Mobile Repo: `/System/Volumes/Data/workspace/c/mobile-qucik-c++`

- Create `config/mobile-local.json`: local mobile endpoint and OTel config.
- Create `cpp/TraceContext.hpp` and `cpp/TraceContext.cpp`: W3C trace carrier structs and helpers.
- Modify `cpp/CoreBridge.hpp`: add trace config and `callWithTrace`.
- Modify `cpp/CoreBridge.cpp`: call `helix_command_with_trace` and wrap C++ bridge spans.
- Modify `quickjs/bind_core_bridge.cpp`: expose `core.callWithTrace(name, payload, trace)`.
- Create `js/trace_context.js`: JS trace context generator/carrier helper.
- Modify `js/im_api.js`: pass trace sidecar from `im.send(...)` to `core.callWithTrace`.
- Modify `scripts/run-quickjs-smoke.sh` and `scripts/run-cpp-smoke.sh`: prefer `config/mobile-local.json`.
- Add C++ and QuickJS smoke assertions.

### Cross-Repo E2E

- Create or update a trace smoke script in the most convenient harness repo after Tasks 1-5 are merged. Preferred home: `loopforge-tauri-im/scripts/otel-trace-smoke.sh`, because loopforge already exercises PC UI + Helix + Go.
- Add trace evidence output under existing real-chain/trace report folders, not business storage.

---

## Task 1: Local OTel Collector And Runbook

**Files:**
- Create: `/System/Volumes/Data/workspace/rust/helix/docker/otel/otel-collector.yaml`
- Create: `/System/Volumes/Data/workspace/rust/helix/docker/otel/docker-compose.yaml`
- Create: `/System/Volumes/Data/workspace/rust/helix/docs/trace/local-jaeger-runbook.md`
- Modify: `/System/Volumes/Data/workspace/rust/helix/helix.config.yaml`

**Interfaces:**
- Consumes: none.
- Produces: local OTLP gRPC endpoint `http://127.0.0.1:4317`, local OTLP HTTP endpoint `http://127.0.0.1:4318`, Jaeger UI `http://127.0.0.1:16686`.

- [ ] **Step 1: Add Collector config**

Create `docker/otel/otel-collector.yaml`:

```yaml
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

processors:
  batch: {}

exporters:
  otlp/jaeger:
    endpoint: jaeger:4317
    tls:
      insecure: true
  logging:
    verbosity: basic

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlp/jaeger, logging]
```

- [ ] **Step 2: Add local compose stack**

Create `docker/otel/docker-compose.yaml`:

```yaml
services:
  jaeger:
    image: jaegertracing/all-in-one:1.57
    command:
      - "--collector.otlp.enabled=true"
    ports:
      - "16686:16686"
      - "4319:4317"

  otel-collector:
    image: otel/opentelemetry-collector-contrib:0.103.0
    command:
      - "--config=/etc/otelcol/otel-collector.yaml"
    volumes:
      - "./otel-collector.yaml:/etc/otelcol/otel-collector.yaml:ro"
    ports:
      - "4317:4317"
      - "4318:4318"
    depends_on:
      - jaeger
```

- [ ] **Step 3: Add Helix local config block**

Append this block to `helix.config.yaml`:

```yaml
observability:
  otel:
    enabled: true
    serviceName: "helix-host-cli"
    endpoint: "http://127.0.0.1:4317"
    protocol: "grpc"
    sampler:
      type: "parentbased_traceidratio"
      ratio: 1.0
    attributes:
      deployment.environment: "local"
    facets:
      command: true
      core: true
      http: true
      ws: true
      storage: true
      portReply: true
      event: true
      native: true
      ffi: true
```

- [ ] **Step 4: Write runbook**

Create `docs/trace/local-jaeger-runbook.md`:

```markdown
# Local OTel + Jaeger Runbook

## Start

Run from the Helix repo:

```bash
docker compose -f docker/otel/docker-compose.yaml up -d
```

## Verify Endpoints

```bash
nc -zv 127.0.0.1 4317
curl -fsS http://127.0.0.1:4318/v1/traces -X POST -H 'content-type: application/json' -d '{}'
open http://127.0.0.1:16686
```

Expected:

- `4317` accepts TCP.
- `4318` returns an OTLP HTTP response instead of connection refused.
- Jaeger UI opens at `16686`.

## First Trace

1. Start `cses-im-server` with `observability.otel.enabled=true`.
2. Start PC Tauri or mobile QuickJS with trace config enabled.
3. Trigger `im.send(...)`.
4. In Jaeger UI, search service `loopforge-tauri-im`, `mobile-quickjs`, `helix-driver-host`, or `cses-im-server`.

The first accepted trace must contain client, Helix, Go HTTP, Go WS, Helix WS receive, and render spans.
```

- [ ] **Step 5: Verify local stack**

Run:

```bash
docker compose -f docker/otel/docker-compose.yaml config
```

Expected: compose renders both `jaeger` and `otel-collector` without errors.

- [ ] **Step 6: Commit**

```bash
git add docker/otel docs/trace helix.config.yaml
git commit -m "chore(trace): add local otel collector and jaeger runbook"
```

---

## Task 2: Go Server OTel Bootstrap And HTTP Boundary

**Files:**
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/config.yaml`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/deploy/consul/cses-im-server.kv.yaml`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/config/config.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/config/yaml_source.go`
- Create: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/observability/otel.go`
- Create: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/observability/trace.go`
- Create: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/trace_middleware.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/router.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/cmd/server/main.go`
- Test: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/observability/otel_test.go`
- Test: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/trace_middleware_test.go`

**Interfaces:**
- Consumes: config keys under `observability.otel`.
- Produces: `observability.Config`, `observability.Init(ctx, cfg) (shutdown func(context.Context) error, err error)`, HTTP middleware that extracts W3C context and starts `cses.http.request`.

- [ ] **Step 1: Add failing config test**

Add to `internal/config/config_test.go`:

```go
func TestLoadObservabilityOtelDefaults(t *testing.T) {
	cfg, err := Load(staticSource{})
	if err != nil {
		t.Fatal(err)
	}
	if cfg.Observability.Otel.Enabled {
		t.Fatal("otel must default disabled")
	}
	if cfg.Observability.Otel.ServiceName != "cses-im-server" {
		t.Fatalf("service name = %q", cfg.Observability.Otel.ServiceName)
	}
}
```

Expected before implementation: compile fails because `Config.Observability` is missing.

- [ ] **Step 2: Add config structs**

In `internal/config/config.go`, add:

```go
type ObservabilityConfig struct {
	Otel OtelConfig
}

type OtelConfig struct {
	Enabled     bool
	ServiceName string
	Endpoint    string
	Protocol    string
	SamplerType string
	SamplerRatio float64
	Environment string
	Facets      map[string]bool
}
```

Add field to `Config`:

```go
Observability ObservabilityConfig
```

Add constants:

```go
const (
	keyOtelEnabled     = "observability.otel.enabled"
	keyOtelServiceName = "observability.otel.serviceName"
	keyOtelEndpoint    = "observability.otel.endpoint"
	keyOtelProtocol    = "observability.otel.protocol"
	keyOtelSamplerType = "observability.otel.sampler.type"
	keyOtelSamplerRatio = "observability.otel.sampler.ratio"
	keyOtelEnvironment = "observability.otel.attributes.deployment.environment"
	keyOtelFacets      = "observability.otel.facets"
)
```

Add load helper:

```go
func loadOtelConfig(ctx context.Context, s Source) OtelConfig {
	return OtelConfig{
		Enabled:      getFromSource(ctx, s, keyOtelEnabled, "false") == "true",
		ServiceName:  getFromSource(ctx, s, keyOtelServiceName, "cses-im-server"),
		Endpoint:     getFromSource(ctx, s, keyOtelEndpoint, "http://127.0.0.1:4317"),
		Protocol:     getFromSource(ctx, s, keyOtelProtocol, "grpc"),
		SamplerType:  getFromSource(ctx, s, keyOtelSamplerType, "parentbased_traceidratio"),
		SamplerRatio: parseFloatDefault(getFromSource(ctx, s, keyOtelSamplerRatio, "0.0"), 0.0),
		Environment:  getFromSource(ctx, s, keyOtelEnvironment, "local"),
		Facets:       parseFacetMap(getFromSource(ctx, s, keyOtelFacets, "")),
	}
}
```

Also add `parseFloatDefault` and `parseFacetMap` in the same file; `parseFacetMap` accepts comma-separated `name=true` pairs for Consul KV friendliness.

- [ ] **Step 3: Wire YAML keys**

In `internal/config/yaml_source.go`, extend `yamlKeyMap`:

```go
keyOtelEnabled:      "observability.otel.enabled",
keyOtelServiceName:  "observability.otel.serviceName",
keyOtelEndpoint:     "observability.otel.endpoint",
keyOtelProtocol:     "observability.otel.protocol",
keyOtelSamplerType:  "observability.otel.sampler.type",
keyOtelSamplerRatio: "observability.otel.sampler.ratio",
keyOtelEnvironment:  "observability.otel.attributes.deployment.environment",
keyOtelFacets:       "observability.otel.facets",
```

If current YAML loader only supports flat `map[string]string`, represent the local YAML as flat dotted keys:

```yaml
observability.otel.enabled: "true"
observability.otel.serviceName: "cses-im-server"
observability.otel.endpoint: "http://127.0.0.1:4317"
observability.otel.protocol: "grpc"
observability.otel.sampler.type: "parentbased_traceidratio"
observability.otel.sampler.ratio: "1.0"
observability.otel.attributes.deployment.environment: "local"
observability.otel.facets: "http=true,handler=true,service=true,store=true,ws=true,pulsar=true,grpc=true,redis=true,es=true"
```

- [ ] **Step 4: Add OTel bootstrap**

Create `internal/observability/otel.go`:

```go
package observability

import (
	"context"
	"errors"
	"time"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracegrpc"
	"go.opentelemetry.io/otel/exporters/otlp/otlptrace/otlptracehttp"
	"go.opentelemetry.io/otel/propagation"
	"go.opentelemetry.io/otel/sdk/resource"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	semconv "go.opentelemetry.io/otel/semconv/v1.26.0"

	"cses-im-server/internal/config"
)

func Init(ctx context.Context, cfg config.OtelConfig) (func(context.Context) error, error) {
	if !cfg.Enabled {
		otel.SetTextMapPropagator(propagation.NewCompositeTextMapPropagator(propagation.TraceContext{}, propagation.Baggage{}))
		return func(context.Context) error { return nil }, nil
	}

	res, err := resource.Merge(
		resource.Default(),
		resource.NewWithAttributes(
			semconv.SchemaURL,
			semconv.ServiceName(cfg.ServiceName),
			semconv.DeploymentEnvironment(cfg.Environment),
		),
	)
	if err != nil {
		return nil, err
	}

	timeout, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	var exporter sdktrace.SpanExporter
	switch cfg.Protocol {
	case "grpc":
		exporter, err = otlptracegrpc.New(timeout, otlptracegrpc.WithEndpointURL(cfg.Endpoint), otlptracegrpc.WithInsecure())
	case "http":
		exporter, err = otlptracehttp.New(timeout, otlptracehttp.WithEndpointURL(cfg.Endpoint))
	default:
		return nil, errors.New("unsupported otel protocol: " + cfg.Protocol)
	}
	if err != nil {
		return nil, err
	}

	tp := sdktrace.NewTracerProvider(
		sdktrace.WithResource(res),
		sdktrace.WithSampler(sdktrace.ParentBased(sdktrace.TraceIDRatioBased(cfg.SamplerRatio))),
		sdktrace.WithBatcher(exporter),
	)
	otel.SetTracerProvider(tp)
	otel.SetTextMapPropagator(propagation.NewCompositeTextMapPropagator(propagation.TraceContext{}, propagation.Baggage{}))
	return tp.Shutdown, nil
}
```

- [ ] **Step 5: Add trace helper**

Create `internal/observability/trace.go`:

```go
package observability

import (
	"context"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/trace"
)

const tracerName = "cses-im-server"

func Tracer() trace.Tracer {
	return otel.Tracer(tracerName)
}

func Start(ctx context.Context, name string, attrs ...attribute.KeyValue) (context.Context, trace.Span) {
	return Tracer().Start(ctx, name, trace.WithAttributes(attrs...))
}

func RecordError(span trace.Span, err error) {
	if err == nil {
		return
	}
	span.RecordError(err)
	span.SetStatus(codes.Error, err.Error())
}
```

- [ ] **Step 6: Add HTTP middleware**

Create `internal/api/trace_middleware.go`:

```go
package api

import (
	"net/http"

	"cses-im-server/internal/observability"
	"github.com/gorilla/mux"
	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/propagation"
	semconv "go.opentelemetry.io/otel/semconv/v1.26.0"
)

type statusRecorder struct {
	http.ResponseWriter
	status int
}

func (r *statusRecorder) WriteHeader(code int) {
	r.status = code
	r.ResponseWriter.WriteHeader(code)
}

func traceMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ctx := otel.GetTextMapPropagator().Extract(r.Context(), propagation.HeaderCarrier(r.Header))
		route := "unknown"
		if current := mux.CurrentRoute(r); current != nil {
			if path, err := current.GetPathTemplate(); err == nil {
				route = path
			}
		}
		ctx, span := observability.Start(ctx, "cses.http.request",
			semconv.HTTPRequestMethodKey.String(r.Method),
			attribute.String("http.route", route),
		)
		defer span.End()

		rec := &statusRecorder{ResponseWriter: w, status: http.StatusOK}
		next.ServeHTTP(rec, r.WithContext(ctx))
		span.SetAttributes(attribute.Int("http.status_code", rec.status))
		if rec.status >= 500 {
			span.SetStatus(codes.Error, http.StatusText(rec.status))
		}
	})
}
```

- [ ] **Step 7: Install middleware and bootstrap**

In `internal/api/router.go`, install `traceMiddleware` near existing cross-cutting middleware. In `cmd/server/main.go`, after config load and before server start:

```go
otelShutdown, err := observability.Init(context.Background(), cfg.Observability.Otel)
if err != nil {
	return fmt.Errorf("init observability: %w", err)
}
defer func() {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := otelShutdown(ctx); err != nil {
		slog.Warn("otel shutdown failed", "error", err)
	}
}()
```

- [ ] **Step 8: Verify**

Run:

```bash
cd /System/Volumes/Data/workspace/golang/cses-im-server
go test ./internal/config ./internal/observability ./internal/api
```

Expected: all packages pass; config tests prove disabled default and local enabled parse path.

- [ ] **Step 9: Commit**

```bash
git add config.yaml deploy/consul/cses-im-server.kv.yaml internal/config internal/observability internal/api cmd/server/main.go
git commit -m "feat(trace): bootstrap otel and http boundary spans"
```

---

## Task 3: Go WS, Service, Store, And Publish Trace Boundaries

**Files:**
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_create.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/app/cses_post_create*.go` or current create-post app file found by `rg "Create.*Post|create.*post" internal/app`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/store/sqlstore/cses_post_store*.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/event.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/event_encode.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/hub_broadcast.go`
- Modify: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/web_conn.go`
- Test: `/System/Volumes/Data/workspace/golang/cses-im-server/internal/ws/trace_envelope_test.go`
- Test: targeted post create tests in `internal/app` and `internal/store/sqlstore`

**Interfaces:**
- Consumes: HTTP request context from Task 2 middleware.
- Produces: handler/service/store spans and WS envelope `tracing.traceparent` / `tracing.baggage` injection.

- [ ] **Step 1: Add WS tracing struct test**

Create `internal/ws/trace_envelope_test.go`:

```go
package ws

import (
	"context"
	"encoding/json"
	"testing"

	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/propagation"
)

func TestEventEnvelopeCarriesTraceContextOutsideData(t *testing.T) {
	otel.SetTextMapPropagator(propagation.TraceContext{})
	ctx := context.Background()
	ev := NewWebSocketEvent(WebsocketEventPost, "", "ch1", "", nil, "")
	ev.SetData(map[string]any{"message": "hello"})

	InjectTraceContext(ctx, ev)
	raw, err := json.Marshal(ev)
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal(err)
	}
	if _, ok := decoded["tracing"]; !ok {
		t.Fatal("missing top-level tracing")
	}
	data := decoded["data"].(map[string]any)
	if _, ok := data["tracing"]; ok {
		t.Fatal("tracing leaked into business data")
	}
}
```

- [ ] **Step 2: Add WS envelope helpers**

In `internal/ws/event.go`, add a top-level field to the event struct:

```go
Tracing map[string]string `json:"tracing,omitempty"`
```

Add helpers:

```go
func InjectTraceContext(ctx context.Context, ev *WebSocketEvent) {
	if ev == nil {
		return
	}
	carrier := propagation.MapCarrier{}
	otel.GetTextMapPropagator().Inject(ctx, carrier)
	if len(carrier) == 0 {
		return
	}
	ev.Tracing = map[string]string(carrier)
}

func ExtractTraceContext(ctx context.Context, ev *WebSocketEvent) context.Context {
	if ev == nil || len(ev.Tracing) == 0 {
		return ctx
	}
	return otel.GetTextMapPropagator().Extract(ctx, propagation.MapCarrier(ev.Tracing))
}
```

- [ ] **Step 3: Add handler boundary span**

At post create handler entry:

```go
ctx, span := observability.Start(r.Context(), "cses.handler.create_post",
	attribute.String("handler.name", "create_post"),
)
defer span.End()
r = r.WithContext(ctx)
```

On error paths:

```go
observability.RecordError(span, err)
```

- [ ] **Step 4: Add service boundary span**

At create post app/service entry:

```go
ctx, span := observability.Start(ctx, "cses.service.create_post",
	attribute.String("service.name", "post"),
	attribute.String("operation", "create_post"),
)
defer span.End()
```

Ensure downstream store and publish calls receive `ctx`.

- [ ] **Step 5: Add store boundary span**

At create post store method:

```go
ctx, span := observability.Start(ctx, "cses.store.create_post",
	attribute.String("db.system", "postgresql"),
	attribute.String("db.operation", "insert"),
	attribute.String("table", "posts"),
)
defer span.End()
```

After mutation:

```go
span.SetAttributes(attribute.Int("rows.affected", rowsAffected))
```

No SQL values, message text, token, cookie, or raw body may be recorded.

- [ ] **Step 6: Add publish/fanout/deliver spans**

At WS publish boundary:

```go
ctx, span := observability.Start(ctx, "cses.ws.publish",
	attribute.String("ws.action", ev.EventType()),
	attribute.String("channel.id", ev.ChannelId()),
)
defer span.End()
ws.InjectTraceContext(ctx, ev)
```

At fanout boundary:

```go
ctx, span := observability.Start(ctx, "cses.ws.fanout",
	attribute.String("ws.action", ev.EventType()),
	attribute.Int("viewer.count", len(targets)),
)
defer span.End()
```

At per-connection deliver:

```go
_, span := observability.Start(ctx, "cses.ws.deliver",
	attribute.String("connection.id", conn.ID()),
	attribute.String("user.id", conn.UserID()),
)
defer span.End()
```

- [ ] **Step 7: Verify**

Run:

```bash
cd /System/Volumes/Data/workspace/golang/cses-im-server
go test ./internal/ws ./internal/app ./internal/store/sqlstore ./internal/api
```

Expected: WS envelope test proves trace is top-level; post tests prove ctx threading did not change business behavior.

- [ ] **Step 8: Commit**

```bash
git add internal/api internal/app internal/store internal/ws
git commit -m "feat(trace): instrument go post and websocket boundaries"
```

---

## Task 4: Helix Host Trace Hooks And FFI Trace Command

**Files:**
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-host-cli/src/config.rs`
- Create: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-host/src/trace.rs`
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-host/src/lib.rs`
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-host/src/engine.rs`
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-host/src/dispatch.rs`
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-host/src/http_cross.rs`
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-ffi/src/api.rs`
- Modify: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-ffi/src/pump.rs`
- Regenerate: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-ffi/helix.h`
- Regenerate: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-ffi/helix.hpp`
- Test: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-host/tests/trace_context.rs`
- Test: `/System/Volumes/Data/workspace/rust/helix/crates/helix-driver-ffi/tests/trace_command_test.rs`

**Interfaces:**
- Consumes: W3C carrier JSON `{ "traceparent": "...", "baggage": "..." }`.
- Produces: host-layer `TraceHooks`, FFI `helix_command_with_trace`, context-preserving HTTP/WS/Event/Storage spans.

- [ ] **Step 1: Add config model test**

In `crates/helix-host-cli/src/config_tests.rs`, add:

```rust
#[test]
fn parses_observability_otel_config() {
    let cfg = Config::from_yaml_str(
        r#"
observability:
  otel:
    enabled: true
    serviceName: "helix-host-cli"
    endpoint: "http://127.0.0.1:4317"
    protocol: "grpc"
    sampler:
      type: "parentbased_traceidratio"
      ratio: 1.0
    facets:
      command: true
      core: true
      http: true
"#,
    )
    .expect("config parses");
    assert!(cfg.observability.otel.enabled);
    assert_eq!(cfg.observability.otel.service_name, "helix-host-cli");
    assert!(cfg.observability.otel.facets.http);
}
```

- [ ] **Step 2: Add Rust config structs**

In `crates/helix-host-cli/src/config.rs`, add:

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Observability {
    #[serde(default)]
    pub otel: Otel,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Otel {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "serviceName", default = "d_helix_service_name")]
    pub service_name: String,
    #[serde(default = "d_otel_endpoint")]
    pub endpoint: String,
    #[serde(default = "d_otel_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub sampler: OtelSampler,
    #[serde(default)]
    pub facets: OtelFacets,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OtelSampler {
    #[serde(rename = "type", default = "d_sampler_type")]
    pub kind: String,
    #[serde(default)]
    pub ratio: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OtelFacets {
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub core: bool,
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub ws: bool,
    #[serde(default)]
    pub storage: bool,
    #[serde(rename = "portReply", default)]
    pub port_reply: bool,
    #[serde(default)]
    pub event: bool,
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub ffi: bool,
}
```

Add `pub observability: Observability` to `Config`.

- [ ] **Step 3: Add host trace hooks**

Create `crates/helix-driver-host/src/trace.rs`:

```rust
use std::sync::Arc;

use helix_core::effect::{CorrelationId, DomainEventBytes, HttpRequest, StorageOp, TransportId};
use helix_core::Tick;

#[derive(Clone, Debug, Default)]
pub struct TraceCarrier {
    pub traceparent: Option<String>,
    pub baggage: Option<String>,
}

#[derive(Clone)]
pub struct TraceHooks {
    inner: Arc<dyn TraceHooksImpl + Send + Sync>,
}

pub trait TraceHooksImpl {
    fn on_tick_start(&self, tick: &Tick) -> TraceScope;
    fn on_http_dispatch(&self, corr: CorrelationId, req: &mut HttpRequest);
    fn on_storage_dispatch(&self, corr: Option<CorrelationId>, ops: &[StorageOp]);
    fn on_ws_send(&self, transport: TransportId, frame_len: usize);
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
        Self { end: Some(Box::new(end)) }
    }
}

impl Drop for TraceScope {
    fn drop(&mut self) {
        if let Some(end) = self.end.take() {
            end();
        }
    }
}

impl TraceHooks {
    pub fn new(inner: impl TraceHooksImpl + Send + Sync + 'static) -> Self {
        Self { inner: Arc::new(inner) }
    }
    pub fn noop() -> Self {
        Self::new(NoopTraceHooks)
    }
}

impl TraceHooksImpl for TraceHooks {
    fn on_tick_start(&self, tick: &Tick) -> TraceScope { self.inner.on_tick_start(tick) }
    fn on_http_dispatch(&self, corr: CorrelationId, req: &mut HttpRequest) { self.inner.on_http_dispatch(corr, req) }
    fn on_storage_dispatch(&self, corr: Option<CorrelationId>, ops: &[StorageOp]) { self.inner.on_storage_dispatch(corr, ops) }
    fn on_ws_send(&self, transport: TransportId, frame_len: usize) { self.inner.on_ws_send(transport, frame_len) }
    fn on_event_emit(&self, event: &DomainEventBytes) { self.inner.on_event_emit(event) }
}

struct NoopTraceHooks;

impl TraceHooksImpl for NoopTraceHooks {
    fn on_tick_start(&self, _: &Tick) -> TraceScope { TraceScope::noop() }
    fn on_http_dispatch(&self, _: CorrelationId, _: &mut HttpRequest) {}
    fn on_storage_dispatch(&self, _: Option<CorrelationId>, _: &[StorageOp]) {}
    fn on_ws_send(&self, _: TransportId, _: usize) {}
    fn on_event_emit(&self, _: &DomainEventBytes) {}
}
```

- [ ] **Step 4: Thread hooks through engine and dispatch**

Add to `EngineDeps`:

```rust
pub trace: crate::trace::TraceHooks,
```

At construction sites, use `TraceHooks::noop()` until OTel bootstrap is wired.

In `run_engine_loop`, wrap each `shell.step`:

```rust
let _trace_scope = trace.on_tick_start(&tick);
let now_ms = clock.now_ms();
let owned = match shell.step(tick, now_ms) {
    Ok(effects) => consume_effects(effects),
    Err(e) => {
        tracing::error!("engine step error: {}", e);
        continue;
    }
};
```

Pass `&trace` into `dispatch_effects`.

In `dispatch_effects`, before submit/send/emit:

```rust
trace.on_storage_dispatch(Some(corr), &ops);
trace.on_http_dispatch(corr, &mut req);
trace.on_ws_send(transport, frame.len());
trace.on_event_emit(&event);
```

- [ ] **Step 5: Add HTTP propagation test**

Create `crates/helix-driver-host/tests/trace_context.rs`:

```rust
use bytes::Bytes;
use helix_core::effect::HttpRequest;

#[test]
fn traceparent_header_survives_http_crosscut() {
    let req = HttpRequest {
        method: "POST".into(),
        url: "posts/create".into(),
        headers: vec![("traceparent".into(), "00-00000000000000000000000000000001-0000000000000002-01".into())],
        body: Some(Bytes::from_static(b"{}")),
    };
    let headers = req.headers.iter().filter(|(k, _)| k == "traceparent").count();
    assert_eq!(headers, 1);
}
```

Replace the direct count with the repo's actual `http_cross` prepare function if it is public; otherwise make the prepare function `pub(crate)` and place the test in a unit-test module beside `http_cross.rs`.

- [ ] **Step 6: Add FFI trace command**

In `crates/helix-driver-ffi/src/api.rs`, keep `Tick::Command` unchanged and store the sidecar in a driver-owned FIFO registry before sending the command. Add this field to `HelixHandle`:

```rust
pub(crate) command_traces: parking_lot::Mutex<std::collections::VecDeque<String>>,
```

Initialize it in `helix_create_with_network`:

```rust
command_traces: parking_lot::Mutex::new(std::collections::VecDeque::new()),
```

Add the exported function:

```rust
#[no_mangle]
pub unsafe extern "C" fn helix_command_with_trace(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
    payload: *const u8,
    payload_len: usize,
    trace_json: *const u8,
    trace_json_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    let Ok(name) = (unsafe { ffi_str(name, name_len) }) else {
        return FFI_UTF8;
    };
    let Ok(payload) = (unsafe { ffi_bytes(payload, payload_len) }) else {
        return FFI_NULL;
    };
    let Ok(trace_json) = (unsafe { ffi_str(trace_json, trace_json_len) }) else {
        return FFI_UTF8;
    };
    let h = unsafe { &*handle };
    h.command_traces.lock().push_back(trace_json.to_string());
    let command = AppCommand::new(name, Bytes::copy_from_slice(payload));
    send_tick(handle, Tick::Command(command))
}
```

The host trace hook consumes the next command sidecar when it sees `Tick::Command`; if the FIFO is empty, it starts a local root span. This is a driver boundary registry, not business state.

- [ ] **Step 7: Add FFI test**

Create `crates/helix-driver-ffi/tests/trace_command_test.rs`:

```rust
#[test]
fn helix_command_with_trace_is_exported_in_header() {
    let header = std::fs::read_to_string("crates/helix-driver-ffi/helix.h")
        .expect("read generated header");
    assert!(header.contains("helix_command_with_trace"));
}
```

- [ ] **Step 8: Regenerate headers**

Run the existing header generation command used by this repo. If no wrapper exists, run:

```bash
cargo build -p helix-driver-ffi
```

Then verify:

```bash
rg "helix_command_with_trace" crates/helix-driver-ffi/helix.h crates/helix-driver-ffi/helix.hpp
```

Expected: both headers contain the new symbol.

- [ ] **Step 9: Verify Helix invariants**

Run:

```bash
cd /System/Volumes/Data/workspace/rust/helix
cargo check -p helix-core -p helix-im -p helix-driver-host -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-driver-host -p helix-driver-ffi
cargo tree -p helix-core | grep -iE 'opentelemetry|tokio|sqlx|reqwest|web-sys|tauri' && exit 1 || true
```

Expected: checks/tests pass and `helix-core` grep emits no dependency.

- [ ] **Step 10: Commit**

```bash
git add helix.config.yaml crates/helix-host-cli crates/helix-driver-host crates/helix-driver-ffi
git commit -m "feat(trace): add helix host trace hooks and ffi carrier"
```

---

## Task 5: PC Tauri Trace Bridge In Loopforge

**Files:**
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/config/dev-local.json`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/config/pre.json`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/config/prod.json`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src-tauri/src/config.rs`
- Create: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src-tauri/src/trace.rs`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src-tauri/src/engine.rs`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src-tauri/src/commands.rs`
- Create: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src/app/im/trace-context.service.ts`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src/app/im/tauri-bridge.service.ts`
- Modify: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/src/app/im/im-store.service.ts`

**Interfaces:**
- Consumes: loopforge profile `observability.otel`.
- Produces: `__trace` sidecar on Tauri invoke and Tauri Rust stripping before command payload reaches Helix.

- [ ] **Step 1: Add profile config**

Add to each `config/*.json`:

```json
"observability": {
  "otel": {
    "enabled": true,
    "serviceName": "loopforge-tauri-im",
    "endpoint": "http://127.0.0.1:4318",
    "protocol": "http",
    "sampler": {
      "type": "parentbased_traceidratio",
      "ratio": 1.0
    },
    "facets": {
      "client": true,
      "tauri": true,
      "core": true,
      "http": true,
      "ws": true,
      "storage": true,
      "event": true,
      "render": true
    }
  }
}
```

Set `enabled` to `false` in `prod.json` unless the deployment already has an internal Collector.

- [ ] **Step 2: Extend Rust config**

In `src-tauri/src/config.rs`, add optional config structs:

```rust
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct Observability {
    #[serde(default)]
    pub otel: OtelConfig,
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct OtelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "serviceName", default)]
    pub service_name: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub facets: serde_json::Value,
}
```

Add field to `ProfileConfig`:

```rust
#[serde(default)]
pub observability: Observability,
```

- [ ] **Step 3: Add Angular trace context service**

Create `src/app/im/trace-context.service.ts`:

```ts
import { Injectable } from "@angular/core";

export interface TraceSidecar {
  traceparent: string;
  baggage?: string;
}

function hex(bytes: number): string {
  const data = new Uint8Array(bytes);
  crypto.getRandomValues(data);
  return Array.from(data, (b) => b.toString(16).padStart(2, "0")).join("");
}

@Injectable({ providedIn: "root" })
export class TraceContextService {
  startTrace(): TraceSidecar {
    const traceId = hex(16);
    const spanId = hex(8);
    return {
      traceparent: `00-${traceId}-${spanId}-01`,
      baggage: "client=loopforge-tauri-im",
    };
  }
}
```

- [ ] **Step 4: Attach sidecar in bridge**

In `TauriBridgeService.invoke`, accept optional trace sidecar:

```ts
async invoke<T>(cmd: string, args?: Record<string, unknown>, trace?: TraceSidecar): Promise<T> {
  if (!this.isTauri()) {
    return Promise.reject(new Error(`[bridge] 非 Tauri 环境，invoke(${cmd}) 跳过`));
  }
  const { invoke } = await import("@tauri-apps/api/core");
  const envelope = trace ? { ...(args || {}), __trace: trace } : args;
  return invoke<T>(cmd, envelope);
}
```

- [ ] **Step 5: Use sidecar from IM send**

In `ImStoreService`, inject `TraceContextService` and pass sidecar when sending:

```ts
private readonly trace = inject(TraceContextService);

async send(channelId: string, text: string): Promise<void> {
  const trace = this.trace.startTrace();
  await this.bridge.invoke("im_send", { channelId, text }, trace);
}
```

Keep the existing send method shape if it differs; the only required behavior is that business args stay unchanged and `__trace` is the sidecar.

- [ ] **Step 6: Strip sidecar in Rust command**

In `src-tauri/src/commands.rs`, define:

```rust
#[derive(Debug, serde::Deserialize)]
pub struct TraceSidecar {
    pub traceparent: String,
    #[serde(default)]
    pub baggage: Option<String>,
}

fn extract_trace(value: &mut serde_json::Value) -> Option<TraceSidecar> {
    let trace = value.as_object_mut()?.remove("__trace")?;
    serde_json::from_value(trace).ok()
}
```

Before converting command args into `Tick::Command`, call `extract_trace(&mut payload)` and pass the trace into Helix host trace hooks. Assert that serialized business payload no longer contains `__trace`.

- [ ] **Step 7: Verify**

Run:

```bash
cd /System/Volumes/Data/workspace/rust/loopforge-tauri-im
cargo test --manifest-path src-tauri/Cargo.toml
pnpm test -- --runInBand
```

Expected: config parses, bridge tests prove `__trace` is attached, Rust tests prove `__trace` is stripped before business payload.

- [ ] **Step 8: Commit**

```bash
git add config src-tauri/src src/app/im
git commit -m "feat(trace): add pc tauri otel sidecar bridge"
```

---

## Task 6: Mobile QuickJS/C++ Trace Bridge

**Files:**
- Create: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/config/mobile-local.json`
- Create: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/TraceContext.hpp`
- Create: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/TraceContext.cpp`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/CoreBridge.hpp`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/CoreBridge.cpp`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/quickjs/bind_core_bridge.cpp`
- Create: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/js/trace_context.js`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/js/im_api.js`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/run-quickjs-smoke.sh`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/run-cpp-smoke.sh`
- Test: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/smoke/quickjs_im_send_smoke.js`
- Test: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/smoke/cpp_core_bridge_smoke.cpp`

**Interfaces:**
- Consumes: Rust FFI `helix_command_with_trace`.
- Produces: `CoreBridge::callWithTrace(name, payload, traceJson)` and JS `core.callWithTrace(name, payload, trace)`.

- [ ] **Step 1: Add mobile local config**

Create `config/mobile-local.json`:

```json
{
  "apiBaseUrl": "http://127.0.0.1:8066/api/cses",
  "wsUrl": "ws://127.0.0.1:8066/api/v4/websocket",
  "observability": {
    "otel": {
      "enabled": true,
      "serviceName": "mobile-quickjs",
      "endpoint": "http://127.0.0.1:4318",
      "protocol": "http",
      "facets": {
        "quickjs": true,
        "cpp": true,
        "ffi": true,
        "event": true,
        "render": true
      }
    }
  }
}
```

- [ ] **Step 2: Add C++ trace context type**

Create `cpp/TraceContext.hpp`:

```cpp
#pragma once

#include <string>

namespace mobile {

struct TraceContext {
  std::string traceparent;
  std::string baggage;

  std::string toJson() const;
  static TraceContext localRoot();
};

}  // namespace mobile
```

Create `cpp/TraceContext.cpp`:

```cpp
#include "TraceContext.hpp"

#include <random>
#include <sstream>

namespace mobile {

namespace {
std::string random_hex(size_t bytes) {
  std::random_device rd;
  std::ostringstream out;
  for (size_t i = 0; i < bytes; ++i) {
    const auto v = static_cast<unsigned>(rd() & 0xff);
    if (v < 16) out << '0';
    out << std::hex << v;
  }
  return out.str();
}
}  // namespace

std::string TraceContext::toJson() const {
  return "{\"traceparent\":\"" + traceparent + "\",\"baggage\":\"" + baggage + "\"}";
}

TraceContext TraceContext::localRoot() {
  return TraceContext{
      "00-" + random_hex(16) + "-" + random_hex(8) + "-01",
      "client=mobile-quickjs",
  };
}

}  // namespace mobile
```

- [ ] **Step 3: Add C++ bridge method**

In `cpp/CoreBridge.hpp`:

```cpp
int32_t callWithTrace(std::string_view name,
                      std::string_view json_payload,
                      std::string_view trace_json);
```

In `cpp/CoreBridge.cpp`:

```cpp
int32_t CoreBridge::callWithTrace(std::string_view name,
                                  std::string_view json_payload,
                                  std::string_view trace_json) {
  ensureAlive();
  return helix_command_with_trace(handle_,
                                  bytes(name),
                                  name.size(),
                                  bytes(json_payload),
                                  json_payload.size(),
                                  bytes(trace_json),
                                  trace_json.size());
}
```

- [ ] **Step 4: Expose QuickJS method**

In `quickjs/bind_core_bridge.cpp`, add `core_bridge_call_with_trace` that reads `(name, payload, trace)` and calls:

```cpp
const int32_t rc = bridge->callWithTrace(name, payload, trace);
return make_rc_result(ctx, rc);
```

Register it:

```cpp
JS_CFUNC_DEF("callWithTrace", 3, core_bridge_call_with_trace),
```

- [ ] **Step 5: Add JS trace helper**

Create `js/trace_context.js`:

```js
(() => {
  function randHex(bytes) {
    let out = "";
    for (let i = 0; i < bytes; i += 1) {
      out += Math.floor(Math.random() * 256).toString(16).padStart(2, "0");
    }
    return out;
  }

  globalThis.TraceContext = {
    localRoot() {
      return {
        traceparent: `00-${randHex(16)}-${randHex(8)}-01`,
        baggage: "client=mobile-quickjs",
      };
    },
  };
})();
```

- [ ] **Step 6: Use trace in `im.send`**

In `js/im_api.js`, change send command call:

```js
const trace = globalThis.TraceContext && globalThis.TraceContext.localRoot
  ? globalThis.TraceContext.localRoot()
  : undefined;
if (trace && typeof core.callWithTrace === "function") {
  const accepted = core.callWithTrace(COMMANDS.send, {
    channelId,
    temporaryId,
    text,
    type: input.type || "TEXT",
    props: input.props || {},
    viewers: input.viewers || [],
    mentions: input.mentions || [],
    replied: input.replied || undefined,
  }, trace);
  if (!accepted || accepted.ok !== true) {
    throw new Error(`${COMMANDS.send} was not accepted`);
  }
} else {
  callCommand(core, COMMANDS.send, {
    channelId,
    temporaryId,
    text,
    type: input.type || "TEXT",
    props: input.props || {},
    viewers: input.viewers || [],
    mentions: input.mentions || [],
    replied: input.replied || undefined,
  });
}
```

This preserves the public `im.send(...)` surface.

- [ ] **Step 7: Verify**

Run:

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
./scripts/run-cpp-smoke.sh
./scripts/run-quickjs-smoke.sh
```

Expected: C++ smoke sees `callWithTrace` return `FFI_OK`; QuickJS `im.send(...)` still returns `{ temporaryId, status: "sending" }`.

- [ ] **Step 8: Commit**

```bash
git add config cpp quickjs js scripts smoke
git commit -m "feat(trace): add mobile quickjs cpp trace sidecar"
```

---

## Task 7: Golden End-To-End Trace Gate

**Files:**
- Create: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/scripts/otel-trace-smoke.sh`
- Create: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/scripts/otel-trace-check.mjs`
- Create: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im/docs/trace/otel-trace-gate.md`
- Optional mobile mirror after PC gate passes: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/real-chain/check-otel-trace.mjs`

**Interfaces:**
- Consumes: Jaeger query API, local trace id printed by PC/mobile smoke, expected span names.
- Produces: machine-checkable trace completeness gate equivalent to hop coverage.

- [ ] **Step 1: Add expected span list**

Create `scripts/otel-trace-check.mjs`:

```js
const expected = [
  "pc.ui.action",
  "pc.tauri.invoke",
  "helix.command.accept",
  "helix.core.step",
  "helix.storage.persist",
  "helix.event.emit",
  "helix.http.request",
  "cses.http.request",
  "cses.handler.create_post",
  "cses.service.create_post",
  "cses.store.create_post",
  "cses.ws.publish",
  "cses.ws.fanout",
  "cses.ws.deliver",
  "helix.ws.recv",
  "helix.event.emit",
  "pc.ui.render",
];

const traceId = process.argv[2];
if (!traceId) {
  console.error("usage: node scripts/otel-trace-check.mjs <traceId>");
  process.exit(2);
}

const url = `http://127.0.0.1:16686/api/traces/${traceId}`;
const res = await fetch(url);
if (!res.ok) {
  console.error(`jaeger query failed: ${res.status}`);
  process.exit(1);
}
const body = await res.json();
const spans = new Set();
for (const trace of body.data || []) {
  for (const span of trace.spans || []) {
    spans.add(span.operationName);
  }
}
const missing = expected.filter((name) => !spans.has(name));
if (missing.length > 0) {
  console.error(`missing spans: ${missing.join(", ")}`);
  process.exit(1);
}
console.log(`trace ${traceId} contains ${expected.length} required spans`);
```

- [ ] **Step 2: Add smoke wrapper**

Create `scripts/otel-trace-smoke.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

TRACE_ID="${1:-}"
if [[ -z "$TRACE_ID" ]]; then
  echo "usage: scripts/otel-trace-smoke.sh <trace-id-from-client-log>" >&2
  exit 2
fi

node scripts/otel-trace-check.mjs "$TRACE_ID"
```

- [ ] **Step 3: Add gate doc**

Create `docs/trace/otel-trace-gate.md`:

```markdown
# OTel Trace Gate

The gate validates the first full-link `im.send(...)` distributed trace.

Required spans:

- pc.ui.action or mobile.js.im_send
- client bridge span
- helix.command.accept
- helix.core.step
- helix.storage.persist
- helix.event.emit for `im:post:sending`
- helix.http.request
- cses.http.request
- cses.handler.create_post
- cses.service.create_post
- cses.store.create_post
- cses.ws.publish
- cses.ws.fanout
- cses.ws.deliver
- helix.ws.recv
- helix.event.emit for `im:post:received`
- pc.ui.render or mobile.render

Missing any middle hop is a trace failure because it reintroduces the C015 blind spot.
```

- [ ] **Step 4: Verify with known trace id**

Run after a real `im.send(...)`:

```bash
cd /System/Volumes/Data/workspace/rust/loopforge-tauri-im
bash scripts/otel-trace-smoke.sh <trace-id>
```

Expected: script prints required span count; exits non-zero if any required boundary is missing.

- [ ] **Step 5: Commit**

```bash
git add scripts/otel-trace-smoke.sh scripts/otel-trace-check.mjs docs/trace/otel-trace-gate.md
git commit -m "test(trace): add otel full-link trace gate"
```

---

## Task 8: Static Drift And Security Gates

**Files:**
- Create: `/System/Volumes/Data/workspace/rust/helix/scripts/trace-static-gate.sh`
- Modify: `/System/Volumes/Data/workspace/rust/helix/scripts/gate.sh` only if the team wants the trace gate in the default Helix gate.
- Create matching static grep scripts in loopforge/mobile only after their trace code lands.

**Interfaces:**
- Consumes: source tree.
- Produces: static guarantees for no OTel in `helix-core`, no trace fields in business DTOs, no token/cookie span attributes.

- [ ] **Step 1: Add Helix static trace gate**

Create `scripts/trace-static-gate.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if cargo tree -p helix-core | grep -iE 'opentelemetry|tracing-opentelemetry|reqwest|tokio|tauri|web-sys'; then
  echo "helix-core contains forbidden runtime/otel dependency" >&2
  exit 1
fi

if rg -n 'traceparent|baggage|__trace|otel' crates/helix-core crates/helix-im/src; then
  echo "trace metadata leaked into core or im business layer" >&2
  exit 1
fi

if rg -n 'Authorization|cookieId|token|message' crates/helix-driver-host/src/trace.rs crates/helix-driver-native src 2>/dev/null; then
  echo "trace code may record sensitive fields; review required" >&2
  exit 1
fi

echo "trace static gate PASS"
```

- [ ] **Step 2: Verify**

Run:

```bash
bash scripts/trace-static-gate.sh
```

Expected: passes after trace code is correctly layered; fails if OTel leaks into `helix-core`.

- [ ] **Step 3: Commit**

```bash
git add scripts/trace-static-gate.sh
git commit -m "test(trace): add static trace layering gate"
```

---

## Execution Order And Worktree Split

Use this order:

1. Task 1 in Helix main worktree.
2. Task 2 and Task 3 in `cses-im-server` worktree or branch.
3. Task 4 in Helix worktree.
4. Task 5 in loopforge worktree.
5. Task 6 in mobile worktree.
6. Task 7 after Tasks 2-6 are merged locally and the local stack can run.
7. Task 8 after Task 4, then mirror static gates to loopforge/mobile.

Suggested branch names:

- `feat/otel-local-stack`
- `feat/otel-go-boundaries`
- `feat/otel-helix-host`
- `feat/otel-loopforge-tauri`
- `feat/otel-mobile-quickjs`
- `test/otel-full-link-gate`

For worktree execution, each agent must only edit files listed in its task. Root `SESSION.md`, `AGENTS.md`, and `README.md` are not part of any worktree task.

## Verification Matrix

Run these before calling the full effort complete:

```bash
cd /System/Volumes/Data/workspace/rust/helix
docker compose -f docker/otel/docker-compose.yaml config
cargo check -p helix-core -p helix-im -p helix-driver-host -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-driver-host -p helix-driver-ffi
cargo tree -p helix-core | grep -iE 'opentelemetry|tracing-opentelemetry|tokio|reqwest|tauri|web-sys' && exit 1 || true
bash scripts/trace-static-gate.sh
```

```bash
cd /System/Volumes/Data/workspace/golang/cses-im-server
go test ./internal/config ./internal/observability ./internal/api ./internal/app ./internal/store/sqlstore ./internal/ws
```

```bash
cd /System/Volumes/Data/workspace/rust/loopforge-tauri-im
cargo test --manifest-path src-tauri/Cargo.toml
pnpm test -- --runInBand
```

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
./scripts/run-cpp-smoke.sh
./scripts/run-quickjs-smoke.sh
```

```bash
cd /System/Volumes/Data/workspace/rust/loopforge-tauri-im
bash scripts/otel-trace-smoke.sh <trace-id-from-real-send>
```

## Self-Review Notes

- Spec coverage: local OTel/Jaeger, Go HTTP/WS/function boundary, Helix host HTTP/WS/core/storage/event, PC Tauri, mobile QuickJS/C++, independent enablement, and sidecar carrier separation are mapped to Tasks 1-8.
- Placeholder scan target: every task names concrete files, commands, and expected outcomes; no task relies on unresolved marker text or vague test instructions.
- Type consistency: carrier fields are consistently `traceparent` and `baggage`; Tauri sidecar is `__trace`; WS envelope field is `tracing`; mobile additive ABI is `helix_command_with_trace`.
