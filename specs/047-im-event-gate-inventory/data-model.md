# MessageV3 Gate Event 数据模型

## GateDefinition

| 字段 | 类型 | 约束 |
|---|---|---|
| `gate_id` | string | 冻结 30 Gate 集合内唯一 |
| `domain` | enum | `post/schedule/timeline_read/channel/sync` |
| `commands` | string[] | canonical command；可为空但须有 host observation |
| `ws_actions` | string[] | 真实 Go wire action，不含兼容 alias |
| `sender_events` | string[] | sender 可见 event 顺序 |
| `receiver_events` | string[] | receiver 可见 event 顺序；persona-local 可为空 |
| `storage_profile` | string | 指向 schema lock 的表列集合 |
| `migration_phase` | enum | `P1..P6` |

## InboundAuthority

| 字段 | 类型 | 约束 |
|---|---|---|
| `tick_kind` | enum | `Command/Inbound/PortReply/Timer` |
| `value` | JSON | 必须通过对应 parser |
| `correlation_id` | string | host 注入；跨 effect/persist/emit |
| `actor_id` | string? | 来自鉴权或持久 event，不从原消息作者猜测 |
| `remote_basis` | TableColumnRef[] | PostgreSQL 持久依据或明确 `N/A` |

## CommittedFact

| 字段 | 类型 | 约束 |
|---|---|---|
| `tenant_id` | string | sender/receiver 独立 |
| `table` | string | 必须存在于 storage schema lock |
| `primary_key` | object | O(1) 定位 |
| `columns` | object | 只含真实本地列 |
| `persist_correlation` | string | matching PersistOk |
| `event_seq` | integer? | 有序 Gate 单调不回退 |

## MessageV3Event

| 字段 | 类型 | 约束 |
|---|---|---|
| `event` | string | `im:<domain>:<action>`，Gate contract 注册 |
| `data` | object | 禁 capability、Frame、Surface、重复别名和 UI directive |

`MessageV3Event` 是唯一运行时 envelope。每个 `data` 顶层字段必须映射到本地持久列、已观测
JSON key，或在 Gate 合同中声明为 correlation/query/role 派生；未知字段 fail closed。

## PersonaArtifact

| 字段 | 类型 | 约束 |
|---|---|---|
| `schema_version` | const | `helix-im-gate-artifact/v1` |
| `gate_id` | string | 与测试类一致 |
| `perspective` | enum | `sender/receiver` |
| `events` | MessageV3Event[] | 保持生产顺序 |
| `storage_basis` | TableColumnRef[] | sender/receiver 独立 |

## Provenance

| 字段 | 类型 | 约束 |
|---|---|---|
| `schema_version` | const | `helix-im-gate-provenance/v1` |
| `gate_id` | string | 与目录一致 |
| `producer_test` | string | Rust integration test target |
| `producer_entry` | const | `ImModule::handle` |
| `helix_head` | string | 40 位 commit |
| `matrix_epoch` | integer | 当前 strict matrix epoch |
| `storage_schema_sha256` | string | 64 hex |
| `artifacts` | map | 文件名到 SHA-256 |
| `consumer_sync` | const | 本版本为 `NOT_SYNCED_BY_SCOPE` |

## MigrationEntry

| 字段 | 类型 | 约束 |
|---|---|---|
| `gate_id` | string | 冻结集合内唯一 |
| `phase` | enum | `P1..P6` |
| `state` | enum | `PLANNED/TEST_RED/MESSAGE_V3_GREEN/LEGACY_REMOVED/MIGRATED/BLOCKED` |
| `test_target` | string | 一 Gate 一个 target |
| `generated_files` | string[] | 固定四文件 |
| `legacy_inventory` | LegacyRef[] | 每项 `removed/N/A` + 证据 |
| `verification` | string[] | 聚焦命令与结果引用 |

## Relationships

```text
GateDefinition 1 ── 1 MigrationEntry
GateDefinition 1 ── * InboundAuthority
InboundAuthority 1 ── * CommittedFact
CommittedFact 1 ── * MessageV3Event
GateDefinition 1 ── 2 PersonaArtifact(sender/receiver)
PersonaArtifact * ── 1 Provenance
```

## State Transitions

```text
PLANNED
  -> TEST_RED
  -> MESSAGE_V3_GREEN
  -> LEGACY_REMOVED
  -> MIGRATED

任一验证失败 -> BLOCKED
BLOCKED 只能在同 Gate 修复后回到失败前状态
```

共享旧壳只有在所有引用 Gate 为 `MIGRATED` 后才能从 `ZERO_REFERENCE` 进入 `DELETED`。
