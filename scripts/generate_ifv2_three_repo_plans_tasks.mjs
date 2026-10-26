#!/usr/bin/env node

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';

const ROOT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
const MASTER = 'specs/020-ifv2-three-repo-uc-control';
const MANIFEST = `${MASTER}/gate-spec-manifest.json`;
const MATRIX = 'crates/helix-im/contracts/strict-matrix.v1.json';
const DOCS = 'docs/interaction-frame-v2-全量替换-按UC';
const CREATED = '2026-07-25';
const HELIX_HEAD = '918999d649007d92b085d8f5aba73329c9241ee4';
const PC_HEAD = '13e42ef2793ea5c9f39b29f5d1ce86b1aca074db';
const MOBILE_HEAD = '674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21';

const GATE_SOURCE_OWNER = new Map([
  ['G-02', 'crates/helix-im/src/send/flow.rs'],
  ['G-03a', 'crates/helix-im/src/send/retry_send.rs'],
  ['G-04', 'crates/helix-im/src/module/action.rs'],
  ['G-04a', 'crates/helix-im/src/sync/increment.rs'],
  ['G-05', 'crates/helix-im/src/forward.rs'],
  ['G-06', 'crates/helix-im/src/outbound/engagement/quick_reply.rs'],
  ['G-07', 'crates/helix-im/src/outbound/engagement/urgent.rs'],
  ['G-08', 'crates/helix-im/src/outbound/posts/existing.rs'],
  ['G-09', 'crates/helix-im/src/outbound/posts/existing.rs'],
  ['G-10', 'crates/helix-im/src/query/local_first.rs'],
  ['G-10a', 'crates/helix-im/src/channel/older_context.rs'],
  ['G-11', 'crates/helix-im/src/query/dispatch.rs'],
  ['G-12', 'crates/helix-im/src/query/read_relay.rs'],
  ['G-13', 'crates/helix-im/src/outbound/channel_admin/read.rs'],
  ['G-14', 'crates/helix-im/src/outbound/posts/read.rs'],
  ['G-14a', 'crates/helix-im/src/outbound/engagement/template_received.rs'],
  ['G-15a', 'crates/helix-im/src/outbound/channel_admin/create.rs'],
  ['G-15', 'crates/helix-im/src/channel/gate/flow.rs'],
  ['G-16', 'crates/helix-im/src/outbound/channel_admin/change_dedicated.rs'],
  ['G-16b', 'crates/helix-im/src/outbound/channel_admin/member_admin.rs'],
  ['G-16c', 'crates/helix-im/src/outbound/channel_admin/change.rs'],
  ['G-S1', 'crates/helix-im/src/sync/increment.rs'],
  ['G-S2', 'crates/helix-im/src/sync/explicit.rs'],
  ['G-S3', 'crates/helix-im/src/send/pong_compensate.rs'],
  ['G-S4', 'crates/helix-im/src/increment_hydration.rs'],
]);

const HELIX_EXISTING_TEST = new Map([
  ['G-02', 'crates/helix-im/tests/interaction_frame_contract_test.rs'],
  ['G-03a', 'crates/helix-im/tests/send_test.rs'],
  ['G-04', 'crates/helix-im/tests/g04_revoke_interaction_test.rs'],
  ['G-04a', 'crates/helix-im/tests/g04_revoke_interaction_test.rs'],
  ['G-05', 'crates/helix-im/tests/g05_forward_interaction_test.rs'],
  ['G-06', 'crates/helix-im/tests/g06_reaction_interaction_test.rs'],
  ['G-07', 'crates/helix-im/tests/g07_urgent_interaction_test.rs'],
  ['G-08', 'crates/helix-im/tests/g08_schedule_create_interaction_test.rs'],
  ['G-09', 'crates/helix-im/tests/g09_schedule_cancel_interaction_test.rs'],
  ['G-10', 'crates/helix-im/tests/g10_timeline_window_interaction_test.rs'],
  ['G-10a', 'crates/helix-im/tests/g10_timeline_window_interaction_test.rs'],
  ['G-11', 'crates/helix-im/tests/g11_locate_interaction_test.rs'],
  ['G-12', 'crates/helix-im/tests/g12_reply_thread_interaction_test.rs'],
  ['G-13', 'crates/helix-im/tests/g13_channel_read_interaction_test.rs'],
  ['G-14', 'crates/helix-im/tests/g14_receipt_template_interaction_test.rs'],
  ['G-14a', 'crates/helix-im/tests/g14_receipt_template_interaction_test.rs'],
  ['G-15a', 'crates/helix-im/tests/channel_create_http_reconcile_test.rs'],
  ['G-15', 'crates/helix-im/tests/g15_topic_interaction_test.rs'],
  ['G-16', 'crates/helix-im/tests/g16_governance_interaction_test.rs'],
  ['G-16b', 'crates/helix-im/tests/g16_governance_interaction_test.rs'],
  ['G-16c', 'crates/helix-im/tests/g16_governance_interaction_test.rs'],
  ['G-S1', 'crates/helix-im/tests/gs_persist_barrier_test.rs'],
  ['G-S2', 'crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs'],
  ['G-S3', 'crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs'],
  ['G-S4', 'crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs'],
]);

const MOBILE_UC_TEST = new Map([
  ['G-02', 'tests/gtest/uc/Uc02DocumentSendTest.cpp'],
  ['G-03a', 'tests/gtest/uc/Uc04RetrySendTest.cpp'],
  ['G-04', 'tests/gtest/uc/Uc05RevokeTest.cpp'],
  ['G-04a', 'tests/gtest/uc/Uc05RevokeTest.cpp'],
  ['G-05', 'tests/gtest/uc/Uc06RelayTest.cpp'],
  ['G-06', 'tests/gtest/uc/Uc07QuickReplyTest.cpp'],
  ['G-07', 'tests/gtest/uc/Uc08UrgentTest.cpp'],
  ['G-08', 'tests/gtest/uc/Uc09ScheduleTest.cpp'],
  ['G-09', 'tests/gtest/uc/Uc09ScheduleTest.cpp'],
  ['G-10', 'tests/gtest/uc/Uc10HistoryTest.cpp'],
  ['G-10a', 'tests/gtest/uc/Uc10HistoryTest.cpp'],
  ['G-11', 'tests/gtest/uc/Uc11LocateTest.cpp'],
  ['G-12', 'tests/gtest/uc/Uc12RepliesTest.cpp'],
  ['G-13', 'tests/gtest/uc/Uc13ReadTest.cpp'],
  ['G-14', 'tests/gtest/uc/Uc13ReadTest.cpp'],
  ['G-14a', 'tests/gtest/uc/Uc14TemplateReceivedTest.cpp'],
  ['G-15a', 'tests/gtest/uc/Uc15ChannelCreateTest.cpp'],
  ['G-15', 'tests/gtest/uc/Uc15ChannelCreateTest.cpp'],
  ['G-16', 'tests/gtest/uc/Uc16ChannelLifecycleTest.cpp'],
  ['G-16b', 'tests/gtest/uc/Uc16ChannelLifecycleTest.cpp'],
  ['G-16c', 'tests/gtest/uc/Uc16ChannelLifecycleTest.cpp'],
  ['G-S1', 'tests/gtest/uc/GsSyncPrerequisiteTest.cpp'],
  ['G-S2', 'tests/gtest/uc/GsSyncPrerequisiteTest.cpp'],
  ['G-S3', 'tests/gtest/uc/GsSyncPrerequisiteTest.cpp'],
  ['G-S4', 'tests/gtest/uc/GsSyncPrerequisiteTest.cpp'],
]);

// 读取 JSON 真源并保留解析错误，使生成过程 fail-closed。
function readJson(path) {
  return JSON.parse(readFileSync(join(ROOT, path), 'utf8'));
}

// 创建父目录并以稳定结尾换行写入生成物。
function write(path, content) {
  const absolute = join(ROOT, path);
  mkdirSync(dirname(absolute), { recursive: true });
  writeFileSync(absolute, `${content.trimEnd()}\n`);
}

// 将 Gate ID 转为 Rust、WDIO 与 fixture 共用的稳定小写标识。
function gateSlug(gateId) {
  return gateId.toLowerCase().replaceAll('-', '').replaceAll(/[^a-z0-9]/g, '');
}

// 将数组去重后渲染为可读合同文本。
function list(values) {
  const unique = [...new Set(values.filter(Boolean))];
  return unique.length ? unique.join('、') : '无';
}

// 返回用户确认的 Gate 特例，防止旧 Matrix 漂移污染实现计划。
function gateClarification(gateId) {
  if (gateId !== 'G-02') return '';
  return 'G02 固定发送 `http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15` 并渲染文档消息卡片；不是附件、文件上传或上传进度状态机，Strict Matrix 的 `upload` effect 必须在实现前纠偏。';
}

