# Implementation Plan: InteractionFrame V2 双仓硬切

**Branch**: `018-interaction-frame-v2-cutover` | **Date**: 2026-07-23 | **Spec**: [spec.md](spec.md)

**Input**: GS01-GS04、G01-G02、G04-G16 的 19 份业务 Spec、25 Gate Strict Matrix，以及用户确认的“V2-only 运行前置 → 首轮全链 → 删除 V1 → 二次全链”顺序。

## Summary

在不改变 Go 业务语义和现有数据库 schema 的前提下，先建立共享的 InteractionFrame V2-only writer/reader、epoch/revision/FrameApplied 和证据基座，再按恢复、发送、消息操作、查询/阅读、频道治理五个纵切批次迁移 25 Gate。所有业务完成必须经过权威事实与 PersistOk；Angular 只消费完整 V2 VM。首轮 Native 全链必须证明 V1 producer/consumer/raw fallback 零命中，随后才物理删除 V1，并用全新 candidate/evidence epoch 再跑一次。

## Technical Context

**Language/Version**: Rust workspace（当前工具链与仓库一致）、TypeScript/Angular 20、Tauri 2、Go 服务端现役版本

**Primary Dependencies**: `helix-core` ports/effects、`helix-im` state machine/SurfaceComposer、`helix-driver-host/native`、cses-client SurfaceRuntime/Jest/Native E2E、现役 cses-im-server HTTP/WS

**Storage**: Helix host SQLite（业务事实、cursor、pending、receipt 等）；远端 PostgreSQL（Go 权威表）；Frame Cache 为可丢弃客户端缓存

**Testing**: Rust replay/contract/integration tests、Angular codec/runtime/component Jest、TypeScript checks、Native dual-profile scenario、PG/SQLite bounded readonly evidence、截图与结构化日志

**Target Platform**: PC/Tauri 为最终 E2E 验收面；V2 合同不得破坏 FFI/Web runtime-agnostic 边界

**Project Type**: 三仓纵切：Helix Rust 业务内核 + cses-client Angular/Tauri shell + 现役 Go authority

**Performance Goals**: Frame apply 为 `O(m)`；slot 查找平均 `O(1)`；实体 lookup 为 `O(1)`/`O(log n)`；时间线 compose 为 `O(visible window)`；禁止逐 Surface 全表扫描和 raw JSON 二次复制

**Constraints**: core 零 runtime/DB/UI 依赖；PersistOk 前零 durable success Frame；一个 Native candidate 连续跑 25 Gate；不直接写 PG/SQLite；同一失败签名最多自动修复两次

**Scale/Scope**: 19 业务 Spec、25 机器 Gate、9 个 Surface、双用户、首轮和删除后两轮完整 E2E

## Constitution Check

*GATE: Phase 0 前及 Phase 1 后均必须通过。*

| 原则 | 设计判定 |
|---|---|
| Runtime-Agnostic Layering | PASS：共享协议/业务变更仅在 `helix-im`，平台传输与 DOM 留在 driver/client |
| Deterministic Facts/Effects/Persistence | PASS：所有 Gate 以 Tick/Effect/PortReply/PersistOk 编排，不以 HTTP 2xx 建立业务成功 |
| InteractionFrame V2 Only | PASS：先切断生产 V1 produce/consume，再允许运行首轮 E2E |
| Evidence Before Completion | PASS：PG、SQLite、Go、Frame、FrameApplied、DOM、截图分面且同 causation |
| Minimal Governed Migration | PASS：不改 Go/DB 语义；共享基座一次迁移，Gate 只增加自己的 composer/VM/组件证据 |
| Spec Kit Loop Admission | PASS：R1 已通过；plan/tasks 分别接受 R2/R3；实现前必须 `IMPLEMENTATION_ADMITTED` |
| 热路径与安全 | PASS：批处理、bounded window、无日志正文/凭据、外部输入 fail-closed |

**Post-design re-check**: PASS。`data-model.md` 没有新增业务数据库迁移；`contracts/` 将 Frame 与证据清单分离；`quickstart.md` 只读验证且不允许手工修库。

## Loop Detail Contract

### Inputs and Context

- **I-001**: 当前 Helix、cses-client 与 Go source pin/工作树；实现前重新冻结并记录 revision、四包 pin、lock、matrix hash 和 fixture hash。
- **I-002**: [strict-matrix.md](strict-matrix.md) 的 25 Gate 输入、Effect、Persist、Surface、组件与证据合同。
- **I-003**: [per-gate-spec-index.md](per-gate-spec-index.md) 的 19 个 Gate Spec；每个 Gate 的 `06`-`10` 文件提供组件、wire、PG/SQLite 与 V1 inventory 细节。
- **I-004**: [discovery.md](discovery.md) 的当前 V1 producer/consumer、surfaceEpoch、FrameApplied 和工具缺口。
- **I-005**: cses-client 当前 Native scripts 与真实双用户认证环境；数据库只允许有界只读查询。

