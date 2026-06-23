# Playbook — 夜间跑 / 早上交 Goal-Loop（无人值守 + 预算/沙箱护栏）

> 这是**操作手册（runbook）**，不是 HX-C 约束卡（不计入 `HX-C*` 索引不变量）。
> 配套：`scripts/night-loop-preflight.sh`（一键启动）· `.claude/night-loop.settings.local.json`（deny 模板）。
> 核心铁律：**达标即停（不是跑满时间）· 全程 worktree 沙箱 · 交付前必绿灯 · 绝不自动 merge main。**

---

## 0. 与 `docs/GOAL.md` 的关系（重要 · 不要混淆）

| 文件 | 层级 | 寿命 | 夜间 loop 怎么对待 |
|---|---|---|---|
| `docs/GOAL.md` | **L2 北极星 / 设计理念不变量** | 永久 | **只读守护**：每 tick 改动若违反其 §1/§2 → 立即 abort 写 BLOCKED |
| `worktrees/night-<task>/NIGHT_GOAL.md` | **L0 本次任务目标** | 一晚即弃 | 每 tick 重读，SUCCESS 全绿即停 |

夜间 agent **永不创建/修改 `GOAL.md`**（那是北极星）；它只读 `docs/GOAL.md` 当护栏，只写自己 worktree 内的 `NIGHT_GOAL.md`。

---

## 1. 总览

```
22:00 启动                                   次日 08:00 你醒来
  │                                               │
  ▼                                               ▼
┌──────────── NIGHT GOAL LOOP（goal 型，达标即停）─────────────┐
│ tick → 做一小步 → 验证闸(gate.sh) → 进度检测 → 预算检测       │
│  ▲                                  │ 红灯↺修   │ 无进展/超预算  │
│  └──────────── 绿灯且未达标，继续 ──┘            ↓ 触发停止      │
└───────────────────────────────────────────────────────────────┘
        全程 git worktree 沙箱 · 守 docs/GOAL.md · 禁毁灭性操作 · 禁 push main
                               │
                               ▼
       晨间 handoff：SESSION §0 终态行 + REPORT.md（diff + 待拍板）+ 留 branch
```

---

## 2. NIGHT_GOAL.md 模板（preflight 自动生成到 worktree 根）

```markdown
# 夜间任务目标 — <task-slug>
任务：<一句话，如「把 helix-im/src/module.rs 拆到 ≤300 行且 gate.sh 全绿」>
启动：<ts> | 分支：night/<task-slug> | worktree：worktrees/night-<task-slug>/

## 守护约束（GUARD — 改动违反任一立即 abort 写 BLOCKED.md）
- 不违反 docs/GOAL.md §1 设计理念不变量 / §2 硬约束（运行时无关 / sans-IO / 依赖单向 …）
- 不违反 HX-C001..C015 在册护栏

## 成功条件（SUCCESS — 全部满足才算 done，必须机器可验证）
- [ ] `bash scripts/gate.sh` 退出码 0
- [ ] `bash scripts/structure-gate.sh` 退出码 0
- [ ] `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` 通过
- [ ] `cargo test -p helix-core` 通过
- [ ] `git status --porcelain` 为空（无未提交残留）
- [ ] <任务专属验收，如某文件 ≤300 行 / 某测试新增并 PASS>

## 停止条件（STOP — 命中任一立刻停，写 PARTIAL + REPORT.md）
- 迭代 ≥ 12 tick 仍未达 SUCCESS
- 连续 3 tick 的 `git rev-parse HEAD` 无变化（无进展 = 卡死）
- 累计输出 token ≥ 400k
- 命中 BLOCKED（需跨仓改动 / 需人决策 / 测试逻辑性失败修不动）

## 禁区（NEVER）
- 不碰 main / dev-zlc；不 push；不 merge；不 git reset --hard；不 cargo clean
- 不改根目录黑名单 SESSION.md / CLAUDE.md / README.md；不创建或改 GOAL.md
- 不删非本任务创建的文件
```

> 关键：SUCCESS 全是**退出码/命令**，杜绝「我觉得改好了」式自我点头（= 研究里的"会说不的闸门"）。

---

## 3. 三道护栏