// 返回当前 Gate 的三仓生产与测试路径集合。
function gatePaths(record) {
  const slug = gateSlug(record.gateId);
  return {
    slug,
    helixOwner: GATE_SOURCE_OWNER.get(record.gateId),
    helixExistingTest: HELIX_EXISTING_TEST.get(record.gateId),
    helixFlowTest: `crates/helix-im/tests/${slug}_full_flow_test.rs`,
    helixFixtureTest: `crates/helix-im/tests/${slug}_wdio_fixture_contract_test.rs`,
    helixSupport: `crates/helix-im/tests/support/${slug}_scenario.rs`,
    helixFixtures: `crates/helix-im/tests/fixtures/${slug}-wdio/`,
    pcRoot: '/System/Volumes/Data/workspace/angular/cses-client',
    pcFixtures: `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/fixtures/${slug}/`,
    pcPage: `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/pageobjects/${slug}.page.ts`,
    pcMock: `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/${slug}-mock.e2e.ts`,
    pcReal: `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/${slug}-single-real.e2e.ts`,
    pcDual: `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs/${slug}-dual-real.e2e.ts`,
    pcReport: `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/reporters/${slug}-summary.ts`,
    mobileRoot: '/System/Volumes/Data/workspace/c/mobile-qucik-c++',
    mobileUcTest: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/${MOBILE_UC_TEST.get(record.gateId)}`,
    mobileFrameTest: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/${record.gateId.replaceAll('-', '').toUpperCase()}FrameContractTest.cpp`,
    mobileFixtures: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/fixtures/${slug}/`,
  };
}

// 渲染单 Gate 的 Phase 0 决策，区分当前事实、计划目标和禁止替代项。
function renderResearch(record, row, paths) {
  const commands = row.commands.map((command) => command.name);
  const effects = row.commands.flatMap((command) => command.effect ?? []);
  const surfaces = row.projections.map((projection) => projection.render_kind);
  return `# Research: ${record.gateId} ${record.title}

**Source Lock**: Helix \`${HELIX_HEAD}\`；cses-client \`${PC_HEAD}\`；Mobile \`${MOBILE_HEAD}\`

## Decision 1 - 唯一业务真源

**Decision**: \`${record.gateId}\` 的 command（${list(commands)}）、effect（${list(effects)}）、持久事实（${list(row.persistence ?? [])}）与 Surface（${list(surfaces)}）只由 Helix 生产路径决定。

**Rationale**: PC 与 Mobile 自行补字段、排序、权限或 reducer 会形成第二套业务逻辑。

**Alternatives considered**: WDIO 手写 ViewModel、GTest 内补 row、Angular/Mobile 从 raw event 重建；全部拒绝。

## Decision 2 - Canonical Fixture

**Decision**: Fixture 只允许由 \`ImModule::handle -> Effect::Emit -> InteractionFrameV2 decode\` 生成，普通测试比较 Golden，只有显式 \`UPDATE_${paths.slug.toUpperCase()}_WDIO_FIXTURES=1\` 才更新。

**Rationale**: Fixture 是 Helix producer 与两个平台 consumer 的跨仓合同，不是 command 同步返回值。

**Alternatives considered**: 从 DOM 反推 JSON、复制其他 Gate Fixture、前端补动态业务字段；全部拒绝。

## Decision 3 - PC 验收分层

**Decision**: PC 分开执行 Fixture parity、Mock replay、single-real、按用户可见影响决定的 dual-real、DOM/FrameApplied 断言和结构化报告。

**Rationale**: Mock 不经过 Rust/Tauri/SQLite/HTTP/WS，不能替代真实链。

**Alternatives considered**: 只跑 Mock、只证明 invoke accepted、固定 sleep 等第三帧；全部拒绝。

## Decision 4 - Mobile 生产接线

**Decision**: Mobile GTest 链接生产 \`MobileImSdk/MobileFfiBridge/SurfaceRuntime/QuickJS binding\`，完整 Frame 经 DomainEvent batch 原子 apply，JS 只通过 \`im.surface.view\` 获取 typed view。

**Rationale**: Mobile 不允许 \`im.store\`、raw reducer 或测试注入 API进入默认生产构建。

**Alternatives considered**: JS 直接消费 raw event、GTest 注入私有 reducer、每 mutation 一次 FFI callback；全部拒绝。

## Decision 5 - Gate-specific authority

**Decision**: HTTP 为 \`${row.http?.request ?? '无独立 HTTP'}\`；WS 为 \`${row.ws?.events ?? '无独立 WS'}\`；远端表为 \`${list(row.remote_store?.tables ?? [])}\`。Transport success 不自动等于 durable terminal。

**Rationale**: terminal authority、persist barrier 与竞态必须从当前生产 handler 和 PortReply/Inbound 证明。

**Alternatives considered**: HTTP 200 即 Green、远端行存在即本地已 apply、固定 HTTP/WS 顺序；全部拒绝。

## Resolved Scope

- 当前 Helix owner：\`${paths.helixOwner}\`；现有聚焦测试：\`${paths.helixExistingTest}\`。
- 当前 Mobile UC 测试：\`${paths.mobileUcTest}\`。
${gateClarification(record.gateId) ? `- Gate 特例：${gateClarification(record.gateId)}` : ''}
- G03 文件上传和 G17-G27 不进入本 Feature。
- 当前计划无未解决 \`NEEDS CLARIFICATION\`；代码漂移在 T001 source revalidation 中 fail-closed 处理。
`;
}

// 渲染跨仓验收实体和状态转换，供任务与报告共享语言。
function renderDataModel(record, row) {
  return `# Data Model: ${record.gateId} ${record.title}

## GateScenario

| Field | Type | Rule |
|---|---|---|
| gateId | string | 固定为 \`${record.gateId}\` |
| actorId / observerId | string | canonical real lane 使用隔离双身份 |
| channelId | string | 每次 canonical run 由 CreateGroupPrerequisite 新建，不复用前一 Gate |
| command | tagged object | 仅允许 ${list(row.commands.map((command) => command.name))} |
| correlation | object | reqId、transactionId、causationId 与业务 ID 可跨层关联 |
| expectedTerminal | enum | success、business-failed、infrastructure-failed、recovered |

## CanonicalFrameArtifact

| Field | Type | Rule |
|---|---|---|
| fixtureId | string | Gate 内唯一并表达独立可观察阶段 |
| schemaVersion | integer | 必须为 InteractionFrame V2 |
| surfaces | array | 仅包含 ${list(row.projections.map((projection) => projection.render_kind))} |
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

## State Transitions

\`Draft -> HelixGreen -> PcGreen -> MobileGreen -> ReviewGreen\`

- 任一 hash/pin/source 漂移：回到 \`Draft\`。
- 任一 required lane 缺失：进入 \`Blocked\`，不得解锁下一 Gate。
- Mock、AcceptedCommand 或 HTTP 2xx 不能直接进入 \`PcGreen\` 或 \`ReviewGreen\`。
`;
}

// 渲染单 Gate 的完整实现计划，保持 domain 到平台 adapter 的单向依赖。
function renderPlan(record, row, paths) {
  const commands = row.commands.map((command) => command.name);
  const effects = row.commands.flatMap((command) => command.effect ?? []);
  const surfaces = row.projections.map((projection) => projection.render_kind);
  return `# Implementation Plan: ${record.gateId} ${record.title} 三仓纵向闭环

**Branch**: \`${record.featureDirectory.split('/').at(-1)}\` | **Date**: ${CREATED} | **Spec**: [spec.md](spec.md)

**Input**: [research.md](research.md)、[data-model.md](data-model.md)、[contracts/gate-acceptance.md](contracts/gate-acceptance.md) 与 Strict Matrix 中 \`${record.gateId}\` 的当前合同。

## Summary

以 Helix 生产 handler \`${paths.helixOwner}\` 为业务真源，补齐 \`${record.gateId}\` 的完整 sans-IO 场景、字段级 InteractionFrame V2、canonical Fixture/Golden/provenance；PC 只读同步 Fixture 后分别完成 WDIO Mock、真实 Tauri/Helix 链、DOM 与 report；Mobile byte-exact 同步同一 Fixture，通过生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest 验证 typed view。三仓完成后执行行为、代码边界与 Frame V2 Review，只有 review-green 才解锁 \`${record.prerequisiteGate === 'G-01' ? '后续 Gate' : '下一 Gate'}\`。

## Technical Context

**Language/Version**: Rust workspace 当前工具链；TypeScript 5.6/Angular 20.3/Node 20；C++17/QuickJS/GoogleTest

**Primary Dependencies**: helix-core ports、helix-im \`ImModule::handle\`/InteractionFrameV2；cses-client TauriMockBridge/SurfaceRuntime/WebdriverIO；MobileImSdk/MobileFfiBridge/SurfaceRuntime/HelixImQuickJsBinding

**Storage**: Helix SQLite StorageOp（${list(row.persistence ?? [])}）；远端 PostgreSQL \`${list(row.remote_store?.tables ?? [])}\` 只作有界证据

**Testing**: Rust deterministic replay + Golden；WDIO Mock/single-real/dual-real + reporter；C++ GTest + QuickJS typed view

**Target Platform**: Helix native/FFI 共用业务核；PC Tauri WKWebView；Mobile C++/QuickJS

**Project Type**: 三仓纵向 UC 交付，Helix domain/application，PC/Mobile platform adapter

**Performance Goals**: core 每 Tick 有界；Frame apply 为 O(k mutations)；fixture parity 为 O(n files)；等待使用 correlation latch，不固定 sleep

**Constraints**: Helix 是唯一业务 reducer；Fixture/Mock/GTest 不补业务字段；测试入口默认 production build 不可达；外部输入零 unwrap/expect；FFI batch 不逐条 callback

**Scale/Scope**: 单一 \`${record.gateId}\`；command=${list(commands)}；effect=${list(effects)}；surface=${list(surfaces)}。${gateClarification(record.gateId)}

## Constitution Check

| 原则 | 设计判定 |
|---|---|
| Runtime-Agnostic Layering | PASS：业务改动只在 helix-im，PC/Mobile 只实现 adapter 与 renderer |
| Deterministic Facts | PASS：时间、ID、PortReply、Inbound 全由 ScenarioRunner 注入 |
| InteractionFrame V2 Truth | PASS：PC/Mobile 不消费 raw business event 重建状态 |
| Evidence Before Completion | PASS：Helix、Fixture、Mock、Real、Mobile 与 Review 分层 |
| Minimal Governed Migration | PASS：只处理 ${record.gateId}，不带入 G03 或 G17-G27 |
| Spec Kit Admission | PASS：R1 已通过；R2/R3 后才允许实现 |
| 热路径与 FFI | PASS：Frame O(k) apply、batch crossing、无无界扫描 |

**Post-design re-check**: PASS。contracts 只冻结 Gate 验收与报告，不增加第二套业务协议；平台 mirror 只读；真实删除仍由 26 Gate 总控和用户决定。

## Loop Detail Contract

### Inputs and Context

- **I-001**: [discovery.md](discovery.md) 的三仓 source lock、当前工作树边界与代码事实。
- **I-002**: Strict Matrix 的 command/effect/persistence/projection/HTTP/WS/remote-store 合同。
- **I-003**: \`${paths.helixExistingTest}\` 与 \`${paths.helixOwner}\` 的当前生产行为。
- **I-004**: G01 canonical Fixture、PC WDIO Mock/Real/report 与 Mobile GTest/QuickJS 基线。
- **I-005**: [contracts/gate-acceptance.md](contracts/gate-acceptance.md)、两个 JSON Schema 与 [quickstart.md](quickstart.md)。

### Action Chain

- **A-001**: fresh group prerequisite -> ${list(commands)} command/observation -> \`ImModule::handle\` -> ordered Effects。
- **A-002**: Persist/HTTP/WS PortReply 或 Inbound -> terminal authority -> SurfaceComposer -> complete InteractionFrame V2。
- **A-003**: Effect::Emit -> typed decode/assert -> \`${paths.helixFixtures}\` Golden/provenance。
- **A-004**: Helix Fixture -> PC mirror/hash -> TauriMockBridge bus replay -> Angular SurfaceRuntime -> DOM/report。
- **A-005**: 真实 MessageV4 UI -> Tauri invoke -> Helix/SQLite/HTTP/WS -> FrameApplied -> Actor/Observer DOM。
- **A-006**: Helix Fixture -> Mobile mirror -> production SDK/FFI DomainEvent batch -> SurfaceRuntime atomic apply -> QuickJS \`im.surface.view\`。
- **A-007**: 三仓证据 -> behavior/code/Frame V2 review -> review-green 或 fail-closed blocker。

### State Contract

- **S-001**: \`Draft -> HelixGreen\`：完整场景、生产 Frame、Golden 与 provenance 全通过。
- **S-002**: \`HelixGreen -> PcGreen\`：Mock 与 required real lane、DOM、FrameApplied、report 全通过。
- **S-003**: \`PcGreen -> MobileGreen\`：mirror parity、生产链 GTest、QuickJS typed view 与 hard-cut 全通过。
- **S-004**: \`MobileGreen -> ReviewGreen\`：行为、代码边界、Frame V2 与 evidence Review 全 Green。
- **S-005**: 任一 pin/hash/schema/source 漂移 -> \`Draft\`；required 环境缺失 -> \`Blocked\`。

### Validators

- **V-001**: \`cargo check -p helix-im\` 与 \`cargo test -p helix-im --test ${paths.slug}_full_flow_test -- --nocapture\` 证明完整确定性流程。
- **V-002**: \`cargo test -p helix-im --test ${paths.slug}_wdio_fixture_contract_test -- --nocapture\` 证明完整 Frame、Golden 与 provenance。
- **V-003**: \`yarn message:wdio:${paths.slug}:mock\` 证明 PC mirror/Mock/DOM/report，不提升为 real。
- **V-004**: \`yarn message:wdio:${paths.slug}:single-real\` 与 required dual lane 证明真实 Tauri/Helix/WS/DOM。
- **V-005**: \`./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter='${record.gateId.replaceAll('-', '').toUpperCase()}FrameContractTest.*'\` 证明 Mobile production consumer 与 QuickJS typed view。
- **V-006**: production-boundary/source scans 证明 Fixture、Mock、测试 ABI、raw reducer 在默认构建不可达。
- **V-007**: cross-repo review 对齐 command、terminal、Frame、hash、DOM、typed view 与报告，输出 review-green 或 blocker。

## Project Structure

### Documentation

\`\`\`text
${record.featureDirectory}/
├── spec.md
├── discovery.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── verification.md
├── contracts/
├── review-ledger.json
└── tasks.md
\`\`\`

### Source Code

\`\`\`text
${paths.helixOwner}
${paths.helixExistingTest}
${paths.helixFlowTest}
${paths.helixFixtureTest}
${paths.helixSupport}
${paths.helixFixtures}

${paths.pcFixtures}
${paths.pcPage}
${paths.pcMock}
${paths.pcReal}
${paths.pcDual}
${paths.pcReport}

${paths.mobileUcTest}
${paths.mobileFrameTest}
${paths.mobileFixtures}
/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileFfiBridge.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp
/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/HelixImQuickJsBinding.cpp
\`\`\`

**Structure Decision**: 不建立平台业务 Store；Helix 测试与 Fixture 属 producer，PC/Mobile 文件只验证或机械 apply 同一 Frame。现有共享 harness 优先扩展，Gate 文件只承载业务特定场景和断言。

## Delivery Phases

1. Source revalidation 与 fresh-group prerequisite。
2. Helix RED 场景、生产 handler/persist/projector、Fixture/Golden。
3. PC mirror、Mock、single/dual real、DOM/report 与 production-boundary。
4. Mobile mirror、GTest、SDK/Bridge/SurfaceRuntime/QuickJS typed view 与 hard-cut。
5. 行为、代码、Frame V2、evidence Review 与 R4。

## Complexity Tracking

无 Constitution 例外。任何需要平台业务 reducer、producer dual-write、raw fallback 或测试注入进入生产的方案直接拒绝，不登记为可接受复杂度。
`;
}

// 渲染 Gate producer 到两端 consumer 的字段级验收合同。
function renderAcceptanceContract(record, row, paths) {
  return `# ${record.gateId} 三仓验收合同

## Producer

- 唯一 producer：\`${paths.helixOwner}\` 经 \`ImModule::handle -> Effect::Emit\`。
- Commands：${list(row.commands.map((command) => command.name))}。
- Persistence：${list(row.persistence ?? [])}。
- Surfaces：${list(row.projections.map((projection) => projection.render_kind))}。
- Golden：\`${paths.helixFixtures}\`，普通测试只比较，显式 update 才覆盖。

## PC Consumer

- Mock 只接管 command receipt；完整 Frame 从真实 \`im:__bus__\` replay。
- Real 必须经过 MessageV4 UI、Tauri invoke、Helix、必要 SQLite/HTTP/WS、FrameApplied 与 DOM。
- 报告必须分开 Mock/Real，记录 correlation、assertion、duration、retry、blocker 与 artifact hash。

## Mobile Consumer

- Fixture mirror 与 Helix byte-exact。
- GTest 链接生产 MobileImSdk、MobileFfiBridge、SurfaceRuntime 与 QuickJS binding。
- JS 只读 \`im.surface.view\` typed view；禁止 \`im.store\`、raw reducer 和测试业务字段。

## Green Formula

\`Helix Green + Fixture Green + PC Mock Green + PC Real Green + Mobile Contract Green + production-boundary Green + Review Green\`

任一 required fail/skip/blocker 均不得 Green。
`;
}

// 渲染 Fixture manifest 的最小机器合同，约束两个 mirror 不可增补业务字段。
function renderFixtureSchema(record, row) {
  return JSON.stringify({
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    title: `${record.gateId} canonical fixture manifest`,
    type: 'object',
    additionalProperties: false,
    required: ['gateId', 'schemaVersion', 'producer', 'fixtures', 'rawSha256', 'semanticSha256', 'surfaces'],
    properties: {
      gateId: { const: record.gateId },
      schemaVersion: { const: 2 },
      producer: { type: 'object' },
      fixtures: { type: 'array', minItems: 1, items: { type: 'string' } },
      rawSha256: { type: 'string', pattern: '^[a-f0-9]{64}$' },
      semanticSha256: { type: 'string', pattern: '^[a-f0-9]{64}$' },
      surfaces: {
        type: 'array',
        uniqueItems: true,
        items: { enum: [...new Set(row.projections.map((projection) => projection.render_kind))] },
      },
    },
  }, null, 2);
}

// 渲染跨仓结构化报告合同，保留分层声明和 blocker。
function renderReportSchema(record) {
  return JSON.stringify({
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    title: `${record.gateId} three-repo report`,
    type: 'object',
    additionalProperties: false,
    required: ['gateId', 'sourcePins', 'lanes', 'review', 'finalVerdict'],
    properties: {
      gateId: { const: record.gateId },
      sourcePins: { type: 'object' },
      lanes: {
        type: 'object',
        required: ['helix', 'fixture', 'pcMock', 'pcReal', 'mobileContract', 'productionBoundary'],
      },
      review: {
        type: 'object',
        required: ['behavior', 'codeBoundary', 'frameV2', 'evidence'],
      },
      finalVerdict: { enum: ['Green', 'Red', 'Blocked', 'Partial'] },
    },
  }, null, 2);
}

// 渲染可执行 quickstart，命令缺失时以实现 Task 创建而不是静默降级。
function renderQuickstart(record, paths) {
  return `# Quickstart: ${record.gateId} ${record.title}

## 1. Source Preflight

\`\`\`bash
cd /System/Volumes/Data/workspace/rust/helix
git status --short
git rev-parse HEAD
git -C /System/Volumes/Data/workspace/angular/cses-client rev-parse HEAD
git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ rev-parse HEAD
\`\`\`

确认三仓 pin、前序 \`${record.prerequisiteGate}\` review-green，以及本次 fresh group prerequisite。脏工作树只记录，不覆盖。

## 2. Helix

\`\`\`bash
cargo check -p helix-im
cargo test -p helix-im --test ${paths.slug}_full_flow_test -- --nocapture
cargo test -p helix-im --test ${paths.slug}_wdio_fixture_contract_test -- --nocapture
\`\`\`

期望：完整场景、Frame 字段、Golden 与 provenance 全通过；普通测试不修改 \`${paths.helixFixtures}\`。

## 3. PC

\`\`\`bash
cd /System/Volumes/Data/workspace/angular/cses-client
yarn message:wdio:${paths.slug}:mock
yarn message:wdio:${paths.slug}:single-real
yarn message:wdio:${paths.slug}:dual-real
\`\`\`

期望：Mock 与 Real 分层报告；无法启动真实环境时标记 Blocked，不用 Mock 替代。

## 4. Mobile

\`\`\`bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter='${record.gateId.replaceAll('-', '').toUpperCase()}FrameContractTest.*'
./scripts/run-uc01-16-gtest.sh
\`\`\`

期望：mirror parity、生产 SDK/Bridge/SurfaceRuntime/QuickJS typed view、V2-only hard-cut 全通过。仅当本 Gate 修改 Venus/QuickJS 接线能力时追加专属 Venus Gate，否则使用共享回归。

## 5. Review

汇总 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 与行为/代码/Frame V2 Review。required pass=100%、fail=0、skip=0 才可 review-green。
`;
}

// 创建严格格式任务对象，统一生成 checklist 与 Loop Task Contracts。
function task(id, phase, story, description, requirement, planItem, scope, preconditions, validator, done, parallel = false) {
  return { id, phase, story, description, requirement, planItem, scope, preconditions, validator, done, parallel };
}

// 返回单 Gate 覆盖 Helix、PC、Mobile 和 Review 的完整任务集合。
function childTasks(record, row, paths) {
  const mobileSdk = '/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp';
  const mobileBridge = '/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileFfiBridge.cpp';
  const mobileRuntime = '/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp';
  const mobileQuickJs = '/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/HelixImQuickJsBinding.cpp';
  const pcSync = '/System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/sync-helix-fixtures.mjs';
  const pcBridge = '/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/tauri-mock-bridge.ts';
  const pcPackage = '/System/Volumes/Data/workspace/angular/cses-client/package.json';
  const feature = record.featureDirectory;
  return [
    task('T001', 'Setup', null, `锁定三仓 HEAD、Helix pin、Strict Matrix hash 与前序 ${record.prerequisiteGate} review-green 到 ${feature}/verification.md`, 'FR-004,FR-018', 'I-001,I-002', `${feature}/verification.md`, 'R1/R2 pass', 'source/hash scan', 'source lock recorded'),
    task('T002', 'Setup', null, `按 ${paths.helixOwner}、${paths.helixExistingTest} 与 ${paths.mobileUcTest} 复核 CURRENT/TARGET/GAP 并更新 ${feature}/research.md`, 'FR-003,FR-015', 'I-002,I-003', `${feature}/research.md`, 'T001', 'path:line review', 'current code map current', true),
    task('T003', 'Foundational', null, `复用并补齐 fresh group CreateGroupPrerequisite 三仓合同于 crates/helix-im/tests/g01_full_flow/group_prerequisite.rs、crates/helix-im/tests/g01_shared/scenario.rs、/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/g01-group-prerequisite.ts、/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/uc/support/UcJsContractFixture.hpp`, 'FR-001', 'A-001', 'three-repo prerequisite support', 'T001', 'isolated group tests', 'new channel per run'),
    task('T004', 'Helix', 'US1', `先在 ${paths.helixFlowTest} 编写 prerequisites/happy/business failure/infrastructure failure/race/recovery RED tests`, 'FR-003,FR-005', 'A-001,A-002', paths.helixFlowTest, 'T002-T003', 'V-001 RED', 'missing production behavior falsified', true),
    task('T005', 'Helix', 'US1', `在 ${paths.helixSupport} 实现确定性 ScenarioRunner，注入 Tick/PortReply/Inbound/time/id 并捕获 ordered Effects`, 'FR-003,FR-005', 'A-001,A-002', paths.helixSupport, 'T004', 'V-001', 'reusable deterministic trace'),
    task('T006', 'Helix', 'US1', `在 ${paths.helixOwner} 对齐 ${list(row.commands.map((command) => command.name))} 的生产 handler、authority 与幂等语义，禁止测试旁路。${gateClarification(record.gateId)}`, 'FR-003,FR-004', 'A-001,A-002', paths.helixOwner, 'T004-T005', 'V-001', 'happy/failure/race pass'),
    task('T007', 'Helix', 'US1', `在 ${paths.helixExistingTest} 补 persist barrier、late reply、duplicate inbound 与 recovery 字段级断言`, 'FR-004,FR-005', 'A-002', paths.helixExistingTest, 'T006', 'focused cargo test', 'terminal authority proven'),
    task('T008', 'Helix', 'US1', `在 crates/helix-im/src/surface_composer.rs 与 crates/helix-im/src/projector/ 校验 ${list(row.projections.map((projection) => projection.render_kind))} 完整 mutation/action/directive，缺口回 Helix 修复`, 'FR-003,FR-013', 'A-002', 'crates/helix-im/src/surface_composer.rs', 'T006-T007', 'Frame field assertions', 'complete V2 frame'),
    task('T009', 'Helix', 'US1', `先在 ${paths.helixFixtureTest} 编写 Fixture inventory、typed decode、字段语义、raw/semantic hash 与 update-policy RED tests`, 'FR-006..FR-008', 'A-003', paths.helixFixtureTest, 'T008', 'V-002 RED', 'fixture drift detected', true),
    task('T010', 'Helix', 'US1', `由 ${paths.helixSupport} 的真实 Emit trace 生成 ${paths.helixFixtures} canonical JSON，不手写平台 VM`, 'FR-006,FR-007', 'A-003', paths.helixFixtures, 'T009', 'V-002', 'Golden byte set complete'),
    task('T011', 'Helix', 'US1', `在 ${paths.helixFixtures}provenance.json 记录 producer、source revision、schema、surface inventory、raw/semantic SHA-256 与允许动态字段`, 'FR-008', 'A-003', `${paths.helixFixtures}provenance.json`, 'T010', 'manifest schema/hash', 'provenance complete'),
    task('T012', 'Helix', 'US1', `运行 cargo check 与 ${paths.slug}_full_flow_test/${paths.slug}_wdio_fixture_contract_test，并登记原始输出到 ${feature}/verification.md`, 'SC-001,SC-002', 'V-001,V-002', `${feature}/verification.md`, 'T004-T011', 'commands exit 0', 'Helix Green'),
    task('T013', 'PC', 'US2', `扩展 ${pcSync} 的 ${record.gateId} manifest/filename/hash/surface 映射，只读同步到 ${paths.pcFixtures}`, 'FR-009,FR-013', 'A-004', pcSync, 'T011-T012', 'fixture parity test', 'PC mirror exact'),
    task('T014', 'PC', 'US2', `在 ${paths.pcFixtures}__tests__/fixture-parity.test.ts 编写 schema/inventory/raw/semantic hash 与禁止字段补写 RED tests`, 'FR-009,FR-013', 'A-004', `${paths.pcFixtures}__tests__/fixture-parity.test.ts`, 'T013', 'focused Node test', 'mirror drift fail-closed', true),
    task('T015', 'PC', 'US2', `扩展 /System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/fixtures/fixture-loader.ts 的 ${record.gateId} typed loading API，不解释业务字段`, 'FR-009,FR-013', 'A-004', '/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/fixtures/fixture-loader.ts', 'T014', 'loader contract test', 'typed fixture available'),
    task('T016', 'PC', 'US2', `在 /System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/harness/__tests__/tauri-mock-bridge.test.ts 先写 ${record.gateId} AcceptedCommand/replay/correlation RED tests，再扩展 ${pcBridge}`, 'FR-009,FR-010', 'A-004', pcBridge, 'T015', 'bridge test', 'receipt/frame split enforced'),
    task('T017', 'PC', 'US2', `在 ${paths.pcPage} 封装真实 MessageV4 操作、Surface ready、DOM 字段、FrameApplied 与自动等待，不引入业务计算`, 'FR-009,FR-013', 'A-004,A-005', paths.pcPage, 'T016', 'page contract test', 'stable observable API', true),
    task('T018', 'PC', 'US2', `在 ${paths.pcMock} 以 Helix Fixture replay 验证完整 happy/failure/race DOM、transactionId 与 claim=mock`, 'FR-009,FR-015', 'A-004', paths.pcMock, 'T016-T017', 'V-003', 'PC Mock Green'),
    task('T019', 'PC', 'US2', `在 ${paths.pcReal} 从真实 UI 创建 fresh group 并执行 ${record.gateId}，关联 invoke/Helix/SQLite/HTTP/WS/FrameApplied/DOM`, 'FR-009,FR-015', 'A-005', paths.pcReal, 'T017-T018', 'V-004', 'single-real evidence closed'),
    task('T020', 'PC', 'US2', `在 ${paths.pcDual} 验证 Actor/Observer 对 ${record.title} 的最终 timeline/dialog/receipt 可见结果；不适用字段以合同理由显式断言`, 'FR-009,FR-015', 'A-005', paths.pcDual, 'T019', 'dual real latch', 'observer evidence closed'),
    task('T021', 'PC', 'US2', `在 ${paths.pcReport} 与 ${pcPackage} 增加 ${paths.slug}:mock/single-real/dual-real/report 命令和分层 assertion/report 汇总`, 'FR-009,FR-018', 'A-004,A-005', paths.pcReport, 'T018-T020', 'report schema test', 'PC report Green'),
    task('T022', 'PC', 'US2', `扩展 /System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/check-production-boundary.mjs，证明 ${record.gateId} Fixture/Mock/WDIO API 不进入 production bundle`, 'FR-010,FR-014', 'V-006', '/System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/check-production-boundary.mjs', 'T013-T021', 'boundary scan', 'PC production boundary Green'),
    task('T023', 'Mobile', 'US3', `新增 /System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/sync-${paths.slug}-helix-fixtures.mjs 只读同步 ${paths.helixFixtures} 到 ${paths.mobileFixtures}`, 'FR-011,FR-013', 'A-006', `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/sync-${paths.slug}-helix-fixtures.mjs`, 'T011-T012', 'byte parity', 'Mobile mirror exact'),
    task('T024', 'Mobile', 'US3', `在 /System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/check-${paths.slug}-fixture-parity.mjs 编写 pin/schema/inventory/raw/semantic hash 与禁止补字段检查`, 'FR-011,FR-013', 'A-006', `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/check-${paths.slug}-fixture-parity.mjs`, 'T023', 'Node parity test', 'mirror fail-closed', true),
    task('T025', 'Mobile', 'US3', `先在 ${paths.mobileFrameTest} 编写完整 Frame envelope、mutation、action、directive、revision、failure/race 原子 apply RED tests`, 'FR-011,FR-013', 'A-006', paths.mobileFrameTest, 'T024', 'V-005 RED', 'runtime gaps falsified', true),
    task('T026', 'Mobile', 'US3', `在 ${paths.mobileUcTest} 对齐 ${record.gateId} command admission、fresh group prerequisite、DomainEvent batch 与 QuickJS typed view 行为`, 'FR-011,FR-013', 'A-006', paths.mobileUcTest, 'T025', 'focused GTest', 'UC behavior current'),
    task('T027', 'Mobile', 'US3', `在 ${mobileSdk} 接入 ${list(row.commands.map((command) => command.name))} 生产 command admission 与 Frame event pump，不注册测试 API`, 'FR-011,FR-012', 'A-006', mobileSdk, 'T025-T026', 'GTest + boundary scan', 'SDK production ingress'),
    task('T028', 'Mobile', 'US3', `在 ${mobileBridge} 保持 helix-driver-ffi DomainEvent batch 边界与错误分类，禁止逐 mutation callback`, 'FR-011,FR-012', 'A-006', mobileBridge, 'T027', 'FFI lifecycle GTest', 'batch bridge Green'),
    task('T029', 'Mobile', 'US3', `在 ${mobileRuntime} 原子校验并 apply ${list(row.projections.map((projection) => projection.render_kind))} Frame，unknown schema/surface/revision fail-closed`, 'FR-011..FR-013', 'A-006', mobileRuntime, 'T028', 'Frame contract GTest', 'atomic typed state'),
    task('T030', 'Mobile', 'US3', `在 ${mobileQuickJs} 与 /System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/api/ImSurfaceApi.cpp 只暴露 ${record.gateId} 的 im.surface.view typed view/action，不增加 raw reducer`, 'FR-011..FR-013', 'A-006', mobileQuickJs, 'T029', 'QuickJS GTest', 'typed view Green'),
    task('T031', 'Mobile', 'US3', `扩展 /System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/check-surface-runtime-legacy-hard-cut.mjs 与 check-surface-runtime-source-pin.mjs，断言 ${record.gateId} V1 producer/consumer/raw fallback=0`, 'FR-012,FR-017', 'V-006', '/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/check-surface-runtime-legacy-hard-cut.mjs', 'T027-T030', 'source/binary scan', 'Mobile hard-cut Green'),
    task('T032', 'Mobile', 'US3', `运行 ${record.gateId.replaceAll('-', '').toUpperCase()}FrameContractTest、${paths.mobileUcTest} 所属 filter 与共享 run-uc01-16-gtest.sh，并登记 ${feature}/verification.md`, 'SC-003,SC-004', 'V-005,V-006', `${feature}/verification.md`, 'T023-T031', 'GTest exit 0', 'Mobile Contract Green'),
    task('T033', 'Review', 'US4', `对比 ${paths.helixFixtures}、${paths.pcFixtures} 与 ${paths.mobileFixtures} 的 inventory/schema/raw/semantic hash，写入 ${feature}/verification.md`, 'FR-013,FR-016', 'A-007', `${feature}/verification.md`, 'T012,T022,T032', 'hash parity', 'three mirrors identical', true),
    task('T034', 'Review', 'US4', `执行 ${record.gateId} 行为 Review，核对权限、终态、失败、竞态、恢复和 Actor/Observer 用户结果，记录 ${feature}/verification.md`, 'FR-015,FR-016', 'A-007', `${feature}/verification.md`, 'T033', 'behavior checklist', 'behavior Green'),
    task('T035', 'Review', 'US4', `执行代码边界 Review，证明业务 reducer 只在 Helix 且 PC/Mobile 测试代码 production-unreachable，记录 ${feature}/verification.md`, 'FR-014..FR-017', 'A-007,V-006', `${feature}/verification.md`, 'T033', 'source/build scan', 'code boundary Green', true),
    task('T036', 'Review', 'US4', `执行 Frame V2 Review，核对 envelope/scope/epoch/revision/mutation/action/directive/correlation，记录 ${feature}/verification.md`, 'FR-013,FR-016', 'A-007', `${feature}/verification.md`, 'T033', 'typed frame diff', 'Frame V2 Green', true),
    task('T037', 'Review', 'US4', `按 contracts/cross-repo-report.schema.json 汇总 Helix/Fixture/PC Mock/PC Real/Mobile/boundary/blocker 到 ${feature}/evidence/gate-report.json`, 'FR-018,SC-001..SC-006', 'A-007', `${feature}/evidence/gate-report.json`, 'T034-T036', 'report schema', 'required pass/fail/skip explicit'),
    task('T038', 'Review', 'US4', `运行 quickstart.md、git diff check 与 mandatory after_implement loop；仅 R4 PASS 时将 ${record.gateId} 标记 review-green 并解锁下一 Gate`, 'SC-001..SC-006', 'V-001..V-007', `${feature}/review-ledger.json`, 'T037', 'loop R4', 'Gate review-green'),
  ];
}

// 渲染任务清单和完整 Loop Task Contracts，保证每个 Task 一一可追踪。
function renderTasks(record, tasks) {
  const phases = [
    ['Setup', 'Phase 1: Setup And Source Lock', '锁定 source、前序 Gate 与当前代码边界。'],
    ['Foundational', 'Phase 2: Shared Prerequisite', '复用三端 fresh-group prerequisite；完成前不得进入业务 story。'],
    ['Helix', 'Phase 3: User Story 1 - Helix Producer Green (P1)', '完整业务流、生产 Frame、Fixture 与 Golden。'],
    ['PC', 'Phase 4: User Story 2 - PC Mock/Real/Report Green (P1)', 'PC 机械消费同源 Frame 并完成真实链。'],
    ['Mobile', 'Phase 5: User Story 3 - Mobile GTest/QuickJS Green (P1)', 'Mobile 生产链原子 apply 与 typed view。'],
    ['Review', 'Phase 6: User Story 4 - Cross-repo Review Green (P1)', '行为、代码、Frame V2 与证据一致。'],
  ];
  const body = phases.map(([key, heading, purpose]) => {
    const lines = tasks.filter((item) => item.phase === key).map((item) => {
      const markers = [item.parallel ? '[P]' : '', item.story ? `[${item.story}]` : ''].filter(Boolean).join(' ');
      return `- [ ] ${item.id}${markers ? ` ${markers}` : ''} ${item.description}`;
    });
    return `## ${heading}\n\n**Purpose**: ${purpose}\n\n${lines.join('\n')}`;
  }).join('\n\n---\n\n');
  const contracts = tasks.map((item) =>
    `| ${item.id} | ${item.requirement} | ${item.planItem} | \`${item.scope}\` | ${item.preconditions} | ${item.validator} | ${item.done} |`,
  ).join('\n');
  return `# Tasks: ${record.gateId} ${record.title} 三仓纵向闭环

**Input**: [spec.md](spec.md)、[plan.md](plan.md)、[research.md](research.md)、[data-model.md](data-model.md)、[contracts/](contracts/) 与 [quickstart.md](quickstart.md)

**Generated By**: \`scripts/generate_ifv2_three_repo_plans_tasks.mjs --tasks\`

**Tests**: 用户明确要求 Helix 单元/回放/Fixture、PC WDIO Mock/Real/report、Mobile GTest/QuickJS；测试任务必须先失败再实现。

**Execution Rule**: 前序 \`${record.prerequisiteGate}\` 未 review-green 时只允许 T001-T002 source revalidation，不得开始 T003 之后的实现。

${body}

## Dependencies And Execution Order

\`\`\`text
T001-T002 -> T003 -> T004-T012 -> T013-T022 -> T023-T032 -> T033-T038
\`\`\`

- Helix Fixture 与 provenance 先于任何 PC/Mobile mirror。
- PC Mock 不依赖真实环境，但不能替代 T019-T021。
- Mobile 可在 Helix Green 后与 PC implementation 并行；最终 Review 等待两端完成。
- 共享 production 文件串行修改，Gate 专属测试/fixture 文件可并行。

## Parallel Opportunities

- T004/T009、T014/T017、T024/T025、T034/T035/T036 写不同文件且前置满足后可并行。
- PC 与 Mobile 在 T012 Helix Green 后可并行，但总控仍不允许并行进入另一个业务 Gate。

## Implementation Strategy

1. MVP 是 T001-T012：先取得真实 Helix producer 与 canonical Fixture。
2. T013-T022 完成 PC Mock/Real/report，不以 Mock 提前 Green。
3. T023-T032 完成 Mobile production consumer、GTest 与 QuickJS typed view。
4. T033-T038 独立 Review 并通过 R4 后才进入下一 Gate。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
${contracts}
`;
}

// 渲染总控 Plan，负责顺序、状态和证据汇总而不吞并子 Gate 实现。
function renderMasterPlan(records) {
  return `# Implementation Plan: InteractionFrame V2 26 Gate 三仓总控

**Branch**: \`020-ifv2-three-repo-uc-control\` | **Date**: ${CREATED} | **Spec**: [spec.md](spec.md)

## Summary

总控按 manifest 固定顺序调度 G01 基线与 25 个子 Feature。每次只激活一个 Gate；子 Feature 独立完成 Helix producer/Fixture、PC Mock/Real/report、Mobile GTest/QuickJS 与 Review。总控只读取 ledger/report/hash 形成状态，不实现任何 Gate 业务，不把低等级证据提升为 Green。

## Technical Context

**Language/Version**: Markdown/JSON/Node 20/Python 3；子 Feature 使用 Rust、TypeScript/Angular/Tauri 与 C++/QuickJS

**Primary Dependencies**: gate-spec-manifest、子 Feature review-ledger、cross-repo report、helix-loop

**Storage**: Spec Kit 文件与 hashed evidence；不直接写业务 SQLite/PG

**Testing**: JSON/schema/link/task-traceability、26 个 R2/R3、逐 Gate R4 汇总

**Target Platform**: Helix、cses-client、Mobile 三仓交付编排

**Project Type**: fail-closed program controller

**Performance Goals**: 按需只加载当前 Gate；状态汇总 O(26)，不扫描全量业务日志

**Constraints**: 严格串行 Gate；子证据不可互相替代；用户决定 legacy 删除；总控不进入生产业务路径

**Scale/Scope**: 26 Active、20 Gap、25 子 Feature、1 个 G01 baseline

## Constitution Check

| 原则 | 设计判定 |
|---|---|
| Layering | PASS：总控只读子证据，不承载业务 reducer |
| Determinism | PASS：顺序和状态来自机器 manifest/ledger |
| Frame V2 Truth | PASS：只接受 Helix canonical report/hash |
| Evidence | PASS：每 Gate required lanes 独立 |
| Minimal Migration | PASS：G03、G17-G27 保持 Gap |
| Loop Admission | PASS：每个 Feature 独立 R1-R4 |

**Post-design re-check**: PASS。总控 contracts 只定义编排状态与删除人类门，不修改三仓协议。

## Loop Detail Contract

### Inputs and Context

- **I-001**: [gate-spec-manifest.json](gate-spec-manifest.json) 的固定执行顺序和 Gap。
- **I-002**: [plan-task-index.json](plan-task-index.json) 的子 Plan/Task/ledger 路由。
- **I-003**: 每个子 Feature 的 review-ledger、verification 和 gate-report。
- **I-004**: G01 baseline \`specs/019-g01-wdio-five-phases\` 的当前候选复验结果。

### Action Chain

- **A-001**: 读取当前 Gate 与前序 ledger -> admission 判定 -> 激活一个子 Feature。
- **A-002**: 子 Gate R4/report/hash -> 独立 consistency review -> review-green 或 blocked。
- **A-003**: review-green -> 更新总控状态 -> 激活下一 Gate。
- **A-004**: 26/26 Green -> 生成 legacy 候选 -> 等待用户选择 -> 删除后重跑三仓。

### State Contract

- **S-001**: \`Planned -> GateActive\`：当前 Gate R1-R3 PASS，前序 review-green。
- **S-002**: \`GateActive -> GateGreen\`：该 Gate R4 与 cross-repo review PASS。
- **S-003**: \`GateGreen -> NextGateActive\`：严格按 manifest 前进。
- **S-004**: \`26Green -> DeletionCandidateReady -> AwaitingUserSelection\`。
- **S-005**: 用户批准后 \`Deleting -> FullRegression -> Closed|Blocked\`。

### Validators

- **V-001**: manifest/index/schema 校验证明 26 Active、25 子 Feature、20 Gap 唯一。
- **V-002**: 26 个 R2/R3 ledger 与 task contract 校验证明计划可实施。
- **V-003**: 每 Gate report schema/hash/required lane 检查证明 review-green 不越级。
- **V-004**: 删除候选反向引用与用户选择记录证明无越权删除。
- **V-005**: 删除后三仓全量回归证明最终候选仍 26/26 Green。

## Project Structure

\`\`\`text
${MASTER}/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
├── tasks.md
├── plan-task-index.json
└── plan-task-index.md

specs/021..045/
└── plan/research/data-model/contracts/quickstart/tasks
\`\`\`

**Structure Decision**: 总控只保存索引、顺序、状态与删除人类门；所有测试和生产接线任务留在对应子 Feature。

## Delivery Phases

1. 规格和 R1（已完成）。
2. 总控与 25 子 Plan/R2。
3. 总控与 25 子 Task/R3。
4. 按 manifest 逐 Gate implementation/R4。
5. 26 Green 后 legacy 候选、用户选择、删除后全量回归。

## Complexity Tracking

无 Constitution 例外。26 个 Feature 的文档数量来自强隔离验收需求，不代表并行实现或复制业务逻辑。

## Child Plans

${records.map((record) => `- \`${record.gateId}\`: [plan](${relative(MASTER, `${record.featureDirectory}/plan.md`)}) · [tasks](${relative(MASTER, `${record.featureDirectory}/tasks.md`)})`).join('\n')}
`;
}

// 渲染总控研究、模型、合同与 quickstart 的最小但完整 Phase 0/1 资产。
function renderMasterArtifacts(records) {
  write(`${MASTER}/research.md`, `# Research: 26 Gate 三仓总控

