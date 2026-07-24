# Data Model: G01 WDIO 前五阶段

## G01Run

| 字段 | 类型 | 规则 |
|---|---|---|
| `runId` | string | 全局唯一；所有 artifact/correlation 的根键 |
| `gate` | `"G01"` | 固定 |
| `lane` | `ui-only \| mock \| integration \| single-real-chain \| dual-real-chain` | 决定拓扑与可声称范围 |
| `candidateId` | string | 绑定 test build 与 source pins |
| `startedAt/endedAt` | ISO-8601 | 单调且时区明确 |
| `status` | `pending \| running \| passed \| failed \| blocked \| needs-human` | 终态不可回退为 running |
| `personas` | TestPersona[] | 单端仅 Actor，双端 Actor+Observer |
| `performance` | PerformanceBreakdown | 所有阶段非负 |

状态：`pending -> running -> passed|failed|blocked|needs-human`。

## TestPersona

| 字段 | 类型 | 规则 |
|---|---|---|
| `role` | `actor \| observer` | Actor=444，Observer=678；报告不得保存密码 |
| `profileDir` | path | 每 run/persona 唯一 |
| `deviceIdHash` | string | 不保存原值 |
| `webdriverPort/logPort` | integer | persona 间不得冲突 |
| `pid/startMarker` | integer/string | 用于排除旧进程 |
| `artifactDir` | path | 必须位于当前 run 根目录 |
| `state` | `stopped \| starting \| ready \| failed \| recovering` | 只恢复损坏 persona |

## CommandReceipt

| 字段 | 类型 | 规则 |
|---|---|---|
| `reqId` | string | 本次 action correlation |
| `command` | `"im_send_message"` | 固定 |
| `helixCommand` | string | 与当前 Tauri 合同一致 |
| `accepted` | boolean | 仅表示入队，不表示业务完成 |

`accepted=true` 不能把 run 提升为 `single-real-chain` 或 `dual-real-chain`。

## FrameFixtureProvenance

| 字段 | 类型 | 规则 |
|---|---|---|
| `fixtureId` | string | `g01-sending|sent|failed-v2` |
| `schemaVersion` | `2` | 其他版本 fail closed |
| `helixCommit` | 40-char SHA | 与 client source pin 一致 |
| `producer` | string | Helix producer/serializer 符号 |
| `sourceKind` | `rust-export \| real-capture` | 必填 |
| `semanticSha256` | 64-char hex | 归一化动态字段后的 hash |
| `fixtureSha256` | 64-char hex | 原始文件 hash |
| `surfaces` | string[] | 由 Helix 真实输出决定，WDIO 不补造 |
| `generatedAt` | ISO-8601 | 可追踪 |

## G01MessageIdentity

| 字段 | 类型 | 规则 |
|---|---|---|
| `channelId` | string | Actor/Observer/DB/Frame 一致 |
| `markerHash` | string | 正文只用不可逆 hash |
| `temporaryIdHash` | string? | sending 阶段可用 |
| `postId` | string? | canonical terminal 后必填 |
| `eventSeq` | integer? | Observer WS/recovery 关联 |
| `reqId/trackId/transactionId` | string? | 至少形成一条无歧义关联链 |

状态：`draft -> temporary(sending) -> canonical(sent)` 或 `temporary(sending) -> failed`；late WS 只能幂等收敛，不能产生第二行。

## CorrelationEvent

| 字段 | 类型 | 规则 |
|---|---|---|
| `runId/gate/persona` | string | 必须匹配当前 run |
| `plane` | `console \| tauri \| helix \| go \| ws \| sqlite \| pg \| frame \| frame-applied \| dom` | 枚举 |
| `keyType/key` | string | allowlist correlation |
| `timestamp` | ISO-8601 | 位于 process start/end window |
| `status/count/durationMs` | scalar | 不含 raw payload |

`CorrelationIndex` 关系：一个 `G01Run` 拥有多个 event；同一 event 可按多个安全 key 建索引。

## EvidenceBundle

| 字段 | 类型 | 规则 |
|---|---|---|
| `runId/caseId/lane` | string | 唯一定位 |
| `requiredPlanes` | string[] | 由 evidence level 决定 |
| `observedPlanes` | string[] | 必须是 required 的超集才可 Green |
| `assertions` | AssertionResult[] | 失败不可被 retry 覆盖 |
| `attachments` | ArtifactRef[] | 只存相对索引/hash/大小 |
| `freshnessWindow` | start/end | 防历史证据混入 |
| `redactionVerdict` | `pass \| fail` | fail 阻断报告发布 |

## AssertionResult

| 字段 | 类型 | 规则 |
|---|---|---|
| `assertionId` | string | 稳定、可聚合 |
| `plane` | string | 对应证据面 |
| `expected/actualSummary` | scalar/object | 必须脱敏且有界 |
| `status` | `pass \| fail \| blocked \| skipped` | mandatory 不可 skipped 后 Green |
| `stage` | `setup \| action \| invoke \| frame \| dom \| evidence \| screenshot \| report` | 首次失败定位 |
| `durationMs` | integer | 非负 |

## PerformanceBreakdown

固定字段：`buildMs`、`startupMs`、`setupMs`、`actionMs`、`commandAckMs`、`frameMs`、`domMs`、`evidenceMs`、`screenshotMs`、`reportMs`、`totalMs`。warm P95 排除 `buildMs/startupMs` 和首次系统授权时间，但原值仍保留在 run manifest。

## G01ReportV1

关系：一个 report 对应一个 `G01Run`，包含多条 case result、一个 evidence manifest 和多个 artifact reference。

完整字段由 [g01-report-v1.schema.json](contracts/g01-report-v1.schema.json) 约束；结果层级不可自动升级，`dual-real-chain` 只有九面证据完整时为 passed。
