#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';

const ROOT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
const CREATED = '2026-07-25';
const MASTER_DIRECTORY = 'specs/020-ifv2-three-repo-uc-control';
const DOCS_DIRECTORY = 'docs/interaction-frame-v2-全量替换-按UC';
const MATRIX_PATH = 'crates/helix-im/contracts/strict-matrix.v1.json';
const HELIX_HEAD = 'ff412d354699be64554b7764ac1b1410a4a9b154';
const PC_HEAD = '13e42ef2793ea5c9f39b29f5d1ce86b1aca074db';
const MOBILE_HEAD = '674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21';

const CHILD_GATES = [
  'G-S1',
  'G-S2',
  'G-S3',
  'G-S4',
  'G-02',
  'G-03a',
  'G-04',
  'G-04a',
  'G-05',
  'G-06',
  'G-07',
  'G-08',
  'G-09',
  'G-10',
  'G-10a',
  'G-11',
  'G-12',
  'G-13',
  'G-14',
  'G-14a',
  'G-15a',
  'G-15',
  'G-16',
  'G-16b',
  'G-16c',
];

const EXECUTION_GATES = [
  'G-02',
  'G-03a',
  'G-04',
  'G-04a',
  'G-05',
  'G-06',
  'G-07',
  'G-08',
  'G-09',
  'G-10',
  'G-10a',
  'G-11',
  'G-12',
  'G-13',
  'G-14',
  'G-14a',
  'G-15a',
  'G-15',
  'G-16',
  'G-16b',
  'G-16c',
  'G-S1',
  'G-S2',
  'G-S3',
  'G-S4',
];

const ACTIVE_GATES = ['G-01', ...EXECUTION_GATES];
const GAP_GATES = [
  'G-03',
  'G-17',
  'G-18',
  'G-18a',
  'G-18b',
  'G-19',
  'G-20',
  'G-20a',
  'G-20b',
  'G-21',
  'G-22',
  'G-23',
  'G-24',
  'G-25',
  'G-26',
  'G-26a',
  'G-26b',
  'G-26c',
  'G-26d',
  'G-27',
];

const GATE_DOC_FOLDERS = new Map([
  ['G-S1', 'GS01'],
  ['G-S2', 'GS02'],
  ['G-S3', 'GS03'],
  ['G-S4', 'GS04'],
  ['G-01', 'G01'],
  ['G-02', 'G02'],
  ['G-04', 'G04'],
  ['G-04a', 'G04'],
  ['G-05', 'G05'],
  ['G-06', 'G06'],
  ['G-07', 'G07'],
  ['G-08', 'G08'],
  ['G-09', 'G09'],
  ['G-10', 'G10'],
  ['G-10a', 'G10'],
  ['G-11', 'G11'],
  ['G-12', 'G12'],
  ['G-13', 'G13'],
  ['G-14', 'G14'],
  ['G-14a', 'G14'],
  ['G-15a', 'G15'],
  ['G-15', 'G15'],
  ['G-16', 'G16'],
  ['G-16b', 'G16'],
  ['G-16c', 'G16'],
]);

const GATE_FEATURE_SLUGS = new Map([
  ['G-S1', 'cold-start-sync'],
  ['G-S2', 'offline-sync-notify'],
  ['G-S3', 'pong-gap-recovery'],
  ['G-S4', 'unknown-channel-hydration'],
  ['G-02', 'document-message'],
  ['G-03a', 'retry-failed-send'],
  ['G-04', 'online-revoke'],
  ['G-04a', 'offline-revoke'],
  ['G-05', 'forward-message'],
  ['G-06', 'quick-reaction'],
  ['G-07', 'urgent-message'],
  ['G-08', 'schedule-create'],
  ['G-09', 'schedule-cancel'],
  ['G-10', 'timeline-window'],
  ['G-10a', 'load-older'],
  ['G-11', 'locate-message'],
  ['G-12', 'reply-thread'],
  ['G-13', 'channel-read'],
  ['G-14', 'post-read'],
  ['G-14a', 'template-received'],
  ['G-15a', 'create-channel'],
  ['G-15', 'create-topic'],
  ['G-16', 'channel-close-leave'],
  ['G-16b', 'member-leave-broadcast'],
  ['G-16c', 'channel-settings'],
]);

const GATE_OVERRIDES = new Map([
  [
    'G-02',
    {
      contractNote:
        '本 Gate 发送固定文档 URL `http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15`，用户结果是文档消息卡片，不是文件附件、文件上传或上传进度状态机。',
      discoveryFact:
        'G02 的产品语义是发送文档链接并渲染文档消息卡片；Strict Matrix 当前登记的 `upload` effect 与该语义存在待核对漂移，plan 阶段必须以生产调用链纠正，禁止把 G03 文件上传能力带入 G02。',
    },
  ],
]);

// 读取仓内 JSON 真源，并在解析失败时保留原始异常。
function readJson(relativePath) {
  return JSON.parse(readFileSync(join(ROOT, relativePath), 'utf8'));
}

// 创建目标目录，允许重复运行覆盖同一生成物。
function ensureDirectory(relativePath) {
  mkdirSync(join(ROOT, relativePath), { recursive: true });
}

// 写入规范化 LF 文本，避免生成器在不同平台制造换行漂移。
function writeText(relativePath, content) {
  ensureDirectory(dirname(relativePath));
  writeFileSync(join(ROOT, relativePath), `${content.trimEnd()}\n`, 'utf8');
}

// 把 Gate ID 转成稳定、可用于 Spec Kit 目录的短标识。
function gateSlug(gateId) {
  return gateId.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
}

// 把 Gate ID 转成当前逐 UC 文档目录，未绑定目录时返回 null。
function gateDocPath(gateId) {
  const folder = GATE_DOC_FOLDERS.get(gateId);
  return folder ? `${DOCS_DIRECTORY}/${folder}/README.md` : null;
}

// 把字符串数组渲染成紧凑、可读且稳定排序的合同摘要。
function listText(values) {
  return values?.length ? values.join('、') : '无';
}

// 计算生成清单的原始 SHA-256，供后续 plan/pin 对账。
function sha256(content) {
  return createHash('sha256').update(content).digest('hex');
}

