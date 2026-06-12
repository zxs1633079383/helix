# /helix-acceptance — 全量验收闸门

执行以下流程并汇总结果：

1. 跑 `scripts/gate.sh`（7 道结构闸门：三栖 check / clippy deny / 测试 / 零运行时 / 零业务 / harness 索引 / public-api）
2. 若用户要求完整验收或改过热路径：追加 `scripts/gate.sh --full`（release 承压三件套）
3. 对照 `docs/ROADMAP.md` §三 checklist 输出一张「闸门 | 结果」表
4. 任何一道红 → 按 `docs/harness/` 对应卡片定位根因，**先报告不偷修**（除非用户已授权 autonomous）
