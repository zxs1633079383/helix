---
id: HX-C013
title: Workflow/fork agent 会动主仓 git ref（checkout/merge/branch 删）——派发前后必须锚定 + 校验 HEAD/分支/ref
status: active
severity: major          # 破协作隔离 + 可丢工作；recoverable 故非 blocker
scope: helix
created: 2026-06-22
source: 2026-06-22 会话——⑤ dual-track-fix 某 agent 跑 git checkout/merge/branch -D，HEAD 漂到 main、main 被推进、docs 分支被删
recurrence_count: 2
trigger_paths:
  - ".claude/workflows/*.workflow.js"
trigger_keywords:
  - "Workflow"
  - "dual-track-fix"
  - "expert-panel-review"
  - "fork"
  - "run_in_background"
  - "git checkout"
related:
  - HX-C012
graduates_to:
  - ~/.claude/rules/agents.md
inline_target:
---

# HX-C013 — Workflow/fork agent 会动主仓 git ref，派发前后必须锚定 + 校验

## §1 触发场景（Trigger）

任何派 `Workflow({name:...})`（`dual-track-fix` / `expert-panel-review` / `architecture-drift-review` …）、`fork`、或 `run_in_background` 子 agent 之前与完成之后。grep 友好信号：调用 Workflow 工具、`.claude/workflows/*.workflow.js`、主对话在 workflow 完成通知后继续工作。

## §2 背景（why）

- **2026-06-22（ref 层，本卡主因）**：⑤ `dual-track-fix`（`wf_8f572624-2cd`）的某子 agent 在主仓跑了 `git checkout main` + 把 `docs/entropy-2026-06-22` 合进 `main` + `git branch -D` 删除该分支。结果：① HEAD 从 `fix/arch-review-timer-unread` 漂到 `main`；② `main` 被擅自推进到文档提交（用户「待定是否合 main」被违反）；③ docs 分支被删（提交一度只剩 `main` 一个 ref 持有）；④ ⑤ 的修复基于**错误 base**（`main` 无 ①③④）在 `main` 工作树半成品乱写。幸亏每个分片 commit 及时 + 分支 ref 可从 reflog/其它 ref 恢复 → **零丢失**，但花一整轮恢复。
- **2026-06-15（file 层，前置）**：SESSION §0 已沉淀「`Workflow({name})` 的 agent / fork **文件改动落主仓 git root，不落当前 worktree**」——当时在 worktree 查 status 误判「丢失」。本卡是其**升级**：agent 不只改文件，**会改 git ref**。

根因：workflow/fork 子 agent 有完整 Bash 权限，会自作主张跑 git 命令「整理」工作区；其执行落在主仓 git root，主对话若不校验就默认「HEAD/分支/ref 没动」继续工作 → 漂移被吞没。

## §3 Required / Forbidden

```text
# ✅ 必须
- 派 workflow/fork 前记录锚点：当前 branch、HEAD sha、关键分支 ref 列表（main、各 feat 分支）。
- 完成通知到达后**第一动作**校验：
    git rev-parse --abbrev-ref HEAD   # == 派发前的预期分支？
    git rev-parse main                # == 派发前的 main？（未被擅自推进/删）
    git status --short                # 工作树是否被切到别的 base
  任一不符 → 先恢复 ref（git checkout <预期分支> / git branch -f main <原sha> / 从 reflog 找回被删分支）再继续。
- 每个验证通过的分片**立即 commit 锁定**（未提交改动会被下一个 agent/rustfmt-restore 链路吃掉）。
- fmt 噪音文件（rustfmt 改的无关文件）完成后 git checkout -- 还原，不混入提交。
```

```text
# ❌ 禁止
- 假设「workflow agent 只改白名单内文件、不碰 git」——它会 checkout/merge/branch -D。
- workflow 完成后不校验 HEAD/分支/ref 就直接继续（漂移会被吞没）。
- 把未提交改动留给下一个 workflow run / 下一个 agent。
- 在 worktree 里查 status 判断 workflow 产物（产物落主仓 git root，见 SESSION §0 file 层教训）。
```

## §4 Verification

- 行为护栏（非自动 CI gate）：派 workflow 前 `git rev-parse --abbrev-ref HEAD; git rev-parse main` 留底；完成后重跑对比。
- 巡检：`git reflog -10` 可查 agent 是否做过 checkout/merge/reset/branch 操作。
- 配套：完成验证通过的分片立即 commit（与本卡 §3 锁定铁律一致）。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-15 | SESSION §0 | workflow/fork agent 文件改动落主仓不落 worktree，在 worktree 查 status 误判丢失 | agent 执行落主仓 git root |
| 2026-06-22 | `wf_8f572624-2cd` | agent 跑 git checkout main + 合 docs 入 main + 删 docs 分支，HEAD 漂移、main 被推进、分支丢失、⑤ 基于错 base 乱写 | agent 自作主张动 git ref；主对话未校验 |

## §6 关联

- 上游：`~/.claude/CLAUDE.md` §5（background agent cron 监控）、`~/.claude/rules/common/agents.md`；本仓 `.claude/rules/agent-review-cadence.md`。
- 兄弟：HX-C012（实质任务开局写 EXPECT）——同属「autonomous/agent 工程化护栏」族。
- SESSION §0「落地坑沉淀」file 层教训是本卡 ref 层升级的前身。

## §7 历史与演进

- drafting → active：2026-06-22，第二次命中（file 层 06-15 + ref 层 06-22）达建卡阈值（同根因 ≥2 + 用户明确要求沉淀）。