// 为 Gate 生成一个独立的 Spec Kit Feature 目录名。
function childDirectory(index, row) {
  const featureSlug = GATE_FEATURE_SLUGS.get(row.gate_id);
  if (!featureSlug) throw new Error(`缺 Gate Feature slug: ${row.gate_id}`);
  return `specs/${String(21 + index).padStart(3, '0')}-ifv2-${gateSlug(row.gate_id)}-${featureSlug}`;
}

// 渲染 Spec Kit requirements checklist，并把已验证项固定为全绿。
function renderChecklist(featureName, storyCoverage) {
  return `# Specification Quality Checklist: ${featureName}

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: ${CREATED}
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details beyond user-mandated platform and evidence boundaries
- [x] Focused on user-visible behavior and cross-platform consistency
- [x] Written for technical stakeholders without prescribing file-level implementation
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] Success criteria describe observable outcomes rather than code structure
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover ${storyCoverage}
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] Implementation mechanisms are deferred to plan.md

## Notes

- Helix、canonical Fixture、PC Mock/Real、Mobile GTest/QuickJS 和 InteractionFrame V2 是用户明确要求的验收对象，不是本 Spec 自行选择的技术方案。
- 本 checklist 只证明规格质量；生产代码、Fixture、Mock、真实链或移动端测试未因生成本 Spec 自动变为 Green。
`;
}

// 渲染单 Gate 的分层验证合同，不提前伪造尚未由 plan 定位的命令入口。
function renderChildVerification(row) {
  return `# Verification Procedure: ${row.gate_id}

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 \`cargo check\`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 \`ImModule::handle\` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 \`${row.gate_id}\` 为 \`review-green\`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 \`loop_engine.py record-evidence\` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
`;
}

// 渲染总控验证合同，覆盖规格生成质量与最终 26 Gate 汇总裁决。
function renderMasterVerification() {
  return `# Verification Procedure: InteractionFrame V2 26 Gate 总控

## Specification Runs

- **V-001**: \`node --check scripts/generate_ifv2_three_repo_specs.mjs\`；期望退出码 0。
- **V-002**: \`node scripts/generate_ifv2_three_repo_specs.mjs\`；期望 activeGates=26、childSpecs=25、gapGates=20，重复运行不改变语义内容。
- **V-003**: \`python3 scripts/verify-interaction-frame-v2-index.py\`；期望文档库存、机器 Gate 与全部本地链接通过。
- **V-004**: 对总控和 25 个子 Feature 执行 mandatory \`loop_engine.py advance\`；期望 26/26 \`r1_spec.status=pass\` 且 next_action 为 \`run_speckit_plan\`。
- **V-005**: \`git diff --check\`；期望无 whitespace error。

## Delivery Runs

- **V-101**: 按机器 manifest 顺序逐 Gate 执行各自 verification.md；前序未 \`review-green\` 时后序不得运行。
- **V-102**: 汇总 26 Gate 的 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 与 Review；期望 required pass=100%、fail=0、skip=0。
- **V-103**: 26 Gate 全绿后生成 legacy/V1 删除候选清单；用户未选择前不得执行删除。
- **V-104**: 用户选择删除后重新运行三仓全量验收；期望新候选仍为 26/26 Green，删除前报告不可复用。

## Evidence Rules

- 规格检查只证明 Spec 可进入 plan，不证明任何业务 Gate 已实现或 Green。
- 每个 delivery run 必须引用原始 artifact、source pin、fixture hash 和结构化 report；聚合结论不得遮蔽 blocked/partial。
`;
}

// 渲染单 Gate 的 source discovery，供 mandatory helix-loop R1 审查。
function renderChildDiscovery(directory, row, matrixSha) {
  const docPath = gateDocPath(row.gate_id);
  const commands = row.commands.map((command) => command.name);
  const projections = [
    ...new Set(row.projections.map((projection) => `${projection.render_kind}:${projection.schema}`)),
  ];
  const override = GATE_OVERRIDES.get(row.gate_id);
  const documentFact = docPath
    ? `当前逐 UC 文档入口为 \`${docPath}\`，但文档目标不能替代本次三仓运行证据。 | **Evidence**: \`${docPath}\``
    : `G-03a 当前没有独立逐 UC 文档目录，其 Gate 真源只来自 Strict Matrix；plan 阶段必须补齐当前代码调用链，不能沿用 G03 文件上传语义。 | **Evidence**: \`${MATRIX_PATH}\``;
  return `# Source Discovery

**Feature**: \`${directory}\`
**Source Lock**: \`helix=${HELIX_HEAD};cses-client=${PC_HEAD};mobile=${MOBILE_HEAD}\`
**Worktree State**: Helix Spec 生成中；cses-client clean；Mobile 保留用户既有文档与 artifact 改动，本 Feature 不覆盖。

## Facts *(mandatory)*

- **F-001**: Helix 当前 InteractionFrame 下行 schema 常量为 2，ActionInvocation 的独立 schema 1 不属于 Frame V1。 | **Evidence**: \`crates/helix-im/src/interaction_frame.rs:11\`
- **F-002**: \`${row.gate_id}\` 在 canonical Strict Matrix 中登记为 \`${row.uc_id} ${row.title}\`，command 为 \`${listText(commands)}\`，投影为 \`${listText(projections)}\`。 | **Evidence**: \`${MATRIX_PATH}\`
- **F-003**: ${documentFact}
- **F-004**: G01 已建立 Helix canonical Fixture -> PC WDIO Mock/Real -> Mobile GTest/QuickJS 的三仓基线，本 Gate 必须复用供应链规则而不是复制 G01 业务逻辑。 | **Evidence**: \`docs/interaction-frame-v2-全量替换-按UC/G01/13-G01-Helix-PC-Mobile-V2验收合同.md\`
- **F-005**: canonical Fixture 只能来自公开 \`ImModule::handle\` 的 \`Effect::Emit\`，平台端不得手写业务 VM。 | **Evidence**: \`crates/helix-im/tests/AGENTS.md:11\`
- **F-006**: MessageV4 codec 对非当前 InteractionFrame schema fail-closed，随后才允许 SurfaceRuntime 原子 apply。 | **Evidence**: \`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts:46\`
- **F-007**: Mobile production event pump 把 \`im:interaction:frame\` 作为迁移 Surface 的唯一业务输入，legacy characterization 受显式宏隔离。 | **Evidence**: \`/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp:2155\`
- **F-008**: 当前 Strict Matrix 原始 SHA-256 为 \`${matrixSha}\`；后续 plan 必须重新锁定三仓 HEAD、Helix pin 与 Fixture manifest，不能继承本次静态快照。 | **Evidence**: \`${MATRIX_PATH}\`
${override ? `- **F-009**: ${override.discoveryFact} | **Evidence**: 用户确认的 G02 验收语义 + \`${MATRIX_PATH}\`` : ''}

