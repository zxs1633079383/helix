# Feature Specification: InteractionFrame V2 双仓硬切

**Feature Branch**: `main (no feature branch created)`

**Created**: 2026-07-22

**Status**: Draft

**Loop Profile**: `helix-loop/v1`

**Input**: 用户要求按照“客户端 V2 reader 就绪 → Helix 全量 V2 writer → 更新 pin → 切断 Helix V1 produce 与 cses-client V1 consume → 纯 V2 全量 E2E → 删除兼容代码 → 删除后二次 E2E”的顺序，梳理 Helix 与 cses-client 当前状态，并为 GS1-GS4、G01-G02、G04-G16 建立从 ActionBinding/HostObservation、Tick、Effect、Persist、InteractionFrame V2 到 Angular 组件和逻辑处理的细粒度 Strict Matrix。

**Matrix**: [strict-matrix.md](strict-matrix.md)

**Current Source Discovery**: [discovery.md](discovery.md)

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 用户在协议切换后继续完成全部 IM 交互 (Priority: P1)

作为桌面 IM 用户，我需要重连、收发消息、撤回、转发、回应、加急、定时、补历史、定位、回复、已读、建群、建话题和频道治理在协议切换后保持原有业务结果，不出现空白、重复、旧状态覆盖或按钮失效。

**Why this priority**: Frame 协议属于所有目标 UC 的共同下行边界；任何一个 Gate 迁移不完整都会直接破坏用户可见交互。

**Independent Test**: 在一个固定双用户 Native candidate 中连续执行目标 Gate，验证每个动作、持久化结果、最终交互状态与对端观察均正确，并确认整个运行没有接收 InteractionFrame V1。

**Acceptance Scenarios**:

1. **Given** 用户打开已附着的消息 Surface，**When** 执行目标 Gate 的有效操作，**Then** 操作必须经当前 ActionBinding 或明确 HostObservation 进入 Helix，并只在权威事实持久化成功后形成用户可见结果。
2. **Given** 一次 Frame 同时改变多个 Surface，**When** 客户端接收该 Frame，**Then** 所有 mutation 必须作为一个事务被接受或全部拒绝，directive 只在事务提交后执行。
3. **Given** Frame 的 schema、scope、epoch、revision 或内容不合法，**When** 客户端验证它，**Then** 当前一致画面必须保留，并发起限定 Surface 的恢复，不得使用原始事件补算业务状态。

---

### User Story 2 - 开发者只维护一套 UI 业务协议 (Priority: P1)

作为 Helix 与 cses-client 维护者，我需要所有目标 Gate 只使用 InteractionFrame V2 作为 UI 业务真源，使新增 UC 不再复制 V1 producer、V1 decoder 或 `*.row.v1` RenderNode 合同。

**Why this priority**: V1/V2 并存会让每次新增字段、Surface 或 Action 都产生双分支，并使 Native 绿灯无法证明 V2 已经生效。

**Independent Test**: 对目标范围执行静态清单、合同生成和跨仓 fixture parity，证明所有 writer 输出 V2、严格 candidate 不接受 V1，且删除兼容代码后不存在可达 V1 producer/consumer。

**Acceptance Scenarios**:

1. **Given** 客户端 V2 reader 已能验证并原子应用 V2，**When** Helix 逐 Gate 切换 writer，**Then** 每个目标 Gate 的成功路径、失败路径和恢复路径均必须产生明确的 V2 合同或明确不产生业务 Frame。
2. **Given** 全量 writer 已切到 V2，**When** 更新 cses-client 的 Helix pin，**Then** runtime pin、生成合同 pin、fixture manifest pin 与证据 pin 必须一致。
3. **Given** 固定 pin 已切断 V1 produce/consume，**When** 全量 E2E 在无 V1 fallback 的候选上证明 V1 数量为零，**Then** 才允许物理删除兼容代码，且删除后的回归必须继续保持全部目标 Gate 正常。

---

### User Story 3 - 验证者能定位任一 Gate 的第一处偏离 (Priority: P2)

