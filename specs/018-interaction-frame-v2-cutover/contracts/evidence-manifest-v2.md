# Contract: V2 Cutover Evidence Manifest

## Required Root Fields

```text
schemaVersion = 2
candidateId
runId
evidenceEpoch
helixRevision
clientRevision
matrixHash
fixtureHash
startedAt
endedAt
wallTimeMs
phase = V2OnlyFirstRun | PostDeletionRun
```

## Required Gate Facets

```text
gateId
actor.commandOrObservation
actor.http
actor.wsOrSync
observer.http
observer.wsOrSync
go.request
go.authority
pg.beforeAfterDelta
sqlite.beforeAfterDelta
helix.tickEffects
helix.persist
helix.frameV2
client.frameApplied
client.dom
client.visual444
client.visual678
noMock
codeReference
```

Gate 可声明某 facet 为 `not_applicable`，但必须给出机器可审查原因；缺失、空字符串或旧 run artifact 均不能替代。

## Counters

```text
v2Frames > 0
v1Frames = 0
rawBusinessFallbacks = 0
frameAppliedSuccess = committedV2Transactions
requiredFail = 0
requiredSkip = 0
```

## Database Evidence

- 查询必须包含 Gate identity、actor/observer、主键和运行时间窗口。
- 保存 query hash、参数摘要、行 identity 与 before/after delta；不保存凭据或无界正文。
- PG/SQLite 都只读；无权限时 Gate 为 BLOCKED。

## Artifact Isolation

`PostDeletionRun` 必须使用不同 candidateId、runId、evidenceEpoch 和 artifact root。Reducer 遇到删除前路径或 hash 时 fail-closed。