## Decision 1

**Decision**: 一个 Gate 完成 Helix、PC、Mobile 与 Review 后再进入下一个。

**Rationale**: 并行实施会让共享生产文件和 Fixture pin 发生交叉漂移。

**Alternatives considered**: 26 Gate 并行、按仓横向批量实现；全部拒绝。

## Decision 2

**Decision**: G01 复用 019 基线；G02 是第一个试点；G03a、G04-G16c、GS1-GS4 依次推进。

**Rationale**: 符合用户确认的试点和纵向闭环策略。

**Alternatives considered**: GS 优先或按代码完成度跳序；除非用户改范围，否则拒绝。

## Decision 3

**Decision**: 26 Green 后只生成删除候选，用户选择后才删除并重跑。

**Rationale**: v1 名称可能属于非 Frame 版本域，机械删除不可接受。

**Alternatives considered**: 逐 Gate 物理删除、名称匹配全删；全部拒绝。

## Source Locks

- Helix: \`${HELIX_HEAD}\`
- cses-client: \`${PC_HEAD}\`
- Mobile: \`${MOBILE_HEAD}\`
- 子 Feature: ${records.length}
`);
  write(`${MASTER}/data-model.md`, `# Data Model: 26 Gate 总控

## GateRecord

gateId、order、featureDirectory、prerequisiteGate、R1-R4、sourcePins、fixtureHashes、laneVerdicts、reviewVerdict。

## ProgramState

\`Planned -> GateActive -> GateGreen -> NextGateActive -> 26Green -> AwaitingDeletionSelection -> FullRegression -> Closed\`

任一 required fail/skip/hash drift 进入 Blocked；不得跳过前序 Gate。

## DeletionCandidate

path、kind、versionDomain、reverseReferences、risk、selectedByUser、deletionCommit、postDeleteEvidence。
`);
  write(`${MASTER}/contracts/execution-order.md`, `# Execution Order Contract

${records.map((record, index) => `${index + 2}. \`${record.gateId}\` 依赖 \`${record.prerequisiteGate}\` review-green。`).join('\n')}

