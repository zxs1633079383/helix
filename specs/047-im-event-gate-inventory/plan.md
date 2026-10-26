# Implementation Plan: MessageV3 Gate Event 硬切重构

**Branch**: `reactor/MessageV3-helix` | **Date**: 2026-07-29 |
**Spec**: [spec.md](./spec.md)

**Input**: 在同一 Spec 047 内完成冻结 30 Gate 的 Helix MessageV3 重构：以
`Effect::Emit { event: DomainEventBytes }`（bytes 载荷为 `{event,data}`）为唯一跨端业务合同，按 Gate 删除 InteractionFrame V2 与旧 V1
inbound/outbound，并由真实 Rust 回放测试生成被 Git 忽略的 `.v1` canonical JSON。

## Summary

重构采用“共享基础一次建立、业务 Gate 逐个硬切、共享旧壳最后清零”的方式。`helix-core`
与 driver 的通用 `Tick/Effect/DomainEventBytes/EventSink` 边界保持不变；变化集中在
`helix-im`：将现有 Frame/Surface projector 和旧 raw compatibility 分支收敛为按领域分组的
MessageV3 event builder。每个 durable event 只允许在 matching `PersistOk` 后从本地事实读回，
然后生成唯一 `{event,data}`。

每个 Gate 新增一个生产 `ImModule::handle` 回放测试类。测试在内存中断言完整
Command/Inbound/Effect/PersistOk/Emit 顺序，并在 `HELIX_GATE_ARTIFACT_DIR` 存在时写出
`.v1/gates/<Gate>/generated/` 四份 JSON。现有 JavaScript 生成器只负责调用测试、建索引、
校验 schema/hash，不再拼业务字段。本版本不修改或同步 cses-client/Mobile。

## Technical Context

**Language/Version**: Rust 1.95.0、Edition 2021；Node.js 仅作证据编排

**Primary Dependencies**: `helix-core`、`bytes`、`serde`、`serde_json`、`thiserror`；
测试侧复用现有 scenario runner，并使用 `sha2`

**Storage**: 远端 PostgreSQL 17.6 只作 schema/权威事实依据；本地 tenant SQLite 是
PersistOk 后业务终态来源；测试使用 sans-IO `StorageOp/PortReply`

**Testing**: 30 个 Gate 各自使用独立目录和显式 `[[test]]` target；目录先按
`message_v3/{post,schedule,timeline,read,channel,channel_member,sync}` 领域分包，再按
Gate 分目录；机器 `domain=channel-member` 映射到 Rust 包 `channel_member`；
聚焦 `cargo test -p helix-im --test message_v3_<gate> -- --nocapture`；最终
`cargo test -p helix-im` 与三端 Rust check/build

**Target Platform**: native、FFI、Web 共用 `DomainEventBytes`；本版本只改 Helix 仓

**Project Type**: runtime-agnostic Rust workspace 业务模块

**Performance Goals**: 单次 inbound 的 event 构造为 O(1) 或 O(k) 有界批次；禁止
Frame/Surface 二次投影和跨 event join；JSON 只序列化一次

**Constraints**: core 零 runtime；durable emit 必须晚于 PersistOk；单 Gate 禁止新旧双写；
`.v1` 被 Git 忽略；不修改 PC/Mobile；所有新增/修改函数有紧邻精简注释

**Scale/Scope**: 精确 30 Gate，8 个依赖阶段，30 个测试 target，120 个 generated JSON
基础文件；G-03 与 G-17 至 G-27 明确不在分母

## Constitution Check

*GATE: Phase 0 前与 Phase 1 后均必须通过。*

