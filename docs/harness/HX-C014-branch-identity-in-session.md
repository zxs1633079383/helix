---
id: HX-C014
title: 每次 workflow/worktree 收尾必把「分支身份」固化进 SESSION §0（当前分支+upstream+该不该删），靠脚本自陈不靠记忆
status: active
severity: medium         # 不丢工作，但反复消耗注意力 + 误操作风险
scope: helix
created: 2026-06-23
source: 2026-06-22 会话——单分支+worktree+手工 merge/删除几天后，「你在哪个分支」一天问 ≥6 次（日志复盘实测）
recurrence_count: 2
trigger_paths:
  - "SESSION.md"
  - "scripts/branch-identity.sh"
trigger_keywords:
  - "你在哪个分支"
  - "哪个分支"
  - "merge"
  - "worktree"
  - "branch -D"
  - "dev-zlc"
related:
  - HX-C013
  - HX-C012
graduates_to:
  - ~/.claude/rules/git-workflow.md
inline_target:
---

# HX-C014 — 分支身份固化进 SESSION §0（脚本自陈，不靠记忆）

## §1 触发场景（Trigger）

任何 workflow / worktree / 多分支并行开发的**收尾节点**（merge 完、push 完、删分支后），以及**新会话开局**。grep 友好信号：用户问「你在哪个分支」「哪个分支可以删」「merge 到哪」；调用 `git worktree` / `git branch -D` / `git checkout`。

## §2 背景（why）

2026-06-22 日志复盘实测：单条主线（helix sync-v2 / im-takeover）一天里「你在哪个分支」「dev-zlc 能合吗」「删这个分支」反复出现 **≥6 次**。根因——分支状态只活在当时对话上下文里，`worktree + 手工 merge + 删分支`几天后，主对话与用户都迷失：

- 当前 HEAD 在哪个分支？有没有 upstream？（本会话实测：主仓在 `ci/test-harbor-runner` 而非以为的 `main`）
- 哪些分支已并入 main、可安全删？哪些 worktree 还占着不能删？
- 领先/落后集成分支多少？（HX-C013 的 ref 漂移 + 本卡的身份迷失是同族「git 状态不可查」病）

「分支身份」和「任务终态」一样，必须从**对话记忆**降级为**文件事实 + 一键自陈**，`grep '🧭 BRANCH-ID' SESSION.md` 即得。

## §3 Required / Forbidden

```text
# ✅ 必须
- 收尾（merge/push/删分支后）+ 开局：跑 `scripts/branch-identity.sh --session`，
  把输出的单行 🧭 BRANCH-ID 写进/刷新 SESSION §0 顶部（覆盖旧的那一行，保持唯一最新）。
- 拿不准「我在哪 / 哪些可删」时：先 `scripts/branch-identity.sh`（full）自陈，再动手，
  不靠记忆回答「你在哪个分支」。
- 删分支前：以 full 模式「可删候选（已并入 main）」清单为准，worktree 占用的分支永不删。
- 长期单分支开发警惕：配合 `scripts/branch-sync-gate.sh`（落后集成≥阈值 FAIL）双管。
```

```text
# ❌ 禁止
- 凭对话记忆回答「当前在 X 分支」——上下文一过即错（本会话就误以为在 main，实际 ci/test-harbor-runner）。
- 删分支不先看 branch-identity 的「可删候选」清单（可能删掉未并入的工作或 worktree 占用分支）。
- SESSION §0 留多条过期 🧭 BRANCH-ID 行（只保留最新一条，旧的就地覆盖）。
- 把「该不该删」留给口头确认而不落 SESSION（下次又得重新调查）。
```

## §4 Verification

- 行为护栏（非 CI 硬 gate）：`grep -c '^🧭 BRANCH-ID @' SESSION.md` 应为 1（行首锚定唯一最新；header 说明里反引号引用不计）。
- 自陈一致性：`scripts/branch-identity.sh --session` 的当前分支 == `git rev-parse --abbrev-ref HEAD`。
- 可删候选准确性：清单内每条 `git rev-list --count main..<branch>` 必须 == 0。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-22 | 日志复盘（198 条） | 「你在哪个分支 / dev-zlc 能合吗 / 删这个分支」一天 ≥6 次 | 分支状态只在对话上下文，worktree+手工 merge 后迷失 |
| 2026-06-23 | 本卡 | 主对话误以为在 main，实际主仓在 `ci/test-harbor-runner`（↑3）| 凭记忆答分支身份 |

## §6 关联

- 兄弟：HX-C013（workflow agent 动 git ref）——同属「git 状态不可查」病；C013 管 ref 漂移，C014 管身份迷失。
- 配套脚本：`scripts/branch-identity.sh`（自陈）+ `scripts/branch-sync-gate.sh`（分叉闸门）。
- 上游：`~/.claude/CLAUDE.md` §6（worktree 并行）+ `rules/common/git-workflow.md`（终态行哲学，本卡是其分支维度延伸）。

## §7 历史与演进

- drafting → active：2026-06-23，第二次命中（06-22 ≥6 次 + 06-23 误判）达建卡阈值（同根因 ≥2 + 用户明确要求实现）。
- merged 条件：`branch-identity.sh --session` 接入 Stop hook 自动刷 SESSION §0 顶行、30 天零「你在哪个分支」复问后晋升 `~/.claude/rules/git-workflow.md`。