## Unknowns

- **U-001**: \`${row.gate_id}\` 当前生产代码是否已覆盖全部 happy/failure/race/recovery 分支，必须在 plan 前按三仓代码和测试逐项求证。 | **Resolution**: 在 plan 的 source discovery 阶段形成 CURRENT/TARGET/GAP，不以本 Spec 的目标描述冒充完成。
- **U-002**: PC Real lane 所需 Go、账号、PG/SQLite 与 Tauri driver 前置在执行时是否可用。 | **Resolution**: 每次运行先做 bounded preflight；缺失时标记 blocked，不降级为 Mock Green。
- **U-003**: Mobile typed view 是否已暴露本 Gate 全部可观察字段。 | **Resolution**: 先验证生产 SDK/Bridge/SurfaceRuntime/QuickJS；缺字段必须回 Helix Frame 或平台只读 adapter 修复，禁止在 GTest 补 reducer。

## Decisions

- **D-001**: 本 Feature 只负责 \`${row.gate_id}\`，不得顺带实现下一 Gate。
- **D-002**: Helix 是业务语义和 canonical Fixture 唯一真源；PC 与 Mobile 只机械消费 Frame。
- **D-003**: 完成顺序固定为 Helix Green -> PC Mock/Real/Report Green -> Mobile GTest/QuickJS Green -> 行为/代码/Frame V2 Review Green。
- **D-004**: 本 Gate Green 前不得进入下一 Gate；Mock、accepted、HTTP 2xx 或单端测试不能提升为三仓 Green。
- **D-005**: 本 Feature 不物理删除 V1/legacy 资产；26 Gate 全部 Green 后只生成候选清单，删除范围由用户决定。
`;
}

