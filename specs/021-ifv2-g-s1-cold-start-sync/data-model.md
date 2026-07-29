# Data Model: G-S1 hello 全量增量

## GateScenario

| Field | Type | Rule |
|---|---|---|
| gateId | string | 固定为 `G-S1` |
| actorId / observerId | string | canonical real lane 使用隔离双身份 |
| channelId | string | 每次 canonical run 由 CreateGroupPrerequisite 新建，不复用前一 Gate |
| command | tagged object | 仅允许 im_query_dialog_list |
| correlation | object | reqId、transactionId、causationId 与业务 ID 可跨层关联 |
| expectedTerminal | enum | success、business-failed、infrastructure-failed、recovered |

## CanonicalFrameArtifact

| Field | Type | Rule |
|---|---|---|
| fixtureId | string | Gate 内唯一并表达独立可观察阶段 |
| schemaVersion | integer | 必须为 InteractionFrame V2 |
| surfaces | array | 仅包含 message.left-dialog-list |
| frame | object | 完整 envelope、scope、revision、mutations、bindings、directives |
| rawSha256 / semanticSha256 | string | Helix、PC、Mobile 三方一致 |
| producer | string | 当前 Helix source revision + 测试入口 |

## PlatformObservation

| Field | Type | Rule |
|---|---|---|
| platform | enum | pc-mock、pc-real-actor、pc-real-observer、mobile-gtest |
| frameApplied | object | transactionId 与 revision 必须匹配 |
| typedView | object | 只能由 SurfaceRuntime apply Frame 得到 |
| visibleState | object | PC DOM 或 Mobile QuickJS typed view |
| evidenceRefs | array | 原始日志、截图、report、hash 清单 |

## GateReview

| Field | Type | Rule |
|---|---|---|
| behavior | verdict | 权限、顺序、失败、恢复一致 |
| codeBoundary | verdict | Helix 唯一业务 reducer，测试能力 production-unreachable |
| frameV2 | verdict | 字段、surface、revision、mutation、action、directive 一致 |
| evidence | verdict | required pass=100%、fail=0、skip=0 |
| final | verdict | 四项全 Green 才可 review-green |

## ColdStartParityEvidence

| Field | Type | Rule |
|---|---|---|
| runId / gateId / persona / channelId | string | 唯一定位 isolated Gate dataset 与当前可见性边界 |
| databasePathBefore / databasePathAfter | path | 必须完全相等，禁止用新文件名替代物理删库恢复 |
| deletedArtifacts | array | 只允许 SQLite main、`-wal`、`-shm`、`-journal` |
| residualArtifacts | array | Green 时必须为空 |
| remoteSnapshot / localSnapshot | normalized rows | 按稳定主键排序，字段 allowlist 必须显式 |
| remoteSha256 / localSha256 | string | Green 时必须相等 |
| missing / extra / mismatch | arrays | Green 时三个集合均为空 |
| frameApplied / visibleProjection | evidence | 证明消费收敛，但不得替代 storage parity |

## State Transitions

`Draft -> HelixGreen -> PcGreen -> MobileGreen -> ReviewGreen`

- 任一 hash/pin/source 漂移：回到 `Draft`。
- 任一 required lane 缺失：进入 `Blocked`，不得解锁下一 Gate。
- Mock、AcceptedCommand 或 HTTP 2xx 不能直接进入 `PcGreen` 或 `ReviewGreen`。
