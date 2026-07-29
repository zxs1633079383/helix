# Research: InteractionFrame V2 双仓硬切

## R-001 单一 Spec Kit feature

**Decision**: 018 是唯一 workflow feature；19 个业务 Spec 作为 Gate 合同，计划和任务集中在 018。

**Rationale**: Spec Kit ledger、R1-R4 和 active feature 都是单上下文。拆成 19 个 feature 会产生 pin、shared foundation、E2E candidate 和删除准入的多真源。

**Alternatives considered**:

- 19 个独立 active feature：拒绝，会争抢 `.specify/feature.json` 并重复 shared V2 基座。
- 只保留总规格：拒绝，无法给每个 Gate 可执行目标与证据。

## R-002 V2-only 切换顺序

**Decision**: 首轮全量 E2E 前，Helix V1 producer 与 cses-client V1 consumer/normalize 必须在生产路径不可达；物理代码删除在首轮全绿之后。

**Rationale**: 双写/双读候选的绿色不能证明 V2 链真实工作；先不可达再删除兼顾证据可信度与回滚诊断。

**Alternatives considered**:

- 首轮允许 V1 fallback：拒绝，无法区分 V2 成功和兼容兜底。
- 先物理删除再首次 E2E：拒绝，风险过大且不利于定位遗漏 Gate。

## R-003 数据库边界

**Decision**: 不新增 PG/SQLite schema 迁移；每个 Gate 只声明现有业务表和正常产品链 write-set，验收使用有界只读 delta。

**Rationale**: Frame V2 是业务投影/交互协议迁移，不应通过数据库修复制造 UI 结果。cursor 与 surface revision 是不同领域状态。

**Alternatives considered**:

- 为 Frame revision 新增业务表：拒绝，Frame Cache 可丢弃且不代表业务 authority。
- 测试前直接清表/改行：拒绝，破坏证据真实性。

## R-004 Shared foundation

**Decision**: 唯一 V2 builder/emitter、V2-only codec/types、surfaceEpoch slot state、FrameApplied 和证据 counters 必须先于任何 Gate 最终签收完成。

**Rationale**: 这些是所有 Gate 的共同协议边界；逐 Gate 各自实现会导致 19 份漂移。

**Alternatives considered**:

- 每 Gate 自带 builder/decoder：拒绝，扩大协议分叉。
- 只在 reducer 统计 schema：拒绝，无法证明 runtime ack 与 raw fallback。

## R-005 Gate sequencing

**Decision**: 按 recovery → send → message operations → query/read/governance → first E2E → delete → second E2E 执行。

**Rationale**: recovery 建立 authority/cache 基线；send 提供 canonical identity；其余 Gate 依赖这些共享事实。最终 E2E 必须批量构建一次，避免按 Gate 重编。

**Alternatives considered**:

- 19 Gate 完全并行：拒绝，共享文件和 source pin 冲突。
- 按 UI 页面顺序：拒绝，不符合依赖与持久化屏障。

## R-006 可观测性与隐私

**Decision**: correlation 贯穿 action/tick/effect/persist/frame/ack，但日志只记录 identity/hash/count/status，不记录 cookie、token、消息正文或无界 VM。

**Rationale**: 多跳诊断需要关联，而真实账号/消息数据必须最小化。

## R-007 SCSS/CSS 验收

**Decision**: 在现有 harness 中新增明确脚本，组合 Sass 构建、目标样式载入、DOM bounding box/overflow 断言和 444/678 截图。

**Rationale**: Jest/截图存在不等于样式编译和布局合同被强制执行。

**Alternatives considered**:

- 仅人工看图：拒绝，不可重复且无法 fail-closed。
- 每组件建立独立视觉服务：拒绝，超出当前迁移范围。

## R-008 Go 变更策略

**Decision**: 默认不改 Go 业务逻辑；只在真链证据证明 wire/authority 缺陷时建立独立修复任务和人工 gate。

**Rationale**: 现役 Go 是权威事实源，本功能目标是 Frame V2 硬切，不应趁机重写服务端。
