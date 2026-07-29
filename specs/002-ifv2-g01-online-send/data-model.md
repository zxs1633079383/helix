# Data Model: SPEC-IFV2-G01-在线发送

## Identifiers and causation

| Entity | Essential fields | Invariants |
|---|---|---|
| `SendRequest` | `requestId`, trusted `actorId`, `channelId`, `submittedAt`, `corrKey` | `requestId` is unique per accepted action; client cannot choose actor/channel outside binding scope. |
| `ActionBinding` | capability=`message.create`, fixed `actionKind=im.send-message`, opaque `actionRef`, actor, surface=`message.composer`, scope=`Channel`, epoch, observed revision, expiry, input contract | Valid only for exact actor/scope/epoch/revision/capability; invocation carries only `actionRef` plus input, not an action kind, and is not a message identity/global send lock. |
| `ActionReceiptV2` | `requestId`, accepted/rejected reason | Changes host overlay only; never creates a business row. |
| `PendingSend` | `requestId`, temporary identity, channel, delivery state, attempt metadata, canonical id optional | Persisted before visible Sending; retained through terminal state under existing lifecycle rules. |

## Message facts

| Entity | Essential fields | Invariants |
|---|---|---|
| `TemporaryMessage` | temporary id, channel, author, text, `delivery=Sending|Failed`, request id | Sender-only visible through sender frame; failure retains same logical identity. |
| `CanonicalMessage` | server id, canonical channel/author/content/time/seq, source request/temp relation | Only authoritative echo/sync can create/settle it; replaces rather than adds a logical row. |
| `DeliveryVm` | `Sending`, `Sent`, `Failed { reason, retryable }` | Exactly one variant; HTTP accepted does not transition it. |
| `VersionedMessageContentVm` | kind, schema version, typed payload | Unknown kind/version rejects the frame; G01 uses text. |

## Presentation and evidence

| Entity | Essential fields | Invariants |
|---|---|---|
| `MessageRowVmV2` | identity, author ref, time, versioned content, delivery, relations, indicators, actions | One logical row through temporary-to-canonical settle; actions renew with owner row/slot. |
| `MessageComposerVm` | availability, `message.create` capability, binding, input contract | Owns authorization, not draft/cursor or message bookkeeping. |
| `InteractionFrameV2` | version, transaction id, runtime, mutations, directives, causation | Decode/validate as one transaction; no generic/unknown business VM. |
| `FrameCacheSlot` | surface, canonical scope, epoch, revision, typed VM | Cache only accepted frames; invalid patch leaves every slot unchanged. |
| `UiDirective` | id, causation, kind | Executes after `FrameApplied`; failure cannot roll back persisted frame. |
| `GsBaselineAdmission` | 001 T050 source pin, Rust/PC DOM/mobile fixture/SQLite-PG evidence locations and verdict | G01 source work unlocks only when all four current-pin verdicts are `PASS`; otherwise every G01 implementation task is blocked. |
| `EvidenceRun` | corr key, two users, channel, request, artifacts, verdict | Pass requires all hop/DB/sender/receiver/counterexample evidence. |

## Relationships

```text
GsBaselineAdmission(PASS) -> ActionBinding -> SendRequest -> PersistOk -> PendingSend
PendingSend -> TemporaryMessage -> authoritative echo/sync -> CanonicalMessage
Committed facts -> SurfaceComposer -> InteractionFrameV2 -> atomic FrameCacheSlot replacement
InteractionFrameV2 -> FrameApplied -> UiDirective
EvidenceRun -> request + effects + facts + frame/cache/DOM observations
```

## Delivery state transitions

```text
DraftOverlay --valid command--> PersistingTemporary --PersistOk--> Sending
PersistingTemporary --PersistErr--> RecoveredDraft (no row)
Sending --HTTP 2xx--> AwaitingCanonical (still Sending)
AwaitingCanonical --canonical PersistOk--> Sent
Sending/AwaitingCanonical --failure PersistOk--> Failed
Failed --valid retry--> PersistingTemporary
Sent --duplicate/late echo--> Sent
```

## Validation rules

1. Reject invalid text/binding/actor/scope/epoch/revision/reply before temporary persist.
2. Persist temporary message, pending send and correlation facts atomically; PersistErr creates no Sending Frame.
3. Persist canonical message, temporary-to-server settle and relevant cursor facts atomically before Sent Frame.
4. Only a successfully applied Sending Frame may clear that request’s recovery draft.
5. Receiver uses independently persisted canonical facts only; no temporary cross-viewer data.
