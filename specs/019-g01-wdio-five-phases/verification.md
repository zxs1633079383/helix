# Verification Procedure

## Required Runs

- **V-001 Production Boundary**: `yarn message:wdio:production-boundary`
  - Expected: production artifact contains zero WDIO plugin, capability, Mock bridge or test log ingress.
- **V-002 Mock/UI Lane**: `yarn message:wdio:g01:mock`
  - Expected: UI-only、`sending -> sent`、`sending -> failed`、IPC reject and reset/restore cases all pass within the declared budget; result level never exceeds `mock`.
- **V-003 Real Integration Lane**: `yarn message:wdio:g01:integration`
  - Expected: no Mock remains; health、read-only query and send command execute through real Tauri/Helix; matching Frame and DOM are present with correlated console and Tauri logs.
- **V-004 Single Real-Chain Lane**: `yarn message:wdio:g01:single-real`
  - Expected: Actor UI、Go HTTP、SQLite、PG、Frame、FrameApplied and Actor DOM correlate; P95 scenario time is at most 15 seconds.
- **V-005 Dual Real-Chain Lane**: `yarn message:wdio:g01:dual-real`
  - Expected: isolated 444/678 instances correlate Actor send and Observer receive across all required evidence planes; P95 scenario time is at most 30 seconds.
- **V-006 Report V1 Consistency**: `yarn message:wdio:report-contract`
  - Expected: Spec/JUnit/summary/manifest counts and process exit code agree; one injected failure retains screenshot and redacted dual-side logs.
- **V-007 Performance Contract**: `yarn message:wdio:g01:performance`
  - Expected: Mock P95 <= 10 seconds, single-real P95 <= 15 seconds, dual-real P95 <= 30 seconds, one topology launch, zero ordinary profile restarts, and every slow case has phase attribution.
- **V-008 Sensitive Data Scan**: `yarn message:wdio:artifact-redaction`
  - Expected: zero password、token、authorization、Cookie、raw business payload or unapproved message content occurrences.

## Evidence Rules

- Store raw command output beneath `evidence/` and register each run with
  `loop_engine.py record-evidence`; do not replace a failed output with prose.
- Each file records command、candidate/pin、run id、start/end、wall time、result、
  retry、skip and blocker.
- Mock、integration、single-real and dual-real evidence IDs are independent and
  cannot satisfy one another.
- A failed first attempt remains immutable even if a later retry passes.
- SQLite and PG evidence is read-only and belongs to the same fresh run correlation.
- Reporter attachments contain only allowlisted, redacted summaries; raw sensitive
  logs are not copied into the report tree.