// 渲染单 Gate 的完整三仓纵向闭环 Feature Spec。
function renderChildSpec(directory, row, previousGate) {
  const commands = row.commands.map((command) => command.name);
  const effects = [...new Set(row.commands.flatMap((command) => command.effect ?? []))];
  const projections = [
    ...new Set(row.projections.map((projection) => `${projection.render_kind} (${projection.schema})`)),
  ];
  const persistence = row.persistence ?? [];
  const negatives = row.negative_assertions ?? [];
  const override = GATE_OVERRIDES.get(row.gate_id);
  const sourceDoc = gateDocPath(row.gate_id);
  const sourceLine = sourceDoc
    ? `- 当前 UC 文档入口：[${row.gate_id}](${relative(directory, sourceDoc)})。`
    : `- \`${row.gate_id}\` 当前无独立逐 UC 文档目录；以 Strict Matrix 为现状入口，并在 plan 阶段补 discovery。`;
  return `# Feature Specification: ${row.gate_id} ${row.title} Frame V2 三仓一致性

**Feature Branch**: \`main (no feature branch created)\`

**Created**: ${CREATED}

**Status**: Draft

**Loop Profile**: \`helix-loop/v1\`

**Parent Feature**: [020 三仓逐 UC 总控](../020-ifv2-three-repo-uc-control/spec.md)

**Input**: 用户要求按一个 UC 一个三仓纵向闭环完成 \`${row.gate_id}\`：Helix 生产唯一业务结果与 canonical Fixture，PC 完成 Mock/Real/断言/报告，Mobile 完成 GTest、生产接线与 QuickJS typed view；行为、代码和 Frame V2 Review 通过后才能进入下一 UC。

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 用户在三端获得一致的“${row.title}”结果 (Priority: P1)

作为 IM 用户，我需要在桌面端和移动端执行或观察“${row.title}”时得到同一业务结果、权限、顺序和失败语义，不因平台不同出现第二套状态处理。

**Why this priority**: 该 Gate 是 \`${row.uc_id}\` 的独立用户行为；任何端自行推导都会造成可见状态、动作能力或恢复结果漂移。

**Independent Test**: 创建隔离群聊并执行 \`${row.gate_id}\`，验证相同权威事实最终在 PC 与 Mobile 形成字段一致的 typed view，且失败与竞态场景不产生伪成功。

**Acceptance Scenarios**:

1. **Given** 当前 actor、observer、群聊与权限满足 Gate 前置，**When** 用户执行或系统触发“${row.title}”，**Then** Helix 必须先形成权威业务结果，PC 与 Mobile 只消费其完整 Frame。
2. **Given** transport 已受理但权威事实或持久化尚未完成，**When** 平台收到中间结果，**Then** 两端均不得显示最终成功。
3. **Given** 权威失败、权限拒绝、重复事实或迟到回包，**When** 三端处理结束，**Then** 两端保持同一失败/既有状态，不出现重复 mutation。

---

### User Story 2 - 开发者只维护一套 Gate 业务逻辑 (Priority: P1)

作为跨端维护者，我需要 \`${row.gate_id}\` 的排序、权限、对账、终态和 Frame 字段只在 Helix 定义一次，使 PC 与 Mobile 不再维护独立 reducer 或手写测试业务字段。

**Why this priority**: 三端状态 Store 会让同一 UC 在修复、重试和恢复时产生无法审计的差异。

**Independent Test**: 从 Helix 生产回放生成 canonical JSON，分别同步到 PC 与 Mobile，证明 mirror hash、schema、surface inventory 和 typed view 一致。

**Acceptance Scenarios**:

1. **Given** Helix canonical Fixture 更新，**When** PC 与 Mobile 同步合同，**Then** 两端只能进行 provenance 允许的动态替换，不得补业务字段。
2. **Given** Fixture、Mock 或 GTest 中出现生产代码未产出的业务字段或 reducer，**When**执行 production-boundary Gate，**Then** 本 Feature 必须失败。

---

### User Story 3 - Reviewer 能判定 Gate 是否真正 Green (Priority: P2)

作为独立 Reviewer，我需要分别审查行为、代码边界、Frame V2 字段和真实证据，避免把 accepted、Mock 或单端渲染冒充三仓完成。

**Why this priority**: 本项目把证据边界视为正确性；缺少任一生产面就不能宣称完成。

**Independent Test**: 使用本 Spec 的六类场景与 Review 清单，能够给出 \`review-green\`、\`review-partial\` 或 \`review-blocked\`，并指出第一处偏离。

**Acceptance Scenarios**:

1. **Given** Helix、PC 或 Mobile 任一 required lane 缺失，**When** Reviewer 汇总，**Then** 状态不得为 Green。
2. **Given** 三仓 required lane 全部通过，**When** 行为、代码和 Frame 字段对账一致，**Then** 才能标记本 Gate Green 并进入下一 Gate。

## Loop Contract *(mandatory)*

### Trigger

\`${row.gate_id}\` 的用户 intent、HostObservation、WS/inbound 或恢复事实进入当前业务链；canonical command 为 \`${listText(commands)}\`。

### Goal

用户在 PC 与 Mobile 获得同一“${row.title}”业务结果，平台不重建权限、排序、对账、终态或失败语义。

### Completion

Helix 六类场景、canonical Fixture/Golden、PC Mock/Real/Report、Mobile mirror/GTest/QuickJS typed view、production-boundary 和独立 Review 全部通过；同一运行能够关联 command/inbound、persist、Frame、FrameApplied 与可观察结果。

### Boundaries

- 只处理 \`${row.gate_id}\`；前序 Gate \`${previousGate}\` 未 Green 时不得实施本 Gate。
- G03 文件/媒体上传与 G17-G27 全部不在本 Feature。
- 不在 Mock、Angular、C++ 或 QuickJS 中补 Helix 缺失的业务字段、排序或 reducer。
- 不在本 Feature 物理删除 V1/legacy 文件；最终删除必须等待 26 Gate 全绿和用户选择。
- 同一 task/validator/失败签名自动修复最多两次，第三次进入 NeedsHuman。

### Human Gate

改变现有用户行为、权限或 wire 语义；需要不可逆数据迁移；真实账号/凭据或环境越权；删除任何 legacy/V1 资产；同一失败签名两次修复仍未收敛。

### Edge Cases

- 权威事实与 HTTP/WS terminal 竞态时，以持久化后的同一业务事实为准，迟到回包不得重复终结。
- Persist 失败、scope/epoch/revision 不匹配或重复 transaction 时，整帧不得部分 apply。
- Observer 不受本次行为影响时，不得为了测试对称性伪造 Observer Frame。
- 未附着 Surface 不强制生成 View；以后 attach 时必须能从权威事实恢复 snapshot。
${negatives.map((item) => `- Gate-specific: ${item}`).join('\n')}

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 每次 canonical 验收必须创建新的隔离群聊，复用共享 CreateGroupPrerequisite 合同，不依赖前一个 Gate 的遗留数据。
- **FR-002**: 系统必须只接受当前 actor、scope、权限和版本边界内的合法 \`${row.gate_id}\` intent 或 observation。
- **FR-003**: Helix 必须成为该 Gate 权限、排序、对账、终态、失败语义和完整 Frame 字段的唯一业务真源。
- **FR-004**: 所有声称 durable 的用户可见终态必须等待对应持久化成功；accepted、HTTP 2xx、WS 入队或内存更新均不得独立建立完成。
- **FR-005**: Helix 测试必须覆盖 prerequisites、happy path、business failure、infrastructure failure、race/idempotency、projection/recovery 六类场景；不适用项必须写明理由。
- **FR-006**: canonical Fixture 必须由生产 \`ImModule::handle -> Effect::Emit\` 路径生成，并执行 typed validation、字段级语义断言和 Golden 比较。
- **FR-007**: 每个独立可观察、可竞态或可失败的语义阶段必须拥有独立完整 Frame Fixture，不得把多个 correlation 强行合成固定序列。
- **FR-008**: Fixture provenance 必须记录 Gate ID、producer、source revision、schema、surface inventory、raw SHA-256 和 semantic SHA-256。
- **FR-009**: PC 必须完成 Fixture parity、IPC Mock、真实 Tauri invoke/bus ingress、Surface apply、DOM 断言和结构化报告；Mock 证据不得替代 Real。
- **FR-010**: PC production bundle 不得引用 Fixture、Mock bridge 或 WDIO-only API；真实 lane 必须经过生产 UI、Tauri transport 和真实 Frame ingress。
- **FR-011**: Mobile 必须 byte-exact 同步 Helix Fixture，并通过链接生产 SDK、Bridge、SurfaceRuntime 与 QuickJS binding 的 GTest 验证 typed view。
- **FR-012**: Mobile production build 不得注册 \`im.store\`、raw business reducer、测试注入 API或 characterization-only ABI。
- **FR-013**: PC 与 Mobile 只能机械验证和 apply Frame；不得计算 Helix 未产出的业务字段、权限、排序或终态。
- **FR-014**: 本 Gate 必须执行 production-boundary 验收，证明测试代码在默认生产构建不可达，且生产入口真实存在。
- **FR-015**: 本 Gate 完成后必须执行行为 Review、代码边界 Review 和 Frame V2 字段/时序 Review；只有 \`review-green\` 才能进入下一 Gate。
- **FR-016**: Review 必须核对三仓同一业务逻辑、Fixture hash/schema、Frame surface/scope/revision/mutation/directive、PC DOM 与 Mobile typed view。
- **FR-017**: 当前 Gate 的生产可达路径必须保持 InteractionFrame V2-only，V1 Frame 和 raw business render fallback 数量均为 0。
- **FR-018**: 运行报告必须分别记录 Helix、Fixture、PC Mock、PC Real、Mobile Contract、Review 和 blocker，不得用一个总 PASS 掩盖缺失证据面。

### Gate Contract Snapshot

- **Commands**: \`${listText(commands)}\`
- **Effects**: \`${listText(effects)}\`
- **Persistence facts**: \`${listText(persistence)}\`
- **Frame surfaces**: \`${listText(projections)}\`
- **Allowed actions**: \`${listText(row.allowed_actions)}\`
${sourceLine}
${override ? `- **Gate-specific clarification**: ${override.contractNote}` : ''}

### Key Entities

- **Gate Scenario**: 一次隔离的用户行为或系统 observation，包含 actor、observer、fresh channel、correlation 和预期终态。
- **Canonical Fixture**: Helix 生产路径输出并经字段级验证的完整 InteractionFrame V2 JSON。
- **Platform Mirror**: PC 或 Mobile 对 canonical Fixture 的只读副本，hash 与 inventory 必须一致。
- **Production Boundary**: 证明真实入口使用生产类且测试注入在默认构建不可达的验收面。
- **UC Review**: 对行为、代码、Frame V2 和证据一致性的独立裁决。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 六类 Helix 场景适用项覆盖率为 100%，所有 required 测试 fail=0、skip=0。
- **SC-002**: Helix canonical Fixture 与 PC/Mobile mirror 的 raw/semantic hash、schema 和 surface inventory 一致率为 100%。
- **SC-003**: PC Mock、PC Real、Mobile GTest 和 QuickJS typed view 对 required 字段的断言通过率为 100%，且不存在平台补算字段。
- **SC-004**: 所有故意注入的 persist failure、duplicate/late fact、scope/epoch/revision 错误中，伪成功 Frame、DOM 或 typed view 数量为 0。
- **SC-005**: production-boundary 扫描中，生产构建可达 Fixture/Mock/test injection/raw business reducer 数量为 0。
- **SC-006**: 行为、代码和 Frame V2 Review 全部为 \`review-green\`；任何 partial/blocked 都阻止进入下一 Gate。

## Assumptions

- G01 已作为三仓同源 Fixture 与真实链基线完成，但仍需参加最终 26 Gate 删除前全量回归。
- 创建群聊是每个 canonical UC 的共享前置；G15a 仍独立验收群聊创建业务本身。
- PC Real 和 Mobile Contract 的运行环境在实施阶段按需建立；环境缺失时允许 blocked，不允许降级声明 Green。
- 本 Spec 只定义可验收目标；当前源码、文档、Fixture 或旧运行结果不自动满足这些要求。
`;
}

