# Contract: InteractionFrame V2 for GS01-GS04

## Scope

This is the implementation contract between `helix-im` and every PC/mobile/Web frame consumer for synchronisation recovery. It supplements, and does not replace, the V2 master contract. It is semantic: no DOM tag, Angular component, Flutter widget, CSS class, or raw WS event appears here.

## Producer Precondition

`helix-im` may emit a recovery Frame only after a correlated `PersistAtomic` succeeds. A connection event, HTTP 2xx, parsed response, or `PersistFire` is not sufficient.

```text
HostObservation / inbound fact
  -> Tick
  -> Effect::PersistAtomic
  -> Tick::PortReply(PersistOk)
  -> SurfaceComposer
  -> InteractionFrameV2
```

## Envelope

```json
{
  "schemaVersion": 2,
  "transactionId": "opaque-correlation-derived-id",
  "causationId": "optional-source-request-or-sync-correlation",
  "runtime": {
    "connection": "online",
    "initialSync": "ready",
    "recovery": "recovered"
  },
  "mutations": ["one unique mutation per surface+scope slot"],
  "directives": ["optional semantic post-commit work"]
}
```

### Envelope invariants

- `schemaVersion` is exactly `2`; unknown fields and unsupported schema are rejected whole-frame.
- `transactionId` is nonempty and deduplicated by consumer bounded transaction history.
- each `(surface, canonicalScope)` appears at most once;
- runtime and every attached mutation validate before any cache mutation;
- a Frame with any attached revision/epoch gap is not partially applied;
- renderer accepts no raw `im:*` payload as a substitute for a rejected Frame.

## Recovery Runtime

| Field | Values in this feature | Meaning |
|---|---|---|
| `connection` | `connecting | online | offline` | transport state, not completion. |
| `initialSync` | `idle | syncing | ready | failed` | login/bootstrap progress. |
| `recovery` | `idle | syncing | recovered | failed | blocked` | reconnect/gap recovery progress. |

`initialSync=syncing` or `recovery=syncing` is rendered as a global status. It does not disable valid existing channel actions. `ready/recovered` may only accompany committed facts.

## Mutation Contract

Every mutation has:

```json
{
  "surface": "message.composer",
  "scope": { "kind": "channel", "channelId": "c-1" },
  "surfaceEpoch": 7,
  "revision": 12,
  "baseRevision": 11,
  "causationId": "sync-correlation",
  "mutation": { "kind": "replace", "vm": {} }
}
```

Consumer rules:

1. Canonicalize scope and find an attached slot. Ignore nonattached slots without inventing an attachment.
2. Stage each attached replacement only when `surfaceEpoch` is current and `baseRevision` agrees with the slot. A duplicate/stale frame is not a partial success.
3. On a revision/epoch/scope error, keep every old slot, emit a scoped resync request, and do not call a raw reducer.
4. On full validation, replace all staged slots and runtime with one cache version publication.
5. Only then run directives in declared order and send `FrameApplied` ack. Directive success/failure may have a separate `DirectiveApplied` ack.

## Composer Bundle

For a currently opened channel that becomes known through GS03/GS04, the producer emits exactly one atomic interaction bundle containing:

- `message.channel-summary@Channel(channelId)`;
- `message.composer@Channel(channelId)`;
- runtime recovery state;
- optional semantic directive such as reveal/focus only after cache commit.

Before committed hydration, no enabled `im.send-message` ActionBinding exists. A disabled binding may be shown only with a semantic reason; it is not a permission decision made by the platform.

## ActionBinding

```json
{
  "actionRef": "opaque-helix-ref",
  "kind": "im.send-message",
  "inputShape": "message.compose.text.v1",
  "surfaceEpoch": 7,
  "enabled": true
}
```

- `actionRef` is opaque and surface-scoped; UI must not concatenate/repair it.
- invocation returns the source `surfaceEpoch` and observed revision verbatim.
- identity switch, permission change, channel close and resync invalidate prior bindings fail-closed.
- accepting an invocation only means queued/owned; sync completion and final business UI remain Frame-authoritative.

## Required GS01-GS04 Bundles

| Scenario | Required V2 output after PersistOk |
|---|---|
| GS01 bootstrap | runtime syncing then `left-dialog-list + unread-summary + runtime ready` in one Frame; route-restored channel gets summary+composer bundle. |
| GS02 server notify/offline gap | runtime recovery syncing; after commit, dialog+unread together, plus only attached timeline/thread/summary/composer affected by facts. |
| GS03 unknown channel | hydration phase has no enabled composer; after commit, summary+composer+runtime in one Frame. |
| GS04 reconnect/relogin | old session scope/bindings invalidated; only new actor's committed scope receives recovery bundle. |

## Consumer Ack and Resync

```text
FrameApplied { transactionId, appliedSlots, clientSessionEpoch }
DirectiveApplied { transactionId, directiveId, status }
SurfaceResyncRequest { surface, scope, lastAcceptedRevision, reason }
```

Ack is a receipt of mechanical consumer progress, not a second business source. A resync request asks Helix for a scope snapshot; it never asks the renderer to replay raw events.