| 原则 | 设计响应 | 状态 |
|---|---|---|
| Runtime-Agnostic | 新 event/canonical 模块只依赖 `helix-core` 与 serde/bytes | PASS |
| Deterministic/PersistOk | 时间、ID、viewer 由剧本注入；terminal emit 只从持久读回释放 | PASS |
| MessageV3 唯一真源 | 唯一出站为 `DomainEventBytes({event,data})`，无 Frame/Surface 包装 | PASS |
| Evidence First | 每 Gate 真实回放测试同时断言顺序并可写 canonical JSON/provenance | PASS |
| Per-Gate Hard Cut | Gate 专属旧链当阶段删除，共享旧壳仅在最后引用消失后删除 | PASS |
| Loop Admission | R1-R3 由 `review-ledger.json` 控制，未通过不得实现 | PASS |

**必须同步的治理真源**：Constitution 已升为 2.0.0；实现前还需按任务更新
`docs/GOAL.md`、`docs/harness/decisions.md`、`docs/ROADMAP.md`、`AGENTS.md` 中失效的
Frame/Surface 目标描述。历史证据可保留，但必须明确标记为 retired。

## Loop Detail Contract

### Inputs and Context

- **I-001**: `spec.md` 的冻结 30 Gate、双视角终态和 FR-037 至 FR-048。
- **I-002**: `contracts/storage-schema-lock.v1.json` 的远端/本地真实表列与 JSON key shape。
- **I-003**: `crates/helix-im/src` 当前 Frame/Surface、旧 raw emit、WS parser、query、
  correlation 和 PersistOk 路径。
- **I-004**: `crates/helix-im/tests` 当前 Gate 回放、fixture producer 和 focused tests。
- **I-005**: `.v1/gates/<Gate>/inbound-sample.json|outbound-sample.json` 是冻结人工审核输入，
  不被脚本重写；Rust 测试另写 `generated/*` 并成为实现后的唯一权威合同，二者不得覆盖或
  倒灌字段。

### Action Chain

- **A-001**: `Tick::Command|Inbound` → Gate parser → `Effect::{Http,Persist,Timer}`。
- **A-002**: correlated authority/`PortReply` → atomic local write → matching `PersistOk`。
- **A-003**: PersistOk → bounded local readback/known committed values → MessageV3 builder。
- **A-004**: builder → 单一 `Effect::Emit(DomainEventBytes({event,data}))` → driver 原样交付。
- **A-005**: Gate replay test → schema validation → generated JSON + provenance + SHA-256。
- **A-006**: Gate evidence pass → 删除该 Gate 的 Frame/V1 专属代码 → 零引用检查 → ledger
  `MIGRATED`。

### State Contract

- **S-001**: 单 Gate 状态
  `PLANNED -> IN_PROGRESS -> TEST_RED -> MESSAGE_V3_GREEN -> LEGACY_REMOVED -> MIGRATED`。
- **S-002**: 任一 schema、Persist barrier、双视角或删除检查失败：
  `IN_PROGRESS|TEST_RED|MESSAGE_V3_GREEN -> BLOCKED`，不得开始下一 Gate。
- **S-003**: `.v1` artifact 状态：
  `MISSING -> GENERATED -> SCHEMA_VALID -> HASHED`；手写或来源不明进入 `REJECTED`。
- **S-004**: 共享旧壳状态：
  `REFERENCED -> LAST_GATE_REMOVED -> ZERO_REFERENCE -> DELETED`。
- **S-005**: P0/P7 共享 workflow phase 只写 `review-ledger.json`，单 Gate 迁移状态只写
  `gate-migration.v1.json`；checker 合并两者后断言 `IN_PROGRESS <= 1`。

### Validators

- **V-001**: Gate focused Rust test证明真实 `ImModule::handle` 与 PersistOk/Emit 顺序。
- **V-002**: JSON Schema 校验证明 envelope、sender/receiver 与 provenance 结构。
- **V-003**: 重复生成 hash 对账证明 canonical artifact 确定性。
- **V-004**: Gate-scoped `rg` 删除清单证明已迁移 Gate 不再调用 V2/V1。
- **V-005**: manifest 集合检查证明 30 Gate missing/extra/duplicate 均为 0。
- **V-006**: `cargo check`、`cargo test -p helix-im`、wasm build 证明共享删除未破坏三端 Rust。
- **V-007**: P0 manifest 校验证明 30 Gate 的生产 owner 与 V2/V1 删除引用在任何生产编码前
  已登记为 `PENDING_REMOVAL/N/A/DEFERRED_BY_SCOPE`，未知项不得使用空数组或 `TODO` 逃逸。