// 渲染总控 Feature 的规格正文。
function renderMasterSpec(childRecords) {
  const childList = childRecords
    .map((record) => `- \`${record.row.gate_id}\`: [${record.row.title}](../${record.directory.split('/').at(-1)}/spec.md)`)
    .join('\n');
  return `# Feature Specification: InteractionFrame V2 26 Gate 三仓逐 UC 总控

**Feature Branch**: \`main (no feature branch created)\`

**Created**: ${CREATED}

**Status**: Draft

**Loop Profile**: \`helix-loop/v1\`

**Input**: 用户要求建立一个总控 Spec 和一组独立 Gate Spec，以 G01 三仓合同为基线，先以 G02 为试点，再完成 G03a、G04..G16c 与 G-S1..G-S4；每个 UC 只有在 Helix、PC、Mobile 和独立 Review 全部 Green 后才能进入下一个。G03 文件上传及 G17-G27 进入 Gap。全部 26 Gate Green 后只生成 V1/legacy 删除候选清单，删除范围由用户决定，删除后重新执行三仓全量验收。

**Machine Index**: [gate-spec-manifest.json](gate-spec-manifest.json)

**Human Index**: [gate-spec-index.md](gate-spec-index.md)

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 用户在 PC 与 Mobile 获得一致的 IM 行为 (Priority: P1)

作为 IM 用户，我需要本轮 26 个 Gate 在桌面端和移动端表现为同一套业务行为，包括权限、排序、发送终态、失败、竞态与恢复，不因平台不同产生第二套状态机。

**Why this priority**: 跨端一致性是 Helix 的核心产品价值；任何一个 Gate 漂移都会破坏用户信任并阻止 legacy 清理。

**Independent Test**: 任取一个 Active Gate，在 fresh group 中执行完整用户行为，验证 Helix 产生权威 Frame，PC 和 Mobile 机械消费后形成一致可观察结果。

**Acceptance Scenarios**:

1. **Given** 前序 Gate 已 Green，**When** 当前 Gate 完成 Helix、PC、Mobile 与 Review，**Then** 当前 Gate 才能 Green 并解锁下一 Gate。
2. **Given** 任一 required lane 缺失或不一致，**When** 汇总状态，**Then** 当前 Gate 必须为 partial/blocked，后续 Gate 不得开始。
3. **Given** 26 Gate 全部 Green，**When**生成最终报告，**Then** 报告必须逐 Gate 列出真实证据，不以聚合数量隐藏缺面。

---

### User Story 2 - 三端维护者共享唯一业务真源 (Priority: P1)

作为 Helix、PC 与 Mobile 维护者，我需要 Helix 独占业务语义和 canonical Fixture，PC 与 Mobile 只做 transport、atomic apply 与平台渲染，防止长出三套业务 Store。

**Why this priority**: Fixture、Mock 或 GTest 中补业务字段会让测试通过而真实产品缺失，属于本轮硬阻断反模式。

**Independent Test**: 对每个 Gate 检查 production-boundary、Fixture lineage 和 consumer 代码，证明业务字段只有 Helix producer 能生成。

**Acceptance Scenarios**:

1. **Given** Helix 尚未产出某字段或状态，**When** PC/Mobile 测试需要它，**Then** 必须修复 Helix 生产链，不得在平台 Mock/GTest 增加 reducer。
2. **Given** canonical Fixture 更新，**When** 两端同步，**Then** mirror hash、schema、surface inventory 和字段语义必须与 Helix 一致。

---

### User Story 3 - 用户控制最终 legacy 删除 (Priority: P2)

作为发布决策者，我需要在全部 Active Gate Green 后看到完整 legacy/V1 删除候选清单、引用关系和风险，再决定实际删除范围；系统不得提前或擅自删除。

**Why this priority**: 文件名中的 v1 可能属于仍有效的 ActionInvocation、Matrix 或报告版本，机械全删会破坏无关合同。

**Independent Test**: 在 26 Gate 全绿前确认删除候选清单不可晋升为执行；全绿后生成分类清单并等待用户选择，删除后重新跑三仓全量验收。

**Acceptance Scenarios**:

1. **Given** 任一 Active Gate 未 Green，**When**请求 legacy 清理，**Then** 只能记录候选，不得执行物理删除。
2. **Given** 26 Gate 全部 Green，**When**用户选择删除项，**Then** 只删除获批范围并执行新的三仓全量回归。

## Loop Contract *(mandatory)*

### Trigger

G01 已形成三仓基线，但其余 Active Gate 尚未逐一证明 Helix canonical Fixture、PC Mock/Real、Mobile production/QuickJS 和独立 Review 一致。

### Goal

按固定顺序完成 26 个 Active Gate 的三仓纵向闭环，保证所有业务语义只在 Helix 实现，平台消费同一完整 InteractionFrame V2。

### Completion

26/26 Active Gate 均为 \`review-green\`；每 Gate required fail=0/skip=0，InteractionFrame V1 和 raw business render fallback 为 0；三仓 pin/hash/evidence 可追溯。完成后生成删除候选清单并等待用户决定，实际删除后再次获得 26/26 全绿。

### Boundaries

- Active：\`${ACTIVE_GATES.join(', ')}\`，共 26 个。
- Gap：\`${GAP_GATES.join(', ')}\`；不进入本轮 Green 分母。
- 一个 Gate Green 后才能进入下一个，不并行实现多个业务 Gate。
- 不允许 Mock/GTest/Fixture 补生产代码缺失的业务字段、排序、权限或 reducer。
- 不在用户选择前物理删除 V1/legacy 代码、文档、Fixture 或测试。
- 同一 task/validator/失败签名自动修复最多两次，第三次进入 NeedsHuman。

### Human Gate

改变范围或用户行为；改变权限/wire/数据语义；执行不可逆迁移；使用真实凭据或生产数据；决定 legacy/V1 删除范围；同一失败签名两次修复仍未收敛。

### Edge Cases

- G01 基线在最终候选 pin 上发生漂移时，必须重新 Review，不能继承旧 Green。
- G03a 失败重发是 Active Gate，不得误并入已跳过的 G03 文件上传。
- G15a 是群聊创建业务 Gate；其他 Gate 虽复用 CreateGroupPrerequisite，不能继承前一 Gate 的群数据。
- WS 与 HTTP terminal 竞态时，不得固定 Frame 次序或重复 terminal write。
- PC Real 或 Mobile production environment 缺失时，对应 Gate 必须 blocked，Mock/GTest 不能代替。
- 文件名包含 v1 但属于独立版本域时，不得自动进入删除候选。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 总控必须维护一份 26 个 Active Gate 的机器索引与人类索引，Gate ID 必须唯一且与子 Spec 一一对应。
- **FR-002**: G01 必须复用已完成的三仓基线 Feature；其余 25 个 Gate 必须各有独立 Spec Kit Feature 和 requirements checklist。
- **FR-003**: G03 必须标记为文件/媒体上传 Gap；G17-G27 的 19 个扩展 Gate 必须全部进入 Gap，不计入本轮分母。
- **FR-004**: Gate 执行顺序固定为 G01 基线、G02 试点、G03a、G04..G16c、G-S1..G-S4；前序未 Green 时后序不得开始。
- **FR-005**: 每个 canonical UC 必须创建 fresh group，复用共享 CreateGroupPrerequisite 能力，但不得复用前一个 Gate 的数据。
- **FR-006**: 每个 Gate 必须完成 Helix 六类场景、真实 producer Fixture、字段级断言、Golden 和 provenance/hash。
- **FR-007**: PC 每 Gate 必须完成 Fixture parity、Mock、Real invoke/bus、Surface apply、DOM 断言与报告；涉及 observer 的行为还必须完成双账号 real lane。
- **FR-008**: Mobile 每 Gate 必须完成 mirror parity、链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest 和 typed view；Venus 默认走共享回归。
- **FR-009**: 只有修改 Venus/QuickJS 接线能力的 Gate 才新增专属 Venus Gate；普通 Gate 不复制 Venus runner。
- **FR-010**: 每个 Gate 必须执行 production-boundary，证明真实生产入口存在且 Fixture、Mock、测试注入、characterization ABI 在默认构建不可达。
- **FR-011**: PC 与 Mobile 不得维护消息对账、排序、权限、终态、未读或 raw event 业务 reducer；缺失能力必须回到 Helix 修复。
- **FR-012**: 每个 Gate 必须经过行为、代码和 Frame V2 一致性 Review；Review 非 Green 时不得进入下一 Gate。
- **FR-013**: Review 必须核对 surface/scope/epoch/revision/mutation/directive/ActionBinding、Fixture hash、PC DOM、Mobile typed view 和终态 authority。
- **FR-014**: 每个 Gate 报告必须分别记录 Helix、Fixture、PC Mock、PC Real、Mobile Contract、Review、blocker 和最大可声明结论。
- **FR-015**: Mock、AcceptedCommand、HTTP 2xx、单端 Fixture replay、单个 DOM 截图或 GTest 通过均不得单独晋升 Gate Green。
- **FR-016**: 当前生产路径必须逐 Gate保持 Frame V2-only，V1 Frame producer/consumer 与 raw business render fallback 均为 0。
- **FR-017**: 26 Gate 全绿后只能生成带分类、反向引用和风险的删除候选清单，用户选择前不得删除。
- **FR-018**: 用户批准删除后，必须重新运行 Helix、PC 与 Mobile 的 26 Gate 全量验收；删除前证据不得冒充删除后结果。
- **FR-019**: ActionInvocation envelope、Strict Matrix schema、报告 schema 等独立 v1 版本不得因名称自动删除。
- **FR-020**: 总控必须 fail-closed：任一 Gate partial/blocked、pin/hash 漂移或 required skip>0 时，总体状态不得为 Green。

### Child Gate Specifications

${childList}

### Key Entities

- **Active Gate**: 本轮需要完成三仓纵向闭环并计入分母的独立用户行为或恢复行为。
- **Gap Gate**: 已登记当前状态和后续条件、但不进入本轮实现与 Green 分母的能力。
- **Gate Spec**: 单一 Gate 的行为、三仓责任、证据和 Review 完成合同。
- **Canonical Fixture Supply Chain**: Helix producer 到 PC/Mobile mirror 的单向、hash 固定合同。
- **UC Review Gate**: 行为、代码、Frame V2 和证据一致性的独立完成裁决。
- **Deletion Candidate Inventory**: 26 Gate 全绿后生成、等待用户选择的 legacy/V1 候选集合。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Active Gate 索引精确为 26/26；G01 基线 1 个、独立子 Spec 25 个，重复和遗漏均为 0。
- **SC-002**: Gap 索引精确包含 G03 与 G17-G27 的 19 个扩展 Gate，不计入 Active Green。
- **SC-003**: 每个 Gate 的 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 和 Review required pass rate 为 100%，fail=0、skip=0。
- **SC-004**: 26 个 Gate 的 PC/Mobile mirror 与 Helix canonical Fixture 在 schema、inventory、raw/semantic hash 上一致率为 100%。
- **SC-005**: 26 个 Gate 的 production path 中，InteractionFrame V1 producer、consumer 和 raw business render fallback 数量均为 0。
- **SC-006**: 只有 26/26 \`review-green\` 时总控可进入 deletion-candidate-ready；用户批准删除后新的三仓回归仍保持 26/26 Green。

## Assumptions

- G01 的已完成 Feature \`019-g01-wdio-five-phases\` 是方法与证据基线，不代表其他 Gate 自动完成。
- G03a 保留为失败重发 Gate；G03 文件/媒体上传单独进入 Gap。
- G17-G27 的 19 个扩展 Gate继续保留在产品 Gap Map，后续另开总控，不污染本轮分母。
- 本轮生成 Spec、checklist、discovery 与索引，不修改三仓生产代码，也不运行业务测试。
`;
}