作为验证者，我需要按同一 correlation 将 Angular 操作、ActionBinding、Tick、Effect、Persist、Frame、Surface apply、组件 DOM 与双用户结果串联起来，以便区分业务失败、协议失败和 UI 失败。

**Why this priority**: 只有端到端相关证据才能防止把 HTTP 受理、旧 V1 兼容渲染或单张截图误判为 V2 完成。

**Independent Test**: 任取一个目标 Gate，使用 Strict Matrix 指定的 correlation 字段和负向断言，能够指出链路第一处缺失或不一致，并将结果判为 PASS、FAIL 或 BLOCKED。

**Acceptance Scenarios**:

1. **Given** 任一 Gate 被标记 PASS，**When** 审查其证据，**Then** 必须能从输入一直追踪到 matching V2 Frame 和 Angular 组件结果。
2. **Given** Persist 失败、revision gap 或 actionRef 过期，**When** 执行负向场景，**Then** 验证必须证明没有伪造的成功 UI，且失败发生在 Strict Matrix 允许的位置。
3. **Given** 某证据面不可用，**When** 汇总 Gate 状态，**Then** 结果必须是 BLOCKED 或 INCONCLUSIVE，不得继承旧运行的 Green。

## Loop Contract *(mandatory)*

### Trigger

当前双仓仍存在 InteractionFrame V1 writer、V1 reader/normalize、RenderNode/v1 Strict Matrix，以及无法证明 `schemaVersion=2` 和 V1 数量为零的旧聚合证据。

### Goal

目标范围内所有 UI 业务结果统一由 InteractionFrame V2 表达和消费；每个机器 Gate 都有从输入到 Angular 组件的细粒度合同、可证伪证据和删除后回归。

### Completion

25 个机器 Gate 在同一固定 pin 上全部通过；运行中的 InteractionFrame V1 为 0、raw business reducer fallback 为 0；V1 writer、reader、normalize、fixture 和相关兼容分支完成删除；删除后的合同测试、Angular 测试和聚合 Native 回归通过。

### Boundaries

- 范围包括 `G-S1` 至 `G-S4`、`G-01`、`G-02`、`G-04/G-04a`、`G-05` 至 `G-16/G-16b/G-16c`。
- 明确排除 `G-03` 和 `G-03a`；其他 `G-17` 至 `G-27` 不在本 Feature。
- 不把 `ActionInvocationEnvelope` 自身的版本、Strict Matrix 文件格式版本或其他独立 API 的 `v1` 字样自动升级；只有经盘点确认属于 InteractionFrame/RenderNode V1 的对象才清退。
- 不直接修改远端 PostgreSQL 或本地 SQLite 制造验收结果，不进行与 Frame 硬切无关的数据迁移。
- 同一 task/validator/失败签名最多自动修复两次，第三次进入 NeedsHuman。

### Human Gate

以下情况需要人工批准：改变已拍板的 UC 业务语义；执行不可逆数据库迁移；扩大凭据或生产数据访问；删除盘点范围之外的版本化协议；同一失败签名两次修复仍未收敛。

### Edge Cases