- **V-008**: 公共红测试必须先证明 `PersistOk` 前无 terminal Emit、匹配回复后恰好释放一次，
  再允许实现公共释放编排。

## Phase 0: Research

研究结论见 [research.md](./research.md)。关键决定：

1. 保留 `Tick/Effect/DomainEventBytes/EventSink` 通用边界，不创建 MessageV3 runtime。
2. 新建 `helix-im/src/event/`，按领域组织唯一 event builder；迁移后删除
   `interaction_frame.rs`、`surface_*`、`projector/` 等业务套壳。
3. 旧 WS action 的服务端 wire parser 按仍在用/compat 两类审计；保留真实 Go wire，
   删除 `legacy_event.rs` 与只服务旧合同的 alias/fallback。
4. Rust 测试是业务 JSON 唯一 producer；Node 只编排，不生成字段。
5. 本版本只产 Helix `.v1`，跨仓镜像延后并记为 `NOT_SYNCED_BY_SCOPE`。
6. P0 先生成完整 30 Gate 删除清单，再写公共或 Gate 生产代码；清单不是实现后的补作文档。

## Phase 1: Design

### Domain Model

见 [data-model.md](./data-model.md)：定义 Gate、InboundAuthority、CommittedFact、
MessageV3Event、PersonaArtifact、Provenance、MigrationEntry 与状态转换。

### Contracts

- [message-v3-event.schema.json](./contracts/message-v3-event.schema.json)
- [gate-artifact.schema.json](./contracts/gate-artifact.schema.json)
- [gate-provenance.schema.json](./contracts/gate-provenance.schema.json)
- [gate-migration.schema.json](./contracts/gate-migration.schema.json)
- 既有 [storage-schema-lock.v1.json](./contracts/storage-schema-lock.v1.json)

### Migration Phases

| 阶段 | Gate | 主要目标 | 阶段删除 |
|---|---|---|---|
| P0 | shared | 完整删除清单、event envelope/builders、Persist 红测、artifact writer、schema、ledger | JavaScript 手拼 payload |
| P1 | G-15a、G-01 | 建群前置与文本发送主链 | 对应 Frame composer/旧 send emit |
| P2 | G-02、G-03a、G-04、G-04a、G-05、G-06、G-07、G-14、G-14a | Post 变更与双视角 | post Frame/旧 compatibility |
| P3 | G-08、G-09 | schedule 私有终态 | schedule Surface/旧 emit |
| P4 | G-10、G-10a、G-10b、G-10c、G-10d、G-11、G-11a、G-12、G-13 | timeline/read | timeline projector/导航 Frame |
| P5 | G-15、G-16、G-16b、G-16c | topic/lifecycle/member/settings | channel/member Surface |
| P6 | G-S1、G-S2、G-S3、G-S4 | 冷启动、sync、gap、陌生频道恢复 | sync Frame/旧恢复 emit |
| P7 | shared | 共享 V2/V1 零引用、模块出口和文档收口 | Frame/Surface/Action 全壳 |

每个 Gate 内部固定采用：测试红灯 → MessageV3 builder/接码 → JSON 生成 → schema/hash →
Gate 专属旧链删除 → focused 回归 → ledger 更新。阶段内 Gate 仍然串行。

### Post-Design Constitution Re-check

设计未引入新 runtime、双写或平台 reducer；生成物只来自 Rust 生产回放；PC/Mobile 同步明确
延期且不冒充本版本完成。Constitution 2.0 六项检查继续 PASS。

## Project Structure

### Documentation