### 3.1 预算闸（防烧钱 + 防空转）
| 维度 | 阈值 | 机制 |
|---|---|---|
| token/钱 | 400k 输出 token | 启动用 `+400k` 预算指令；Workflow 内 `budget.remaining()` 守循环 |
| 迭代 | ≤ 12 tick | NIGHT_GOAL.md STOP，runner 计数 |
| 无进展 | 连续 3 tick HEAD 不动 | 每 tick 记 `git rev-parse HEAD`，三次相同即停 |

### 3.2 沙箱闸（防权限失控 / 误删）
- **worktree 隔离**：`worktrees/night-<task>/`，独立 branch `night/<task>`；`/worktrees/` 已在 .gitignore。
- **df 预检**：可用 < 10GB 直接拒启（ENOSPC 防御，preflight 已内置）。
- **deny 白名单**：`--arm` 把 `.claude/night-loop.settings.local.json` 装入 `.claude/settings.local.json`（备份原文件），deny push/merge/reset/rm -rf/cargo clean/gh 写。晨间 `--disarm` 还原。
- **commit 前 guard**：每次 commit 前断言 `pwd` 含 `worktrees/night-<task>` 且 HEAD 在 `night/<task>`，漂了立即 abort（HX-C013 / cwd-branch guard）。

### 3.3 验证闸（交付前必绿灯）
每 tick 收尾 + 判 done 前，**必跑** NIGHT_GOAL.md 的 SUCCESS 命令链：
- 绿灯 + 未达标 → 继续；红灯 → 本 tick 修，修不动 → STOP/BLOCKED；全绿 + 达标 → 才允许写 `✅ DONE`。
- 无明确测试的主观任务（文档 / UI）→ test gate 换 **writer→verifier rubric 双 agent 对账**。

---

## 4. 启动 / 编排（Claude Code 现成机制）

```bash
# 22:00 一键预检 + 建沙箱 + 生成 NIGHT_GOAL.md（+ 可选装 deny）
bash scripts/night-loop-preflight.sh refactor-module --arm
#   → 产出 worktrees/night-refactor-module/ + NIGHT_GOAL.md，打印下一步

# 然后派 autonomous agent / Workflow 跑 goal loop，prompt 要点：
#   读 NIGHT_GOAL.md + docs/GOAL.md → 循环执行 → 每 tick 跑验证闸
#   → 命中 STOP 写 REPORT.md + SESSION §0 终态行；预算 +400k
```

- **主循环**：goal-loop 优先（`/loop` dynamic 模式自定步长，或后台 Agent + `ScheduleWakeup` 长心跳 1200s+ 兜底）。
- **完成回灌靠 task-notification 自动唤醒主对话**（HX-C015）—— **不挂 10min 密集 cron 轮询**；只挂 20-30min「假死兜底」心跳。
- 外部不可追踪态（远程 CI）才用短周期主动轮询。

---

## 5. 晨间 handoff（30 秒看懂）

收尾**强制**写终态行（CLAUDE.md 全局铁律），落 `SESSION.md §0`：

```
✅ DONE 夜间拆 module.rs @2026-06-24T06:12 | commit a1b2..c3d4 | 8/8 步 | helix night/refactor-module
⚠️ PARTIAL 夜间拆 module.rs @06:12 | 卡在 ws/registry 循环依赖 | 剩 2 文件 | coord: worktrees/night-refactor-module/BLOCKED.md
```

并留 `worktrees/night-<task>/REPORT.md`：状态 / 预算用量 / `git diff --stat main..night/<task>` / 验证闸结果 / **需你拍板项** / 合并指引。

> **绝不自动 merge main** —— agent 只交「绿灯 branch + 报告」，merge 决策权留你（早上看 diff 再定，前置冲突分析见 CLAUDE.md §6.5）。

---

## 6. 红线 checklist（启动前过一遍）
- [ ] NIGHT_GOAL.md 的 SUCCESS 全是机器可验证命令
- [ ] STOP 三闸齐全：迭代 + 无进展 + token 预算
- [ ] worktree 已建 · df ≥ 10GB · branch ≠ main/dev-zlc
- [ ] deny 已 `--arm`（push/merge/reset/rm -rf/cargo clean/gh）
- [ ] 验证闸在「每 tick 收尾」和「判 done 前」都跑
- [ ] 收尾写终态行 → SESSION §0 + REPORT.md
- [ ] 不自动 merge · 不碰根目录黑名单 · 不创建/改 GOAL.md
