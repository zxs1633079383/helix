# `.claude/workflows/` — 具名可重放工作流注册表

这些 `*.workflow.js` 是 **`Workflow` 工具的具名预定义脚本**：放进本目录后，`Workflow({ name: "<name>", args: {...} })` 即可一键调起（团队成员 / cron / 跨会话复用，支持 resume）。

> 来源：这是 bun「100w rust 迁移」（PR #30412 `23427dbc12`）`.claude/workflows/phase-*.workflow.js` 同款机制——把「怎么编排 agent」从**文档配方**升级成**可执行、可重放、可 resume 的代码资产**。
> 它把根 `CLAUDE.md §4.6` / `.claude/rules/expert-dispatch.md §3` 的 L3 编制配方固化成脚本。

## 编制 = 廉价批量层 + 专家评审层

- **批量机械活**（逐文件 port/fix/sweep）→ 默认 workflow 子 agent（不带 agentType）+ schema + 指向 rules 的 prompt。便宜、可大 fan-out。
- **发现/扫描** → `agentType:'Explore'`。
- **领域评审 / 正确性闸门** → 4 专家：`rust-dev` / `rust-runtime-agnostic-core-dev` / `rust-tauri-skeleton-dev` / `dioxus-dev`（仅 core 内核经验，prompt 已锁死禁渲染层）。

## 在册工作流

| name | 模式 | 干什么 | args |
|---|---|---|---|
| `expert-panel-review` | 扇出-综合 + 对抗验证 | 四专家并行审一个设计/diff/方案，每条发现 2 票对抗证伪，汇总确认项 | `{ target, context? }` |
| `dual-track-fix` | 分类(分片)+扇出+对抗+回炉+gate 过滤 | 按 crate 分片(白名单零交集)并行修复 → 专家对抗 review → blocking 回炉；主对话跑 `gate.sh` + 分 commit | `{ shards:[{crate,whitelist,task,lens?}], baseline? }` |
| `stress-verify` | 生成-过滤 + 对抗验证 | 多场景实测出数字 → core-dev+rust-dev 对抗审方法学；无数字/方法学不成立不收 | `{ claim, scenarios:[{name,cmd,expect?}] }` |
| `architecture-drift-review` | 扇出-综合 + 对抗验证 | 对 `<last_reviewed_commit>..HEAD` 的 diff **+ SESSION §0 的 ⚡ DRIFT-PRED 预期对账信号**（§4.8）查架构偏移（HX-C001..C008 + GOAL §1/§2 + 全流程合理性），对抗证伪 → drift 判定 + **候选约束回放闸门** + 收敛 todo + ledger 更新指令。**强制节律**：gap≥10 commit 必跑（见 `.claude/rules/agent-review-cadence.md`）| `{ base? }`（缺省读 ledger） |

## 调用示例

```
Workflow({ name: "expert-panel-review", args: { target: "docs/GOAL.md", context: "审 §1 设计理念与 §2 硬约束是否自洽" } })

Workflow({ name: "dual-track-fix", args: { shards: [
  { crate: "helix-core",          whitelist: ["crates/helix-core/src/event_loop.rs"], task: "...", lens: "rust" },
  { crate: "helix-driver-native", whitelist: ["crates/helix-driver-native/src/ws.rs"], task: "...", lens: "core" }
] } })

Workflow({ name: "stress-verify", args: { claim: "push 落库热路径 O(1)", scenarios: [
  { name: "bench-insert", cmd: "cargo bench -p helix-core --bench hotpath", expect: "随 N 增长基本持平" }
] } })
```

## 约束与边界

- workflow **不做 git merge / commit**（项目 §6 铁律：merge 由主对话做前置冲突分析后执行）。`dual-track-fix` 只产出 reviewed 改动 + merge-plan，主对话跑 `scripts/gate.sh` 终验后分 commit。
- 编译/测试 cadence：workflow agent = autonomous 模式，**自跑验证**（根 `CLAUDE.md §3`）；与主对话单任务 prompt「输出命令让用户决定」不冲突。
- 脚本无 `Date.now()/Math.random()`（会 break resume）；按 index/label 区分 agent。
- **⚠️ args 以 JSON 字符串注入**（本运行时实测 `typeof args === "string"`，非已解析对象）。三个在册脚本顶部都有归一化 shim（`typeof a === "string" → JSON.parse`），所以 `Workflow({ name, args: {...} })` 直接传对象即可，脚本内一律用 `ARGS.xxx` 取值（不要直接 `args.xxx`）。**新写脚本必须照抄这段 shim**，否则 `args.target` 在字符串上取属性返回 undefined → guard 直接报 `args.xxx required`（2026-06-12 踩坑实锤：diag workflow 实测 `typeofArgs:"string"`）。
- 改/加 workflow 后跑语法校验：`node --check`（需先包进 async fn，见 git 历史里的校验片段）。