```text
specs/047-im-event-gate-inventory/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── verification.md
├── review-ledger.json
├── tasks.md
├── contracts/
│   ├── storage-schema-lock.v1.json
│   ├── message-v3-event.schema.json
│   ├── gate-artifact.schema.json
│   ├── gate-provenance.schema.json
│   ├── gate-migration.schema.json
│   └── gate-migration.v1.json      # T005 生成的 30 Gate 迁移清单实例
└── .v1/                         # ignored runtime evidence
    └── gates/<Gate>/generated/
        ├── inbound.json
        ├── outbound.sender.json
        ├── outbound.receiver.json
        └── provenance.json
```

### Source Code

```text
crates/helix-im/src/
├── event/
│   ├── mod.rs                   # 唯一 MessageV3 envelope/build 入口
│   ├── post.rs
│   ├── channel.rs
│   ├── channel_member.rs
│   ├── schedule.rs
│   ├── timeline.rs
│   ├── read.rs
│   └── sync.rs
├── module/                      # Command/Inbound/PortReply 编排
├── ws/                          # 只保留真实 Go wire parser/handler
├── query/                       # 本地事实读回，不产 Frame
├── channel/                     # 持久写程序与 cursor
└── sync/                        # 恢复状态机

crates/helix-im/
├── Cargo.toml                    # 每个嵌套 Gate 目录显式注册一个 [[test]]
└── tests/message_v3/
    ├── support/
    │   ├── artifact.rs           # JSON/sha/provenance 公共测试工具
    │   └── scenario.rs           # ImModule::handle 回放公共工具
    ├── contracts/
    │   ├── event/main.rs
    │   ├── artifact_schema/main.rs
    │   └── gate_registry/main.rs
    ├── post/
    │   ├── g01/main.rs
    │   ├── g02/main.rs
    │   ├── g03a/main.rs
    │   ├── g04/main.rs
    │   ├── g04a/main.rs
    │   ├── g05/main.rs
    │   ├── g06/main.rs
    │   ├── g07/main.rs
    │   └── g14a/main.rs
    ├── schedule/
    │   ├── g08/main.rs
    │   └── g09/main.rs
    ├── timeline/
    │   ├── g10/main.rs
    │   ├── g10a/main.rs
    │   ├── g10b/main.rs
    │   ├── g10c/main.rs
    │   ├── g10d/main.rs
    │   ├── g11/main.rs
    │   ├── g11a/main.rs
    │   └── g12/main.rs
    ├── read/
    │   ├── g13/main.rs
    │   └── g14/main.rs
    ├── channel/
    │   ├── g15a/main.rs
    │   ├── g15/main.rs
    │   ├── g16/main.rs
    │   └── g16c/main.rs
    ├── channel_member/
    │   └── g16b/main.rs
    └── sync/
        ├── gs1/main.rs
        ├── gs2/main.rs
        ├── gs3/main.rs
        └── gs4/main.rs

specs/047-im-event-gate-inventory/scripts/
└── generate-v1-evidence.mjs     # 只运行测试、索引、schema/hash 验证
```

迁移完成后从 `lib.rs` 删除 `interaction_frame`、`surface_composer`、`surface_runtime`、
`dialog_frame`、`dialog_patch`、`projector` 及只服务旧 Action/Frame 的 re-export。可复用的
消息 shaping、query 和持久逻辑必须移动到 `event/` 或原领域模块，不以旧目录名残留。

**Structure Decision**: 采用单 crate 内的深模块重构，不新增 crate 或运行时。生产代码按领域
分模块；测试也按七个业务域的 Rust package + Gate 独立目录组织，并通过 `Cargo.toml`
显式注册独立 integration target。`channel-member` 仅在 Rust 文件路径中转写为
`channel_member`。event builder 只负责“已提交事实 → event bytes”，module 负责业务编排，
driver 继续原样搬运 bytes。

## Complexity Tracking

无 Constitution 例外。30 Gate 的规模通过严格串行和共享 artifact harness 控制，不使用双写
兼容层换取并行。
