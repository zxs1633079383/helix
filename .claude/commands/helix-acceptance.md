# /helix-acceptance — 全量验收闸门

执行以下流程并汇总结果：

1. 跑 `scripts/gate.sh`（当前 11 步默认门：双端 check / clippy / 测试 / 零运行时 / trace 静态门 / 零业务 / harness 索引 / FFI 仓内自验 / public-api / structure-gate 等；以脚本输出为准）
2. 若用户要求完整验收或改过热路径：追加 `scripts/gate.sh --full`（release 承压三件套）
3. 对照 `docs/ROADMAP.md` §三 checklist 输出一张「闸门 | 结果」表
4. 任何一道红 → 按 `docs/harness/` 对应卡片定位根因，**先报告不偷修**（除非用户已授权 autonomous）。若 `scripts/structure-gate.sh` 红，不得声称 acceptance green。