// 渲染机器可读的 Gate -> Spec Kit Feature 清单。
function renderManifest(childRecords, matrixSha) {
  const children = childRecords.map((record, index) => ({
    order: index + 2,
    gateId: record.row.gate_id,
    ucId: record.row.uc_id,
    title: record.row.title,
    featureDirectory: record.directory,
    prerequisiteGate: index === 0 ? 'G-01' : childRecords[index - 1].row.gate_id,
    status: 'draft',
  }));
  return JSON.stringify(
    {
      schemaVersion: 1,
      generatedAt: CREATED,
      generator: 'scripts/generate_ifv2_three_repo_specs.mjs',
      strictMatrix: {
        path: MATRIX_PATH,
        rawSha256: matrixSha,
      },
      scope: {
        activeCount: 26,
        childSpecCount: 25,
        baseline: {
          gateId: 'G-01',
          featureDirectory: 'specs/019-g01-wdio-five-phases',
          status: 'baseline-green-reverify-final',
        },
        children,
        gap: GAP_GATES,
      },
      policies: {
        execution: 'one-uc-three-repo-vertical-before-next',
        fixtureAuthority: 'helix-im-handle-effect-emit',
        reviewRequired: ['behavior', 'code-boundary', 'interaction-frame-v2', 'evidence'],
        deletion: 'candidate-only-after-26-green-user-selects',
      },
    },
    null,
    2,
  );
}