- V2 reader 可以解析 V2，但仍静默接受 V1 时，不得把“reader ready”写成“cutover complete”。
- Helix HEAD、cses-client Cargo pin、生成 Strict Matrix pin 或 fixture source pin 不一致时，禁止运行最终签收。
- 一笔 V2 Frame 中任一 mutation 非法时，整帧不得部分提交，directive 不得执行。
- V2 Frame 只有 runtime 更新而没有已附着业务 Surface 时，客户端仍必须正确应用运行态；不得制造空业务行。
- 未附着 Surface 不强制创建；其权威事实仍应持久化，并在以后 attach 时通过 V2 snapshot 恢复。
- actionRef 的 actor、scope、surfaceEpoch 或 observedRevision 失效时必须拒绝，组件不得改走命名业务命令绕过 binding。
- 删除 V1 兼容代码后发现遗漏 Gate 时，只修复该 Gate 的 V2 producer/consumer，不恢复 V1 或 raw fallback。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 系统必须维护一份覆盖 19 个业务 UC、25 个机器 Gate 的 Strict Matrix；每行必须包含输入、ActionBinding/HostObservation、Tick、Effect、Persist barrier、Surface/VM、Frame、Angular 组件、逻辑处理、负向断言和证据。
- **FR-002**: 客户端 V2 reader 就绪必须由合法 V2、未知 schema、错误 scope、错误 epoch、revision gap、重复 transaction、整帧原子性和 directive-after-commit 测试共同证明。
- **FR-003**: 迁移源码阶段可以暂存双读代码，但进入任何全量 E2E candidate 前，客户端生产可达路径必须只接受 V2；目标 Gate 的 V1 Frame 必须 fail-closed，禁止 normalize 或 fallback。
- **FR-004**: Helix 必须为所有目标 Gate 使用唯一的 V2 Frame builder/emitter，不得继续通过默认 V1 builder 形成业务下行。
- **FR-005**: 每个用户动作必须使用 Helix 投影的 opaque ActionBinding；组件只提交 actionRef 与合同允许的 input，不得选择业务命令、actor、channel、message identity 或权限结果。
- **FR-006**: 无用户动作的同步、WS、timer 和 port reply 路径必须在 Matrix 中声明明确 HostObservation/Inbound 输入，不得伪造 ActionBinding。
- **FR-007**: 每个输入必须归一为确定的 Tick；每个外部 I/O 必须表现为 Effect，并通过相关 PortReply 返回事实。
- **FR-008**: 声称 durable 业务状态的 Frame 必须以 matching PersistOk 为前置；HTTP 2xx、WS 入队、同步结束信号或内存 mutation 均不能独立发布最终业务 Frame。
- **FR-009**: 每个 V2 Frame 必须携带有效 `schemaVersion=2`、transactionId、runtime、SurfaceMutationV2 和 directives；Action 结果必须携带可关联的 causation。
- **FR-010**: 每个 SurfaceMutationV2 必须具有匹配 Surface 的 scope、正整数 surfaceEpoch、单调 revision、合法 baseRevision 和完整 typed VM/row。
- **FR-011**: Angular 必须只通过 V2 codec、SurfaceRuntime 原子 apply、九 Surface adapter/facade 和组件输入消费业务状态；raw `im:*` 事件不得写入业务 VM。
- **FR-012**: Angular 必须在整帧提交后更新 action 状态、执行 directive 并发送 FrameApplied；失败、重复、未匹配或要求 resync 的 Frame 不得伪造已提交 ack。
- **FR-013**: 每个 Gate 必须明确组件 owner 和逻辑 handler；组件发起动作时必须能追溯到 SurfaceActionRequested 与 opaque action invocation。
- **FR-014**: Matrix 必须区分 CURRENT、TARGET 和 GAP；文档目标、测试 fixture、旧 handoff 或 HTTP 成功不得写成当前 runtime V2 证据。
- **FR-015**: 更新 pin 前必须冻结 Helix revision、Strict Matrix semantic hash、fixture manifest hash 和客户端生成合同，并证明四者来自同一候选。
- **FR-016**: 更新 pin 后的全量 E2E 必须在一个 Native candidate、两个隔离用户和一次连续运行中执行所有 25 个机器 Gate，不按 Gate 重编 Native。
- **FR-017**: 全量 E2E 必须显式统计 Frame schema；完成条件包含 `v2_frames > 0`、`v1_frames = 0`、`raw_business_fallback = 0`，而不仅是业务 UI Green。
- **FR-018**: 每个 Gate 必须关联 actor/observer、Tick/Effect、远端权威事实、本地 Persist、V2 Frame、Surface apply 和 DOM/交互证据；缺任一必需面即不能 PASS。
- **FR-019**: V1 禁收必须在首次全量 E2E 之前完成；固定 pin 的生产 reader 对未知或 V1 schema 必须 fail-closed 并进入受控诊断/恢复。
- **FR-020**: 删除清单必须覆盖 Helix V1 producer/builder/capability、Angular V1 type/codec/normalize、V1 fixtures/tests 和已确认属于旧渲染协议的 RenderNode/v1 合同。
- **FR-021**: 删除动作不得误删独立版本合同，包括仍有效的 ActionInvocation envelope 版本、Strict Matrix 文档 schema 版本和非 Frame API 版本。
- **FR-022**: 首次纯 V2 E2E 只作为物理删除准入；删除兼容代码之后必须重新执行跨仓 fixture parity、Helix focused/full tests、Angular codec/runtime/component tests 和聚合 Native 回归。
- **FR-023**: 既有 004-017 FULL GREEN 只作为业务功能基线；除非证据明确断言目标 Frame 全为 V2，否则不得继承为本 Feature 的 V2 PASS。
- **FR-024**: 所有 hop 必须携带可对齐的 runId、gateId、requestId/operationId、causationId、transactionId、surface/scope、persist operation id 和 FrameApplied ack。
- **FR-025**: 迁移过程不得恢复 raw reducer、producer 双写或客户端业务重算；发现缺口时必须修复对应 V2 链路。

