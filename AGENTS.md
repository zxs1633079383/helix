# AGENTS.md

本仓的 agent 指令体系以 **CLAUDE.md 为单一真源**（就近继承）：

1. 根 [`CLAUDE.md`](./CLAUDE.md) —— 三层架构铁律 / 三栖 checklist / 编译测试 cadence / harness 索引
2. `crates/<crate>/CLAUDE.md` —— 各 crate 的局部不变量与测试 pattern（就近优先于根）
3. [`.claude/rules/runtime-agnostic-core.md`](./.claude/rules/runtime-agnostic-core.md) —— 三栖规约完整展开
4. [`docs/GOAL.md`](./docs/GOAL.md) —— 北极星，与之冲突先停下问

**行为真源**：实现/修复 IM 行为有争议时，现网 `~/workspace/angular/cses-client/src-tauri/src/features/im/` 对应 handler 是 source of truth（类比 Bun 的 Zig-as-reference）。

**提交前闸门**：`scripts/gate.sh`（三栖 check + clippy + test + 零依赖 + 零业务 grep）。