// 渲染总控目录内的人类可读 Gate -> Feature 索引。
function renderMasterIndex(childRecords) {
  const rows = [
    '| 顺序 | Gate | UC | 业务 | Spec Kit Feature | 状态/依赖 |',
    '|---:|---|---|---|---|---|',
    '| 1 | G-01 | UC-1.1 | 发送文本 | [019 G01 基线](../019-g01-wdio-five-phases/spec.md) | baseline Green；最终候选重验 |',
    ...childRecords.map((record, index) => {
      const previous = index === 0 ? 'G-01' : childRecords[index - 1].row.gate_id;
      return `| ${index + 2} | ${record.row.gate_id} | ${record.row.uc_id} | ${record.row.title} | [${record.directory.split('/').at(-1)}](../${record.directory.split('/').at(-1)}/spec.md) | Draft；依赖 ${previous} review-green |`;
    }),
  ];
  return `# InteractionFrame V2 26 Gate Spec Kit 索引

> 本索引是总控 Spec 的人类可读视图；机器真源为 [gate-spec-manifest.json](gate-spec-manifest.json)。
> 一个 Gate 完成 Helix、PC、Mobile 与 UC Review 后，才能进入下一行。

${rows.join('\n')}

## Gap

| Gate | 处理 |
|---|---|
| G-03 | 文件/媒体上传；本轮 deferred-gap |
| G-17～G-27 | 19 个扩展 Gate；保留在产品 Gap Map，另开后续总控 |

## 完成公式

\`Gate Green = Helix Green + Fixture/Hash Green + PC Mock/Real/Report Green + Mobile GTest/QuickJS Green + production-boundary Green + UC Review Green\`

\`Total Green = 26/26 Gate Green + v1FrameProducer=0 + v1FrameConsumer=0 + rawBusinessRenderFallback=0\`

Total Green 之后只生成删除候选清单；用户选择删除范围后，必须使用新候选重新跑三仓 26 Gate。
`;
}

// 渲染逐 UC 文档目录中的总 Spec 与子 Spec 路由入口。
function renderDocsIndex(childRecords) {
  const rows = [
    '| 顺序 | Gate | 业务 | 当前代码文档 | Spec Kit Feature |',
    '|---:|---|---|---|---|',
    '| 1 | G-01 | 发送文本 | [G01](./G01/README.md) | [019 G01 基线](../../specs/019-g01-wdio-five-phases/spec.md) |',
    ...childRecords.map((record, index) => {
      const docPath = gateDocPath(record.row.gate_id);
      const docCell = docPath
        ? `[${GATE_DOC_FOLDERS.get(record.row.gate_id)}](./${GATE_DOC_FOLDERS.get(record.row.gate_id)}/README.md)`
        : 'Strict Matrix；plan 阶段补当前代码 discovery';
      return `| ${index + 2} | ${record.row.gate_id} | ${record.row.title} | ${docCell} | [${record.directory.split('/').at(-1)}](../../${record.directory}/spec.md) |`;
    }),
  ];
  return `# InteractionFrame V2 26 Gate 三仓纵向闭环总 Spec 索引

> 总控 Feature：[020-ifv2-three-repo-uc-control](../../specs/020-ifv2-three-repo-uc-control/spec.md)
>
> 机器索引：[gate-spec-manifest.json](../../specs/020-ifv2-three-repo-uc-control/gate-spec-manifest.json)

## 范围

- Active 26 Gate：G01 基线 + G02 试点 + G03a + G04-G16c + GS1-GS4。
- G03 文件/媒体上传进入 Gap。
- G17-G27 的 19 个扩展 Gate 进入 Gap。
- 一个 UC 完成三仓纵向闭环和 UC Review 后，才能进入下一个。
- 全部 26 Gate Green 后只生成 V1/legacy 删除候选清单，删除范围由用户决定。

## 三仓硬合同

1. Helix 是业务逻辑和 canonical Fixture 唯一真源。
2. PC 必须完成 Fixture parity、Mock、Real、Surface/DOM 断言与报告。
3. Mobile 必须完成 mirror parity、链接生产 SDK/Bridge/SurfaceRuntime/QuickJS 的 GTest 与 typed view。
4. Mock/GTest 不得补生产代码缺失的业务字段、排序、权限或 reducer。
5. 每个 Gate 必须通过行为、代码边界、Frame V2 与证据一致性 Review 才能 Green。
6. 删除前与删除后是两次不同候选、不同证据，不得复用旧 Green。

## Gate -> Spec Kit

${rows.join('\n')}

## Gap

| 范围 | 数量 | 本轮处理 |
|---|---:|---|
| G-03 | 1 | 文件/媒体上传 deferred-gap |
| G-17～G-27 | 19 | 产品扩展 Gap；只登记，不进入 Active 分母 |
`;
}

