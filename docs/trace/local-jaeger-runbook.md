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