### Key Entities

- **Gate Contract Row**: 一个机器 Gate 的完整输入、状态转换、持久化、Frame、组件和证据合同。
- **ActionBinding**: Helix 投影出的 opaque、actor/scope/epoch/revision 绑定能力；组件不能自行构造其业务权限。
- **InteractionFrame V2**: 一次原子 UI 业务事务，包含 runtime、带 surfaceEpoch 的 mutations、causation 和 commit 后 directives。
- **Surface Slot**: 由 SurfaceId 与 scope 唯一标识的客户端 Frame Cache 槽，具有 epoch、revision 和状态。
- **Cutover Candidate**: Helix revision、客户端 pin、生成合同和 fixture hash 全部冻结的一次候选。
- **Deletion Inventory**: 已证明被 V2 替代、允许物理删除的 V1 producer、consumer、fixture 和文档引用集合。
- **Evidence Window**: 同一 candidate、双用户和有界时间范围内生成的跨 hop 证据集合。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Strict Matrix 对目标范围的覆盖率为 25/25，每行必需字段完整率为 100%，G03/G03a 出现次数为 0。
- **SC-002**: 固定 pin 的聚合运行中，25/25 Gate 业务结果正确，required fail=0、required skip=0。
- **SC-003**: 聚合运行的目标业务 Frame 中，100% 为 schemaVersion 2；InteractionFrame V1 数量和 raw business fallback 数量均为 0。
- **SC-004**: 100% 的 action Gate 能从组件 actionRef 追踪至 Tick、Effect、PersistOk、matching causation Frame 和组件终态；100% 的 observation Gate 能从 inbound/host observation 追踪至同等终态。
- **SC-005**: 所有故意注入的 Persist 失败、错误 epoch/revision、重复 transaction 和未知 schema 场景中，错误成功 UI 数量为 0。
- **SC-006**: Helix revision、客户端 Cargo pin、生成 Strict Matrix pin、fixture manifest pin 和最终证据 pin 一致率为 100%。
- **SC-007**: V1 删除清单完成率为 100%，删除后静态扫描的可达 V1 Frame producer/consumer 为 0，且删除后聚合回归保持 25/25 PASS。

## Assumptions

- “双仓”指 `/System/Volumes/Data/workspace/rust/helix` 与 `/System/Volumes/Data/workspace/angular/cses-client`；本 Feature 的 UI 完成面是 Native/Tauri 上的 Angular message-v4。
- 目标范围按用户此前约束解释为 GS1-GS4、G01-G02、G04-G16，并排除 G03 与 G03a；已有子 Gate 04a、10a、14a、15a、16b、16c 必须保留。
- Go 服务端和远端 PostgreSQL 作为权威事实与 E2E 依赖存在，但除非 V2 迁移暴露真实 wire/authority 缺陷，本 Feature 不主动修改 Go 业务语义。
- 现有业务数据库模型可以继续使用；本 Feature 不以数据库 schema 迁移作为默认方案。
- 旧 017 聚合结果证明过一版固定 pin 的业务交互可用，但其证据未证明所有业务 Frame 均为 schemaVersion 2，因此需要新的 V2 cutover 验收。
