# Quickstart: 26 Gate 总控

1. 读取 [plan-task-index.md](plan-task-index.md)，只进入当前 Gate。
2. 确认前序 Gate review-green。
3. 执行当前子 Feature 的 quickstart.md 和 tasks.md。
4. R4 与 cross-repo report Green 后才更新总控并进入下一行。
5. 26/26 Green 后生成删除候选；等待用户选择；删除后重新跑 26 Gate。

```bash
python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance
node scripts/verify_ifv2_three_repo_plans_tasks.mjs
git diff --check
```
