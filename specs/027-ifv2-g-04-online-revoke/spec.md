# Feature Specification: G-04 撤回 Frame V2 三仓一致性

**Feature Branch**: `main (no feature branch created)`

**Created**: 2026-07-25

**Status**: Draft

**Loop Profile**: `helix-loop/v1`

**Parent Feature**: [020 三仓逐 UC 总控](../020-ifv2-three-repo-uc-control/spec.md)

**Input**: 用户要求按一个 UC 一个三仓纵向闭环完成 `G-04`：Helix 生产唯一业务结果与 canonical Fixture，PC 完成 Mock/Real/断言/报告，Mobile 完成 GTest、生产接线与 QuickJS typed view；行为、代码和 Frame V2 Review 通过后才能进入下一 UC。

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 用户在三端获得一致的“撤回”结果 (Priority: P1)

作为 IM 用户，我需要在桌面端和移动端执行或观察“撤回”时得到同一业务结果、权限、顺序和失败语义，不因平台不同出现第二套状态处理。

**Why this priority**: 该 Gate 是 `UC-1.5` 的独立用户行为；任何端自行推导都会造成可见状态、动作能力或恢复结果漂移。

**Independent Test**: 创建隔离群聊并执行 `G-04`，验证相同权威事实最终在 PC 与 Mobile 形成字段一致的 typed view，且失败与竞态场景不产生伪成功。

**Acceptance Scenarios**:

1. **Given** 当前 actor、observer、群聊与权限满足 Gate 前置，**When** 用户执行或系统触发“撤回”，**Then** Helix 必须先形成权威业务结果，PC 与 Mobile 只消费其完整 Frame。
2. **Given** transport 已受理但权威事实或持久化尚未完成，**When** 平台收到中间结果，**Then** 两端均不得显示最终成功。
3. **Given** 权威失败、权限拒绝、重复事实或迟到回包，**When** 三端处理结束，**Then** 两端保持同一失败/既有状态，不出现重复 mutation。

---

### User Story 2 - 开发者只维护一套 Gate 业务逻辑 (Priority: P1)

作为跨端维护者，我需要 `G-04` 的排序、权限、对账、终态和 Frame 字段只在 Helix 定义一次，使 PC 与 Mobile 不再维护独立 reducer 或手写测试业务字段。

**Why this priority**: 三端状态 Store 会让同一 UC 在修复、重试和恢复时产生无法审计的差异。

**Independent Test**: 从 Helix 生产回放生成 canonical JSON，分别同步到 PC 与 Mobile，证明 mirror hash、schema、surface inventory 和 typed view 一致。

**Acceptance Scenarios**:

1. **Given** Helix canonical Fixture 更新，**When** PC 与 Mobile 同步合同，**Then** 两端只能进行 provenance 允许的动态替换，不得补业务字段。
2. **Given** Fixture、Mock 或 GTest 中出现生产代码未产出的业务字段或 reducer，**When**执行 production-boundary Gate，**Then** 本 Feature 必须失败。

---

### User Story 3 - Reviewer 能判定 Gate 是否真正 Green (Priority: P2)

作为独立 Reviewer，我需要分别审查行为、代码边界、Frame V2 字段和真实证据，避免把 accepted、Mock 或单端渲染冒充三仓完成。

**Why this priority**: 本项目把证据边界视为正确性；缺少任一生产面就不能宣称完成。

**Independent Test**: 使用本 Spec 的六类场景与 Review 清单，能够给出 `review-green`、`review-partial` 或 `review-blocked`，并指出第一处偏离。

**Acceptance Scenarios**:

1. **Given** Helix、PC 或 Mobile 任一 required lane 缺失，**When** Reviewer 汇总，**Then** 状态不得为 Green。
2. **Given** 三仓 required lane 全部通过，**When** 行为、代码和 Frame 字段对账一致，**Then** 才能标记本 Gate Green 并进入下一 Gate。

## Loop Contract *(mandatory)*

### Trigger

`G-04` 的用户 intent、HostObservation、WS/inbound 或恢复事实进入当前业务链；canonical command 为 `im_revoke`。

### Goal

用户在 PC 与 Mobile 获得同一“撤回”业务结果，平台不重建权限、排序、对账、终态或失败语义。

### Completion

Helix 六类场景、canonical Fixture/Golden、PC Mock/Real/Report、Mobile mirror/GTest/QuickJS typed view、production-boundary 和独立 Review 全部通过；同一运行能够关联 command/inbound、persist、Frame、FrameApplied 与可观察结果。

