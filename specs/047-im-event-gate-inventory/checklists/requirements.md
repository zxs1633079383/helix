# Specification Quality Checklist: IM 事件域 Gate 与双视角合同盘点

**Purpose**: 在进入补充确认或规划前，验证 Gate 规格完整性与证据边界。
**Created**: 2026-07-28
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] 技术重构所需的实现细节仅用于界定删除边界、生产真源和可证伪验证，不引入平台 UI 方案
- [x] Focused on user value and business needs
- [x] 面向 IM、Helix 与客户端工程评审者可独立审阅，业务角色和双视角结果无需先读源码
- [x] All mandatory sections completed

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain
- [x] Requirements are testable and unambiguous
- [x] Success criteria are measurable
- [x] 成功标准区分业务终态与必要的仓内物理下线证据，未用具体 UI 实现替代结果
- [x] All acceptance scenarios are defined
- [x] Edge cases are identified
- [x] Scope is clearly bounded
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria
- [x] User scenarios cover primary flows
- [x] Feature meets measurable outcomes defined in Success Criteria
- [x] 规格中的路径、事件名和测试入口均属于本次协议硬切的可验证合同，不包含实现代码

## Notes

- 规格中的 `H-FULL`、`H-FORMAL`、`W-M/S/D` 只表示测试源码入口存在，统一附带
  `NOT_RUN`，没有升级为运行态证据。
- 当前 Strict Matrix 的 epoch 12/50 Gate 与截图 30 Gate 差异已进入范围对账。
- 30 个 Gate 的 outbound 审阅样例已统一为 sender/receiver 双视角；persona-local
  receiver 空集、G14 read/readers 分流与 G16b roster/left 分流均有机器断言。
- G04 已区分 Go 的 `posts_update` 必达主广播与条件式 `update_channel` 副广播；目标 raw
  Emit 只保留本地 `message` 与 `channel_member` 可读回字段。viewer 撤回文案不再伪装成
  数据库字段；Go 的全表 latest 判断与 Helix raw channel 绝对态缺口保持
  `PARTIAL/INFERRED`。
- 30 个 Gate 的 inbound/outbound 样例现均引用远端 PostgreSQL 与本地 444/999 SQLite
  schema lock；直接落库字段与查询/角色/correlation 派生字段分开列出，便于逐 Gate 人工审核。
- G-03 以及 G-17 至 G-27 是否进入下一批，保留为用户审核决策，不是规格歧义。
- 本清单按“技术协议重构规格”而非面向终端用户的产品 PRD 评估；实现名只在它们构成物理删除
  或证据边界时保留。SC-009 只约束 Helix，本版本未同步的客户端引用保持
  `NOT_SYNCED_BY_SCOPE`。
