# Task 8 Report: Helix Trace Static Gate

## Scope

- Added `scripts/trace-static-gate.sh`.
- Wired it into default `scripts/gate.sh`.
- Did not modify root `AGENTS.md`, `CLAUDE.md`, `README.md`, `SESSION.md`, crates source, Cargo files, or `docs/harness/**`.

## Gate Coverage

- `helix-core` dependency tree rejects `opentelemetry`, `tracing-opentelemetry`, `reqwest`, `tokio`, `tauri`, and `web-sys`.
- `helix-core` and `helix-im/src` reject trace sidecar terms: `traceparent`, `baggage`, `__trace`, `otel`, and `opentelemetry`.
- Trace-related implementation files are discovered with `rg -l 'TraceCarrier|TraceHooks|opentelemetry|traceparent|baggage|__trace'`.
- Sensitive/business content scan is limited to trace-related files and requires a span/log/attribute context on the same line as `Authorization`, `cookie`, `cookieId`, `token`, `message`, `text`, `payload`, or `body`.
- Raw trace JSON output is rejected when `raw_json` appears in a logging/span/attribute context.

## Verification

- `bash -n scripts/trace-static-gate.sh scripts/gate.sh`
- `bash scripts/trace-static-gate.sh`
- `git diff --check`

All commands passed in this worktree.