### Action Chain

- **A-001**: 冻结 source/contract/fixture pin → 建立 V2-only shared baseline → focused negative tests 证明 V1 fail-closed。
- **A-002**: 为一个 Gate 写失败测试 → 迁移 Helix Tick/Effect/Persist/SurfaceComposer → 迁移 Angular typed VM/组件 → focused evidence 达到 READY_FOR_FINAL_E2E。
- **A-003**: 19 个 Gate 全部 ready → 构建单一 Native candidate → 连续运行 25 Gate → 由场景原生断言验证双用户、Go、PG、SQLite、Frame、DOM 与截图；facet collector 继续产出诊断但不作为本轮阻断器。
- **A-004**: 首轮 required fail/skip=0 且 V1/raw=0 → 审批 deletion inventory → 物理删除 V1 → 生成新 candidate/evidence epoch → 二次完整回归。
- **A-005**: 场景原生断言失败或 pin 漂移 → 判 BLOCKED/FAIL → 只修第一处偏离，不恢复 V1/dual-write/raw fallback；严格 facet 缺面单独记为 diagnostic，不冒充 Green，也不覆盖场景结果。

### State Contract

- **S-001**: `Draft` → `SharedV2Ready`：V2 writer/reader、epoch、ack、fixtures 和 counters focused PASS。
- **S-002**: `SharedV2Ready` → `GateReady`：单 Gate focused Rust/Angular/contract 通过并登记最终证据查询。
- **S-003**: `GateReady(19/19)` → `V2OnlyCandidate`：Helix 与 client pin/hash 一致，生产 V1 路径不可达。
- **S-004**: `V2OnlyCandidate` → `FirstE2EPass`：25/25 场景原生断言与 V2 counters 全部通过；严格 facet reducer 仅输出诊断。
- **S-005**: `FirstE2EPass` → `V1Deleted`：deletion inventory 100% 执行且静态/合同检查无可达 V1。
- **S-006**: `V1Deleted` → `Closed`：新 candidate 与新 evidence epoch 的删除后二次 E2E 25/25 PASS。
- **S-007**: 任一状态 → `Blocked/NeedsHuman`：不可逆迁移、权限/业务语义变化或同签名第三次失败。

### Validators

- **V-001**: `cargo test -p helix-im --test interaction_frame_contract_test --no-default-features` 证明 V2 wire、epoch/revision、原子 apply 输入与 V1 fail-closed 合同。
- **V-002**: `cargo test -p helix-im` 证明 25 Gate replay、Persist barrier 和确定性回归。
- **V-003**: `npm run message:command-api:gate && npm run message:projection-ownership && npm run message:surface-runtime:source-pin` 证明命令/ownership/pin。
- **V-004**: focused Jest/TypeScript/SCSS-CSS gate 证明 codec/runtime/component、样式编译和布局合同。
- **V-005**: `npm run message:desktop:scenario:all` 的显式 25-Gate V2 profile 生成同一 candidate 的双用户运行证据。
- **V-006**: 场景汇总证明 required fail=0、skip=0、`v1Frames=0`、`rawBusinessFallbacks=0`；`message:real-chain:collect/generate-matrix/reduce` 保留为非阻断诊断，缺面不得伪报为全 facet Green。
- **V-007**: 有界 PG/SQLite readonly delta 对账证明 authority 与本地 PersistOk，不允许手工写库。
- **V-008**: 删除后静态扫描、fixture parity、Rust/Angular/full Native rerun 证明无 V1 回归且新 evidence epoch 独立。

## Architecture And Dependency Direction

```text
Angular Component
  -> opaque ActionBinding / HostObservation
  -> Tauri transport adapter
  -> helix-im Tick
  -> Effect (HTTP / Storage / Timer)
  -> Go authority / SQLite driver
  -> PortReply / PersistOk
  -> SurfaceComposer
  -> InteractionFrameV2
  -> SurfaceRuntime whole-frame apply
  -> FrameApplied
  -> DOM / ARIA / screenshot
```

- domain 不依赖 Angular、Tauri、Go、SQLite 或 PG。
- application 只编排 Tick/Effect/PortReply/SurfaceChangeSet。
- infrastructure 实现 HTTP/WS/Storage ports，不解释 Surface 业务。
- interface 只做 schema 验证、slot transaction、directive 与组件渲染。

## Delivery Phases

### Phase 0 - Source Lock And Research

冻结三仓 revision、matrix/fixture hash、V1 inventory 和工具能力。产物为 `research.md`、`data-model.md`、`contracts/` 与 `quickstart.md`。

### Phase 1 - Shared V2-only Foundation

1. Helix 唯一 V2 builder/emitter，禁止默认 V1。
2. Angular V2-only codec/types，删除生产 legacy normalize 可达性。
3. SurfaceRuntime 保存/校验 surfaceEpoch，整帧原子提交并生产回传 FrameApplied。
4. fixtures/matrix 统一 schemaVersion=2。
5. E2E collector/reducer 增加 V1/V2/raw/ack counters、显式 25-Gate profile。
6. 新增 SCSS/CSS/444/678 布局 gate。

