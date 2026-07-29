# Data Model: InteractionFrame V2 Cutover

## 1. CutoverCandidate

| 字段 | 规则 |
|---|---|
| candidateId | 唯一且写入所有 evidence artifact |
| helixRevision | immutable commit |
| clientRevision | cses-client commit |
| cargoPins | 四个 Helix package revision 必须相同 |
| lockHash | `Cargo.lock` hash |
| matrixHash | Strict Matrix semantic hash |
| fixtureHash | Helix/client fixture manifest hash |
| phase | `V2OnlyFirstRun` 或 `PostDeletionRun` |

状态：`Draft -> Frozen -> Running -> Passed|Failed|Blocked`。pin/hash 变化使 candidate 失效，不允许续用旧证据。

## 2. GateContract

| 字段 | 规则 |
|---|---|
| gateId | 25 个 canonical Gate 之一 |
| businessSpec | 19 个 Spec 之一 |
| actor/observer | 明确 viewer 和权限边界 |
| input | ActionBinding 或 HostObservation/Inbound/Timer |
| tick/effects | 有序且可关联 |
| persistWriteSet | 现有 SQLite 业务事实集合 |
| authorityTables | 现有 PG 权威表集合 |
| surfaces | 只包含受事实影响且已 attach 的 Surface |
| components | Angular owner |
| evidenceRequirements | Gate 专属 + 公共证据 |

## 3. InteractionFrameV2

| 字段 | 验证 |
|---|---|
| schemaVersion | 必须等于 2 |
| transactionId | candidate/run 内唯一，重复幂等 |
| causationId | 可关联 Action/Observation/authority/Persist |
| runtime | 可选运行态 mutation，不制造空业务行 |
| mutations | 非空或明确 runtime-only；每项 slot 唯一 |
| directives | 整帧提交后执行 |

未知 schema、重复 slot、错误 scope/epoch/revision、非法 VM 使整帧拒绝，不部分提交。

## 4. SurfaceSlot

| 字段 | 规则 |
|---|---|
| surfaceId/scope | 唯一 key |
| surfaceEpoch | attach/session generation；变化使旧 action/frame 失效 |
| revision | 单调 |
| state | `detached/attached/applying/ready/resync-required` |
| vm | 完整 typed VM，不是 partial JSON |

状态：`Detached -> Attached(snapshot) -> Ready -> Applying -> Ready`；gap/epoch mismatch 进入 `ResyncRequired`，保留最后一致 VM。

## 5. ActionBinding

| 字段 | 规则 |
|---|---|
| actionRef | opaque，不由组件构造 |
| actor/scope | 与 viewer 和 Surface 绑定 |
| surfaceEpoch/observedRevision | 调用时校验 |
| kind/inputSchema | Helix 固定 capability 与允许输入 |
| lease | 过期/已使用/状态变化后拒绝 |

## 6. PersistTransaction

| 字段 | 规则 |
|---|---|
| operationId/correlation | 与 authority 和 Frame causation 对齐 |
| writeSet | Gate Spec 声明的 SQLite 表/实体 |
| result | `PersistOk` 或 `PersistErr` |
| cursor/identity | 只在同一 atomic commit 内推进 |

`PersistErr` 保证业务事实、cursor、canonical identity 和成功 Frame 均不推进。

## 7. EvidenceManifestV2

| 字段 | 规则 |
|---|---|
| runId/candidateId/evidenceEpoch | 删除前后不可复用 |
| gateId | canonical Gate |
| frameCounters | v1/v2/raw fallback |
| action/effect/persist/frameAck | 每跳结果与 correlation |
| pg/sqlite | query hash、row identity、before/after delta |
| dom/visual | selector assertions 与 actor/observer screenshot |
| timing | start/end/wall time/retry/wait/block |
| verdict | PASS/FAIL/BLOCKED；缺 required facet 不得 PASS |

## 8. Existing Business Entities

本功能不新增数据库实体。各 Gate 继续使用既有 `channel`、`channel_member`、`channel_event`、`post`、`post_schedule`、message/pending/cursor/receipt/reply-tree 等模型；具体表和主键以各 Gate `09-远端PG与本地SQLite.md` 与运行态 schema 为准。
