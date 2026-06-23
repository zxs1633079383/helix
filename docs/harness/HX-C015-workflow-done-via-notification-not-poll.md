---
id: HX-C015
title: workflow/后台 agent 完成靠 task-notification 自动回灌，禁手工「等 workflow 跑完报告结果」轮询；只外部不可追踪状态才 /loop
status: active
severity: medium         # 不丢工作，但烧注意力 + 拉长 wall-clock + 污染上下文
scope: helix
created: 2026-06-23
source: 2026-06-22 会话——「等 workflow 跑完报告结果 / 等 drift-review 跑完报告结果」手工催 ≥5 次（日志复盘实测）
recurrence_count: 2
trigger_paths:
  - ".claude/workflows/*.workflow.js"
trigger_keywords:
  - "等 workflow"
  - "等 workflow 跑完"
  - "等 drift-review"
  - "跑完报告结果"
  - "进度到哪里"
  - "run_in_background"
related:
  - HX-C013
graduates_to:
  - ~/.claude/rules/agents.md
inline_target:
---

# HX-C015 — workflow 完成靠通知回灌，不手工轮询；外部不可追踪状态才 /loop

## §1 触发场景（Trigger）

派出 `Workflow({name})` / `run_in_background` agent 之后。grep 友好信号：用户/主对话说「等 workflow 跑完报告结果」「等 drift-review 跑完」「进度到哪里了」「恢复 workflow」。

## §2 背景（why）

2026-06-22 日志复盘：「等 workflow 跑完报告结果」「等 drift-review 跑完报告结果」手工催 **≥5 次**。这是纯浪费——

- **harness 已对 `Workflow` 与 `run_in_background` agent 在完成时自动发 `<task-notification>` 重新唤起主对话**（见本仓顶层 Context management + Agent 工具说明 + 全局 CLAUDE.md §5）。完成信号**不需要轮询**，到点自动回灌。
- 手工「等…报告结果」的代价：① 占一轮对话什么都没干；② 主对话空转/反复 Read output 文件抖动；③ `ScheduleWakeup` 工具说明明确「不要为 harness 能追踪的后台工作挂短周期轮询」。

判据一句话（与全局 §5 一致）：**harness 能自动通知完成的 → 等通知 + 必要时长心跳兜底；harness 追不到的外部状态（CI run / 远程部署 / 队列）→ 才 `/loop` 主动轮询。**

## §3 Required / Forbidden

```text
# ✅ 必须
- 派 Workflow / run_in_background agent 后：直接做别的事 / 把控制权交回用户，
  等 <task-notification> 自动回灌再处理结果（这是默认且唯一推荐路径）。
- 真要等外部不可追踪状态（CI job、远程 deploy、harbor 镜像 pull、远端队列）：
  用 `/loop`（ScheduleWakeup），间隔按「该状态多快变」选：
    - <5 min（60–270s）：CI / deploy 这类活跃外部态（缓存还热）。
    - 派后无具体信号、纯兜底：1200–1800s 长心跳（别选 300s）。
- 多 background agent 并行且确需进度窗 → 全局 §5 的「长周期心跳兜底 + 异常闸门」cron（20–30min，非 10min 密集）。
```

```text
# ❌ 禁止
- 对 Workflow / run_in_background agent 手工「等 X 跑完报告结果」轮询——完成会自动回灌。
- 派完后反复 Read output 文件刷进度（抖动 + 烧 token）。
- 给 harness 能追踪的后台工作挂 10min 密集 cron（2026-06-07 实测 119 次巡检 110+ 空转）。
- 选 300s 唤醒间隔（最差档：吃了缓存 miss 又没摊薄——要么 <270s 保缓存，要么 ≥1200s 摊销）。
```

## §4 Verification

- 行为护栏（非 CI gate）：派 Workflow/agent 后的下一条主对话动作**不是**「等…报告结果」轮询。
- 巡检：日志里 `grep '等 workflow.*报告结果\|等.*跑完' /workspace/java/logs/<date>.json` 计数应随时间下降（§4 收敛目标）。
- `/loop` 用对场景：唤醒 reason 指向外部不可追踪态（CI/deploy/queue），不是「等本地 workflow」。

## §5 Recurrence Log

| 日期 | session/commit | 现象 | 根因 |
|---|---|---|---|
| 2026-06-07 | csesapi→cses-im-server 迁移 | 固定 10min cron 巡检 119 次，真触发干预个位数，110+ 空转 | 给可追踪后台工作挂密集轮询 |
| 2026-06-22 | 日志复盘（198 条） | 「等 workflow/drift-review 跑完报告结果」手工催 ≥5 次 | 没用完成通知自动回灌，习惯性轮询 |

## §6 关联

- 上游：`~/.claude/CLAUDE.md` §5（事件驱动 + 异常告警，砍固定 10min 空转）——本卡是其 helix 项目级落地。
- 工具说明：`ScheduleWakeup`（缓存窗口与 delaySeconds 选择）+ Agent / Workflow（完成自动 task-notification）。
- 兄弟：HX-C013（workflow agent git ref 护栏）——同属「autonomous/agent 工程化护栏」族。
- 手册：`OPERATING.md` §1 新增「派 workflow 后怎么等」行。

## §7 历史与演进

- drafting → active：2026-06-23，第二次命中（06-07 密集 cron 空转 + 06-22 手工催 ≥5 次）达建卡阈值（同根因 ≥2 + 用户明确要求实现）。
- merged 条件：30 天日志零「等 workflow…报告结果」复现后晋升 `~/.claude/rules/agents.md`。