G01 为顺序 1。G03 与 G17-G27 不在 Active 分母。
`);
  write(`${MASTER}/contracts/gate-status.schema.json`, JSON.stringify({
    $schema: 'https://json-schema.org/draft/2020-12/schema',
    type: 'object',
    required: ['gateId', 'order', 'lanes', 'review', 'verdict'],
    properties: {
      gateId: { type: 'string' },
      order: { type: 'integer', minimum: 1, maximum: 26 },
      lanes: { type: 'object' },
      review: { type: 'object' },
      verdict: { enum: ['Green', 'Red', 'Blocked', 'Partial', 'Draft'] },
    },
  }, null, 2));
  write(`${MASTER}/quickstart.md`, `# Quickstart: 26 Gate 总控

1. 读取 [plan-task-index.md](plan-task-index.md)，只进入当前 Gate。
2. 确认前序 Gate review-green。
3. 执行当前子 Feature 的 quickstart.md 和 tasks.md。
4. R4 与 cross-repo report Green 后才更新总控并进入下一行。
5. 26/26 Green 后生成删除候选；等待用户选择；删除后重新跑 26 Gate。

\`\`\`bash
python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance
node scripts/verify_ifv2_three_repo_plans_tasks.mjs
git diff --check
\`\`\`
`);
}