### Phase 2 - Recovery Gate Plans

| Spec | 实现计划 | focused done signal |
|---|---|---|
| GS01 | hello/increment → channel/member/cursor PersistAtomic → dialog/unread V2 snapshot | increment end 后 matching PersistOk/FrameApplied |
| GS02 | sync notify → contiguous batch → message+cursor atomic → affected surfaces | full/mixed/no_change 有界终止 |
| GS03 | pong gaps/hash mismatch → only-gap queue → bounded compensation | 非 gap 零请求、无自激 |
| GS04 | unknown channel observation → hydrate → summary/composer/timeline | PersistOk 前 composer 不可发送 |

### Phase 3 - Send Gate Plans

| Spec | 实现计划 | focused done signal |
|---|---|---|
| G01 | message.create binding → pending persist → HTTP accept → WS canonical settle | temporaryId 只收敛为一行 |
| G02 | typed document input → pending/canonical flow → typed document VM | 无 raw props reducer |

### Phase 4 - Message Operation Gate Plans

| Spec | 实现计划 | focused done signal |
|---|---|---|
| G04/G04a | online revoke 与 type-3 replay 归一到相同撤回事实 | online 1 HTTP、offline 0 HTTP |
| G05 | per-target forward correlation/persist/frame | 部分失败逐目标准确 |
| G06 | reaction authority aggregate → timeline/thread V2 | 并发/重复不错误累加 |
| G07 | urgent/confirm 分离 binding → receipt projection | viewer 权限与人数正确 |
| G08 | owner-only schedule create/hydrate | observer 零草稿泄漏 |
| G09 | owner-only cancel terminal | 重复取消零新增 mutation |

### Phase 5 - Query, Read And Channel Gate Plans

| Spec | 实现计划 | focused done signal |
|---|---|---|
| G10/G10a | local-first bounded window + conditional backfill | Complete 零 HTTP、滚动锚点保持 |
| G11 | locate context commit → post-commit reveal directive | 失败不移动当前视图 |
| G12 | root/reply/branch graph persist → thread/timeline V2 | 三锚一致、无环无重复 |
| G13 | channel read boundary → unread surfaces atomic | boundary 单调、viewer 隔离 |
| G14/G14a | post read 与 template received 分离语义 | receipt 与 received 不混写 |
| G15/G15a | topic/channel creation → entity bundle → first surfaces | 新 channel/topic 唯一且 viewer-correct |
| G16/G16b/G16c | close/leave/remove/settings 分离终局 | close/read-only、self detach、target-only detach |

### Phase 6 - First V2-only E2E

在同一 immutable pin 和单 Native candidate 中连续跑 25 Gate。每 Gate 的原生断言执行 action/observation、Go、PG、SQLite、Frame、FrameApplied、DOM 与截图检查，聚合 required fail/skip=0 与 V1/raw=0；严格 facet 缺面另记诊断。

### Phase 7 - V1 Physical Deletion

按 inventory 删除 Helix V1 writer/builder、Angular V1 union/codec/normalize、RenderNode/v1 旧合同、V1 fixtures/tests。独立版本 API 的 `v1` 不按名称误删。

### Phase 8 - Post-deletion Regression

构建新 candidate，使用新 runId/artifact root/evidence epoch 重跑 contract、Rust、Angular、SCSS/CSS 和完整 25 Gate Native E2E。

## Project Structure

### Documentation

```text
specs/018-interaction-frame-v2-cutover/
├── spec.md
├── discovery.md
├── research.md
├── plan.md
├── data-model.md
├── quickstart.md
├── strict-matrix.md
├── per-gate-spec-index.md
├── contracts/
│   ├── interaction-frame-v2.md
│   └── evidence-manifest-v2.md
├── verification.md
├── review-ledger.json
├── evidence/
└── tasks.md
```

### Source Code

```text
crates/helix-im/
├── src/{interaction_frame.rs,surface_composer.rs,surface_runtime.rs,dialog_frame.rs}
├── src/{module,projector,sync,send,query,channel}/
├── contracts/strict-matrix.v1.json
└── tests/

/System/Volumes/Data/workspace/angular/cses-client/
├── src/pages/message-v4/surface-runtime/
├── src/pages/message-v4/components/
├── src/pages/message-v4/real-chain/
├── src-tauri/src/features/helix_im/
├── scripts/message-real-chain/
└── tests/e2e/desktop/
```

**Structure Decision**: 不新增运行时工程或通用框架。共享合同在现有 `interaction_frame/surface_runtime`，Gate 逻辑留在既有高内聚模块，证据能力扩展现有 Native harness。

## Complexity Tracking

无 Constitution 例外。19 个 Gate 的重复任务通过共享 Phase 1 与统一 evidence manifest 收敛，不复制 Frame Cache 或数据库访问实现。
