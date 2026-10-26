# Specification Quality Checklist: SPEC-IFV2-G01-在线发送

**Purpose**: 验证 G01 在线发送 Spec 在进入计划阶段前的完整性与可测性。
**Created**: 2026-07-21
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] CHK001 规格聚焦发送者、接收者和验证者的结果，不使用实现文件替代行为。
- [x] CHK002 所有必填章节已填充，没有模板占位符。
- [x] CHK003 将 Rust/DOM/MCP 指为用户要求的验证边界，而未预先规定代码结构或数据库写入方案。

## Requirement Completeness

- [x] CHK004 不含 `[NEEDS CLARIFICATION]` 标记。
- [x] CHK005 FR-001 至 FR-014 都可由接受或反证场景验证。
- [x] CHK006 正向发送、失败恢复、双用户隔离和证据对账都有独立用户故事。
- [x] CHK007 成功标准量化覆盖正常、失败、证据完整性、自动化测试和进度报告。
- [x] CHK008 边界条件覆盖权限变化、重启/恢复、不一致呈现、重复提交和重复/迟到确认。
- [x] CHK009 范围明确排除非文本 Gate、raw emit UI 回退及任何 direct database repair。
- [x] CHK010 假设明确了远端权威记录的语义、隔离账户/频道与只读查询能力。

## Feature Readiness

- [x] CHK011 每项功能要求都映射到用户场景、验收场景或成功标准。
- [x] CHK012 验收要求 Rust 行为测试、`cses-client` DOM、双用户、数据库对账和至少一个失败反证。
- [x] CHK013 30 分钟报告作为持续工作合同，不引入无人值守定时任务。
- [x] CHK014 修复仅允许受控源码/测试修复和重跑验证，禁止用数据库或 UI 伪造结果。
- [x] CHK015 G01 的 ActionBinding -> Effect -> Persist -> SurfaceComposer -> Frame 顺序被明确要求，且失败不得跳过前置阶段。

## Notes

- 无澄清项。具体文件 owner、wire 字段、fixture 路径和命令属于后续 `/speckit-plan` 的实施映射。