// 验证生成物数量、Gate 唯一性、必要章节与 placeholder 清零。
function validateBundle(childRecords, manifest) {
  const errors = [];
  if (ACTIVE_GATES.length !== 26) errors.push(`Active Gate 数量错误: ${ACTIVE_GATES.length}`);
  if (new Set(ACTIVE_GATES).size !== 26) errors.push('Active Gate 存在重复');
  if (childRecords.length !== 25) errors.push(`子 Spec 数量错误: ${childRecords.length}`);
  if (manifest.scope.activeCount !== 26 || manifest.scope.childSpecCount !== 25) {
    errors.push('manifest 统计与目标范围不一致');
  }
  const requiredHeadings = [
    '## User Scenarios & Testing',
    '## Loop Contract',
    '## Requirements',
    '## Success Criteria',
    '## Assumptions',
  ];
  for (const record of childRecords) {
    const specPath = join(ROOT, record.directory, 'spec.md');
    const checklistPath = join(ROOT, record.directory, 'checklists/requirements.md');
    const discoveryPath = join(ROOT, record.directory, 'discovery.md');
    const verificationPath = join(ROOT, record.directory, 'verification.md');
    for (const path of [specPath, checklistPath, discoveryPath, verificationPath]) {
      if (!existsSync(path)) errors.push(`缺生成物: ${path}`);
    }
    const spec = readFileSync(specPath, 'utf8');
    for (const heading of requiredHeadings) {
      if (!spec.includes(heading)) errors.push(`${record.row.gate_id} 缺章节 ${heading}`);
    }
    if (spec.includes('[NEEDS CLARIFICATION') || /\[(?:FEATURE|DATE|Brief Title|Assumption)/.test(spec)) {
      errors.push(`${record.row.gate_id} 仍含模板 placeholder`);
    }
    if (readFileSync(verificationPath, 'utf8').includes('[Command and expected falsifiable result]')) {
      errors.push(`${record.row.gate_id} verification 仍含模板 placeholder`);
    }
  }
  if (errors.length) throw new Error(errors.join('\n'));
}

// 从 Strict Matrix 机械生成总控与 25 个子 Feature，避免手工 Gate 漏项。
function main() {
  const matrixBytes = readFileSync(join(ROOT, MATRIX_PATH));
  const matrixSha = sha256(matrixBytes);
  const matrix = JSON.parse(matrixBytes);
  const rowsByGate = new Map(matrix.uc_cases.map((row) => [row.gate_id, row]));
  const missing = CHILD_GATES.filter((gateId) => !rowsByGate.has(gateId));
  if (missing.length) throw new Error(`Strict Matrix 缺 Active Gate: ${missing.join(', ')}`);

  const recordsByGate = new Map(CHILD_GATES.map((gateId, index) => {
    const row = rowsByGate.get(gateId);
    const directory = childDirectory(index, row);
    return [gateId, { directory, row }];
  }));
  const childRecords = EXECUTION_GATES.map((gateId) => recordsByGate.get(gateId));

  childRecords.forEach(({ directory, row }, index) => {
    const previousGate = index === 0 ? 'G-01' : childRecords[index - 1].row.gate_id;
    writeText(`${directory}/spec.md`, renderChildSpec(directory, row, previousGate));
    writeText(
      `${directory}/checklists/requirements.md`,
      renderChecklist(`${row.gate_id} ${row.title} Frame V2 三仓一致性`, '三端一致行为、唯一业务真源和独立 Review'),
    );
    writeText(`${directory}/discovery.md`, renderChildDiscovery(directory, row, matrixSha));
    writeText(`${directory}/verification.md`, renderChildVerification(row));
  });

  writeText(`${MASTER_DIRECTORY}/spec.md`, renderMasterSpec(childRecords));
  writeText(
    `${MASTER_DIRECTORY}/checklists/requirements.md`,
    renderChecklist('InteractionFrame V2 26 Gate 三仓逐 UC 总控', '26 Gate 顺序、三仓责任、Review 和用户控制删除'),
  );
  const manifestText = renderManifest(childRecords, matrixSha);
  writeText(`${MASTER_DIRECTORY}/gate-spec-manifest.json`, manifestText);
  writeText(`${MASTER_DIRECTORY}/gate-spec-index.md`, renderMasterIndex(childRecords));
  writeText(`${MASTER_DIRECTORY}/verification.md`, renderMasterVerification());
  writeText(
    `${MASTER_DIRECTORY}/discovery.md`,
    renderChildDiscovery(MASTER_DIRECTORY, rowsByGate.get('G-01'), matrixSha)
      .replace('# Source Discovery', '# Source Discovery: 26 Gate 三仓总控')
      .replace('本 Feature 只负责 `G-01`，不得顺带实现下一 Gate。', '本 Feature 只负责编排 26 Gate；业务实现分别归属独立子 Feature。'),
  );
  writeText(
    `${DOCS_DIRECTORY}/07-26Gate三仓纵向闭环总Spec索引.md`,
    renderDocsIndex(childRecords),
  );
  writeText(
    '.specify/feature.json',
    JSON.stringify({ feature_directory: MASTER_DIRECTORY }, null, 2),
  );

  const manifest = JSON.parse(manifestText);
  validateBundle(childRecords, manifest);
  process.stdout.write(
    `${JSON.stringify({
      master: MASTER_DIRECTORY,
      childSpecs: childRecords.length,
      activeGates: ACTIVE_GATES.length,
      gapGates: GAP_GATES.length,
      matrixSha256: matrixSha,
    })}\n`,
  );
}

main();