// 渲染总控 Tasks：每个 Gate 一个按需入口，具体实现留在子 tasks.md。
function renderMasterTasks(records) {
  const tasks = [
    task('T001', 'Setup', null, `校验 ${MASTER}/gate-spec-manifest.json 与 plan-task-index.json 的 26 Active/20 Gap/唯一顺序`, 'FR-001..FR-004', 'I-001,I-002', `${MASTER}/plan-task-index.json`, 'R1/R2 pass', 'V-001', 'index valid'),
    task('T002', 'Setup', null, `复验 specs/019-g01-wdio-five-phases/verification.md 的最终候选 source/hash/Mock/Real/Mobile 基线`, 'FR-002,FR-014', 'I-004', 'specs/019-g01-wdio-five-phases/verification.md', 'T001', 'G01 focused gates', 'G01 baseline current'),
    ...records.map((record, index) => task(
      `T${String(index + 3).padStart(3, '0')}`,
      'Gates',
      'US1',
      `仅在 ${record.prerequisiteGate} review-green 后执行 ${record.featureDirectory}/tasks.md，并将 R4/report/hash 结果登记到 ${MASTER}/verification.md`,
      'FR-004..FR-016',
      'A-001..A-003',
      `${record.featureDirectory}/tasks.md`,
      index === 0 ? 'T002' : `T${String(index + 2).padStart(3, '0')}`,
      'child R4 + report schema',
      `${record.gateId} review-green`,
    )),
    task('T028', 'Closure', 'US2', `汇总 26 Gate required pass/fail/skip、source pin 与 Fixture hash 到 ${MASTER}/evidence/program-report.json`, 'FR-014..FR-020', 'A-003', `${MASTER}/evidence/program-report.json`, 'T027', 'V-003', '26/26 report Green'),
    task('T029', 'Closure', 'US3', `生成 V1/legacy 删除候选与反向引用到 ${MASTER}/evidence/legacy-deletion-candidates.json，不执行删除`, 'FR-017,FR-019', 'A-004', `${MASTER}/evidence/legacy-deletion-candidates.json`, 'T028', 'V-004', 'candidate list ready'),
    task('T030', 'Closure', 'US3', `记录用户选择到 ${MASTER}/evidence/legacy-deletion-selection.json；无选择时停止`, 'FR-017..FR-019', 'A-004', `${MASTER}/evidence/legacy-deletion-selection.json`, 'T029', 'human gate', 'approved scope explicit'),
    task('T031', 'Closure', 'US3', `按用户批准范围执行删除并在 ${MASTER}/verification.md 记录 commit/path，不触碰未选资产`, 'FR-018,FR-019', 'A-004', `${MASTER}/verification.md`, 'T030', 'reverse reference scan', 'approved deletion only'),
    task('T032', 'Closure', 'US3', `删除后重新执行三仓 26 Gate 全量回归并运行 mandatory R4，结果写入 ${MASTER}/evidence/post-delete-regression.json`, 'FR-018,SC-006', 'A-004,V-005', `${MASTER}/evidence/post-delete-regression.json`, 'T031', 'V-005 + loop R4', 'program Closed or Blocked'),
  ];
  const body = [
    ['Setup', 'Phase 1: Program Setup', '锁定机器索引与 G01 当前基线。'],
    ['Gates', 'Phase 2: User Story 1 - Sequential Gate Delivery (P1)', '按需逐 Gate 进入子 Feature。'],
    ['Closure', 'Phase 3: User Story 2/3 - Evidence And User-controlled Deletion (P2)', '汇总证据并执行删除人类门。'],
  ].map(([key, heading, purpose]) => `## ${heading}\n\n**Purpose**: ${purpose}\n\n${tasks.filter((item) => item.phase === key).map((item) => {
    const marker = item.story ? ` [${item.story}]` : '';
    return `- [ ] ${item.id}${marker} ${item.description}`;
  }).join('\n')}`).join('\n\n---\n\n');
  return `# Tasks: InteractionFrame V2 26 Gate 三仓总控

**Input**: [plan.md](plan.md)、[plan-task-index.md](plan-task-index.md) 与全部子 Feature tasks.md。

**Generated By**: \`scripts/generate_ifv2_three_repo_plans_tasks.mjs --tasks\`

${body}

## Dependencies And Execution Order

\`T001 -> T002 -> T003..T027 严格串行 -> T028 -> T029 -> Human T030 -> T031 -> T032\`

总控 Gate task 不复制子任务；执行时只加载当前子 Feature。

## Parallel Opportunities

无跨 Gate 并行实现。当前 Gate 内部仅按其 tasks.md 标注的 [P] 机会并行。

## Implementation Strategy

先完成 G02 试点并校准模板，再按 manifest 顺序推进；每个 Gate R4 后做一次一致性 Review。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
${tasks.map((item) => `| ${item.id} | ${item.requirement} | ${item.planItem} | \`${item.scope}\` | ${item.preconditions} | ${item.validator} | ${item.done} |`).join('\n')}
`;
}

// 生成机器/人读索引，支持总控按需进入单一 Gate。
function renderIndexes(records) {
  const machine = {
    schemaVersion: 1,
    generatedAt: CREATED,
    sourcePins: { helix: HELIX_HEAD, csesClient: PC_HEAD, mobile: MOBILE_HEAD },
    master: {
      featureDirectory: MASTER,
      plan: `${MASTER}/plan.md`,
      tasks: `${MASTER}/tasks.md`,
    },
    baseline: {
      gateId: 'G-01',
      featureDirectory: 'specs/019-g01-wdio-five-phases',
      plan: 'specs/019-g01-wdio-five-phases/plan.md',
      tasks: 'specs/019-g01-wdio-five-phases/tasks.md',
    },
    gates: records.map((record) => ({
      order: record.order,
      gateId: record.gateId,
      title: record.title,
      prerequisiteGate: record.prerequisiteGate,
      featureDirectory: record.featureDirectory,
      plan: `${record.featureDirectory}/plan.md`,
      tasks: `${record.featureDirectory}/tasks.md`,
      quickstart: `${record.featureDirectory}/quickstart.md`,
      contracts: `${record.featureDirectory}/contracts/`,
    })),
  };
  write(`${MASTER}/plan-task-index.json`, JSON.stringify(machine, null, 2));
  const rows = [
    '| 顺序 | Gate | 业务 | Plan | Tasks | Quickstart | 前置 |',
    '|---:|---|---|---|---|---|---|',
    '| 1 | G-01 | 发送文本基线 | [plan](../019-g01-wdio-five-phases/plan.md) | [tasks](../019-g01-wdio-five-phases/tasks.md) | [verification](../019-g01-wdio-five-phases/verification.md) | baseline reverify |',
    ...records.map((record) => {
      const base = record.featureDirectory.split('/').at(-1);
      return `| ${record.order} | ${record.gateId} | ${record.title} | [plan](../${base}/plan.md) | [tasks](../${base}/tasks.md) | [quickstart](../${base}/quickstart.md) | ${record.prerequisiteGate} review-green |`;
    }),
  ];
  write(`${MASTER}/plan-task-index.md`, `# InteractionFrame V2 26 Gate Plan/Task 索引