### Boundaries

- 只处理 `G-04`；前序 Gate `G-03a` 未 Green 时不得实施本 Gate。
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
- Gate-specific: UI 不得自行替换撤回文案或生命周期

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 每次 canonical 验收必须创建新的隔离群聊，复用共享 CreateGroupPrerequisite 合同，不依赖前一个 Gate 的遗留数据。
- **FR-002**: 系统必须只接受当前 actor、scope、权限和版本边界内的合法 `G-04` intent 或 observation。
- **FR-003**: Helix 必须成为该 Gate 权限、排序、对账、终态、失败语义和完整 Frame 字段的唯一业务真源。
- **FR-004**: 所有声称 durable 的用户可见终态必须等待对应持久化成功；accepted、HTTP 2xx、WS 入队或内存更新均不得独立建立完成。
- **FR-005**: Helix 测试必须覆盖 prerequisites、happy path、business failure、infrastructure failure、race/idempotency、projection/recovery 六类场景；不适用项必须写明理由。
- **FR-006**: canonical Fixture 必须由生产 `ImModule::handle -> Effect::Emit` 路径生成，并执行 typed validation、字段级语义断言和 Golden 比较。
- **FR-007**: 每个独立可观察、可竞态或可失败的语义阶段必须拥有独立完整 Frame Fixture，不得把多个 correlation 强行合成固定序列。
- **FR-008**: Fixture provenance 必须记录 Gate ID、producer、source revision、schema、surface inventory、raw SHA-256 和 semantic SHA-256。
- **FR-009**: PC 必须完成 Fixture parity、IPC Mock、真实 Tauri invoke/bus ingress、Surface apply、DOM 断言和结构化报告；Mock 证据不得替代 Real。
- **FR-010**: PC production bundle 不得引用 Fixture、Mock bridge 或 WDIO-only API；真实 lane 必须经过生产 UI、Tauri transport 和真实 Frame ingress。
- **FR-011**: Mobile 必须 byte-exact 同步 Helix Fixture，并通过链接生产 SDK、Bridge、SurfaceRuntime 与 QuickJS binding 的 GTest 验证 typed view。
- **FR-012**: Mobile production build 不得注册 `im.store`、raw business reducer、测试注入 API或 characterization-only ABI。
- **FR-013**: PC 与 Mobile 只能机械验证和 apply Frame；不得计算 Helix 未产出的业务字段、权限、排序或终态。
- **FR-014**: 本 Gate 必须执行 production-boundary 验收，证明测试代码在默认生产构建不可达，且生产入口真实存在。
- **FR-015**: 本 Gate 完成后必须执行行为 Review、代码边界 Review 和 Frame V2 字段/时序 Review；只有 `review-green` 才能进入下一 Gate。
- **FR-016**: Review 必须核对三仓同一业务逻辑、Fixture hash/schema、Frame surface/scope/revision/mutation/directive、PC DOM 与 Mobile typed view。
- **FR-017**: 当前 Gate 的生产可达路径必须保持 InteractionFrame V2-only，V1 Frame 和 raw business render fallback 数量均为 0。
- **FR-018**: 运行报告必须分别记录 Helix、Fixture、PC Mock、PC Real、Mobile Contract、Review 和 blocker，不得用一个总 PASS 掩盖缺失证据面。

### Gate Contract Snapshot

- **Commands**: `im_revoke`
- **Effects**: `http_write、storage_upsert、projection`
- **Persistence facts**: `message`
- **Frame surfaces**: `message.timeline (InteractionFrame/v2)`
- **Allowed actions**: `无`
- 当前 UC 文档入口：[G-04](../../docs/interaction-frame-v2-全量替换-按UC/G04/README.md)。


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
- **SC-006**: 行为、代码和 Frame V2 Review 全部为 `review-green`；任何 partial/blocked 都阻止进入下一 Gate。

## Assumptions

- G01 已作为三仓同源 Fixture 与真实链基线完成，但仍需参加最终 26 Gate 删除前全量回归。
- 创建群聊是每个 canonical UC 的共享前置；G15a 仍独立验收群聊创建业务本身。
- PC Real 和 Mobile Contract 的运行环境在实施阶段按需建立；环境缺失时允许 blocked，不允许降级声明 Green。
- 本 Spec 只定义可验收目标；当前源码、文档、Fixture 或旧运行结果不自动满足这些要求。
