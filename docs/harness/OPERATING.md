# Helix Harness — 触发操作手册（什么事 → 调什么）

> 2026-06-12 沉淀。解决「工程化资产躺着没人调」的问题。
> **一句话判据**：**机械活（fmt/lint/gate）= 钩子自动跑；判断活（workflow/skill/expert）= 人工触发，但触发条件写在这张表里，照表调，不靠记。**
> 母本方法论见根 [`CLAUDE.md`](../../CLAUDE.md) §4 + [`.claude/rules/expert-dispatch.md`](../../.claude/rules/expert-dispatch.md)（渐进式披露 L0→L3）。

---

## 1. 四个节律（按事件粒度，每格标自动/人工）

| 触发事件 | 调什么 | 自动/人工 | 怎么发生 |
|---|---|---|---|
| **存盘 `.rs`** | `cargo fmt` | 🤖 自动 | PostToolUse hook（`.claude/settings.json`），每次 Edit/Write `.rs` |
| **编译** | clippy deny-list 拦截 | 🤖 自动 | `cargo clippy/check` 时 `clippy.toml` 生效 |
| **开局新会话** | 提醒读 `ROADMAP §二 P0` + 本手册 + 列未完 P0 | 🤖 自动 | SessionStart hook |
| **会话收尾** | 有未提交 `.rs`→提醒 `gate.sh`；有未提交 `.md`→提醒 `/helix-drift-check` | 🤖 自动 | Stop hook（只读 `git status`，不跑编译） |
| **准备 commit** | `scripts/gate.sh`（七道闸门快门） | 👤 人工（push 时 pre-push 兜底自动） | 命令；或 `git push` 触发 pre-push |
| **phase 完成 / 验收** | `/helix-acceptance`（跑全部结构+行为闸门汇总） | 👤 人工 | slash |
| **加业务模块** | `/helix-new-module` | 👤 人工 | slash |
| **写回放测试** | `/helix-replay-test` | 👤 人工 | slash |
| **承压验收 / 性能结论** | `/helix-stress-verify`（手册）+ `Workflow({name:"stress-verify"})`（实测随队） | 👤 人工 | slash + workflow |
| **重审查**（架构方案 / diff / 设计是否有坑） | `Workflow({name:"expert-panel-review"})` | 👤 人工 | workflow（4 专家对抗证伪） |
| **P0 多片并行修复** | `Workflow({name:"dual-track-fix"})` | 👤 人工 | workflow（分片→对抗→回炉→主对话 gate+commit） |
| **借用/Send 报错·trait 争议·热路径审查** | `rust-dev` agent ↔ `/rust-knowledge` | 👤 人工 | 判断触发 |
| **wasm 编不过·三栖边界·FFI** | `rust-runtime-agnostic-core-dev` ↔ `/rust-runtime-agnostic-core-knowledge` | 👤 人工 | 判断触发 |
| **现网对齐·IPC 契约·Tauri 接缝** | `rust-tauri-skeleton-dev` ↔ `/rust-tauri-skeleton-knowledge` | 👤 人工 | 判断触发 |
| **内核循环·Effect 批次·调度** | `dioxus-dev`（仅 core 经验，禁渲染层） ↔ `/dioxus-knowledge` | 👤 人工 | 判断触发 |
| **文档↔代码漂移（熵管理）** | `/helix-drift-check` + 更新状态文档 + 查 harness 索引不变量 | 👤 人工（可挂 cron） | slash |
| **同根因≥3 / 用户拍板 / Spec** | 新建 `docs/harness/HX-C{NNN}` + 同步索引三处 | 👤 人工 | 三铁律（README §1） |

---

## 2. 「哪些该自动、哪些该保持人工」（回答：这些 🟡 正常吗）

**正常。** 不是所有资产都该自动——**判断活自动化 = 烧钱 + 放大错误**。分界：

- 🤖 **该自动**（机械、确定、便宜、无判断）：fmt / clippy / gate.sh / 开局收尾提醒。已全部接钩子。
- 👤 **该保持人工**（判断、昂贵、需触发条件）：
  - **workflow**（panel-review 一次 28 agent / 2.4M token）——绝不能自动 fan-out，必须人在重决策/重审查时拍板调。
  - **skill / expert agent**——干对应活、撞对应问题时调；自动加载只会污染上下文。
  - 它们「躺着」**不是 bug**，只要**触发条件可见**（就是本表）。问题从来不是「没自动」，而是「触发条件埋在文档里没人翻」——本表就是解药。

**真正要修的只有两类**：① 触发条件不可见 → 本表固化；② 实际坏了 → 修（见下）。

---

## 3. 已修的两个「实际坏了」（2026-06-12）

- **pre-push 没装** → `scripts/install-hooks.sh` 跑一次即生效（新 clone 必跑）。**钩子不进版本库，必须 installer**。
- **workflow `args` 调不起来** → 本运行时把 `args` 当 **JSON 字符串**注入（实测 `typeof args === "string"`），脚本旧代码 `args.target` 在字符串上取属性返回 undefined。三个在册脚本已加归一化 shim（parse-if-string），`Workflow({name, args:{...}})` 现可用。详见 [`.claude/workflows/README.md`](../../.claude/workflows/README.md)。

---

## 4. workflow 调用速查（修复后）

```js
// 重审查：审一个方案/diff 有没有坑（4 专家对抗证伪）
Workflow({ name: "expert-panel-review", args: { target: "docs/ROADMAP.md 的 X", context: "审 Y" } })

// P0 多片并行修复（白名单零交集 → 对抗 review → 回炉 → 主对话 gate+commit）
Workflow({ name: "dual-track-fix", args: { shards: [
  { crate: "helix-im", whitelist: ["crates/helix-im/src/parser.rs"], task: "...", lens: "rust" }
] } })

// 性能/承压主张实测随队（无数字不收）
Workflow({ name: "stress-verify", args: { claim: "...", scenarios: [{ name, cmd, expect }] } })
```

> workflow **不做 git merge/commit**（项目 §6：merge 由主对话前置冲突分析后执行）。

---

## 5. 索引

本手册 ↔ 根 [`CLAUDE.md`](../../CLAUDE.md) §4.6（专家注册表）+ [`README.md`](./README.md)（坑卡库）+ [`.claude/workflows/README.md`](../../.claude/workflows/README.md)（编排脚本）三处互链。改触发器（加 hook / 加 workflow / 加 skill）→ 同步本表。