> 机器真源：[plan-task-index.json](plan-task-index.json)。只打开当前 Gate 的 Plan/Tasks，避免把 26 个实现上下文一次性加载。

${rows.join('\n')}

## 每 Gate 固定产出

Helix 完整场景与 canonical Fixture/Golden；PC WDIO Mock/Real/assertion/report；Mobile mirror/GTest/production SDK-Bridge-SurfaceRuntime/QuickJS typed view；行为、代码与 Frame V2 Review。
`);
  write(`${DOCS}/08-26Gate-Plan-Task实施索引.md`, `# InteractionFrame V2 26 Gate Plan/Task 实施索引

- [总控 Plan](../../${MASTER}/plan.md)
- [总控 Tasks](../../${MASTER}/tasks.md)
- [机器索引](../../${MASTER}/plan-task-index.json)
- [人读索引](../../${MASTER}/plan-task-index.md)

## 按需执行

${records.map((record) => `- \`${record.gateId}\` ${record.title}：[Plan](../../${record.featureDirectory}/plan.md) · [Tasks](../../${record.featureDirectory}/tasks.md) · [Quickstart](../../${record.featureDirectory}/quickstart.md)`).join('\n')}

G01 使用既有 019 Plan/Tasks 并在最终候选重验。G03 与 G17-G27 仍为 Gap。
`);
}

// 校验必需设计资产、Task 格式、合同行与索引数量。
function validate(records) {
  const errors = [];
  const taskPattern = /^- \[ \] (T\d{3})\b/gm;
  const contractPattern = /^\|\s*(T\d{3})\s*\|/gm;
  for (const record of [{ featureDirectory: MASTER, gateId: 'MASTER' }, ...records]) {
    const required = ['plan.md', 'research.md', 'data-model.md', 'quickstart.md', 'tasks.md'];
    for (const file of required) {
      if (!existsSync(join(ROOT, record.featureDirectory, file))) errors.push(`${record.gateId} missing ${file}`);
    }
    const tasksText = readFileSync(join(ROOT, record.featureDirectory, 'tasks.md'), 'utf8');
    const ids = [...tasksText.matchAll(taskPattern)].map((match) => match[1]);
    const rows = [...tasksText.matchAll(contractPattern)].map((match) => match[1]);
    if (!ids.length || ids.join(',') !== rows.join(',')) errors.push(`${record.gateId} task/contract mismatch`);
    if (new Set(ids).size !== ids.length) errors.push(`${record.gateId} duplicate task ID`);
    const combined = required.map((file) => readFileSync(join(ROOT, record.featureDirectory, file), 'utf8')).join('\n');
    if (/\[(?:FEATURE|DATE|NEEDS CLARIFICATION|Command and expected falsifiable result)/i.test(combined)) {
      errors.push(`${record.gateId} unresolved placeholder`);
    }
  }
  if (records.length !== 25) errors.push(`child record count ${records.length}`);
  if (errors.length) throw new Error(errors.join('\n'));
}

// 生成 Plan 阶段设计资产；--tasks 仅在 R2 通过后生成 Tasks 和索引。
function main() {
  const mode = process.argv[2] ?? '--all';
  if (!['--plans', '--tasks', '--all'].includes(mode)) throw new Error(`unknown mode ${mode}`);
  const manifest = readJson(MANIFEST);
  const matrix = readJson(MATRIX);
  const rows = new Map(matrix.uc_cases.map((row) => [row.gate_id, row]));
  const records = manifest.scope.children;

  if (mode !== '--tasks') {
    for (const record of records) {
      const row = rows.get(record.gateId);
      const paths = gatePaths(record);
      write(`${record.featureDirectory}/plan.md`, renderPlan(record, row, paths));
      write(`${record.featureDirectory}/research.md`, renderResearch(record, row, paths));
      write(`${record.featureDirectory}/data-model.md`, renderDataModel(record, row));
      write(`${record.featureDirectory}/contracts/gate-acceptance.md`, renderAcceptanceContract(record, row, paths));
      write(`${record.featureDirectory}/contracts/canonical-fixture.schema.json`, renderFixtureSchema(record, row));
      write(`${record.featureDirectory}/contracts/cross-repo-report.schema.json`, renderReportSchema(record));
      write(`${record.featureDirectory}/quickstart.md`, renderQuickstart(record, paths));
    }
    write(`${MASTER}/plan.md`, renderMasterPlan(records));
    renderMasterArtifacts(records);
  }

  if (mode !== '--plans') {
    for (const record of records) {
      const row = rows.get(record.gateId);
      const paths = gatePaths(record);
      write(`${record.featureDirectory}/tasks.md`, renderTasks(record, childTasks(record, row, paths)));
    }
    write(`${MASTER}/tasks.md`, renderMasterTasks(records));
    renderIndexes(records);
    validate(records);
  }

  process.stdout.write(`${JSON.stringify({ mode, master: MASTER, childFeatures: records.length })}\n`);
}

main();
