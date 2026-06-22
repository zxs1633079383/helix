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
| `host-cli-go-e2e` | 驱动-断言 + 失败诊断 | host-cli 真连 Go 跑 13-action e2e（包 `scripts/host-cli-go-e2e.sh`）：逐 action 经 host-cli stdin 发命令→grep `HELIX_DUMP_FRAMES` 入站帧的 WS action 断言→PASS/FAIL 表；失败则诊断 agent 读日志定位根因。**前置：Go(:8065)+grpc up**（外部状态，workflow 起不了）。验的是共享 driver-host I/O 栈 + wire 契约（跑一次，native/ffi 复用，端增量靠 smoke.cpp+F5） | `{ channelId(必), cookieId(必), companyId?, closeChannelId?, createBody?, wsUrl?, apiBase?, settleMs? }` |

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

## Plan Gate（spec-before-edit · 动手前先出 spec 块 · 对标 Trend 1）

> 来源：`2026 Agentic Coding Trends Report` **Trend 1「The software development lifecycle changes dramatically」**——agentic SDLC = `Express intent → Agent understands →【显式 spec】→ Agent implements → Human review`，关键差异「**Human guides, agent executes**」。配套本仓 `spec-driven-development` skill（SPECIFY→PLAN→TASKS→IMPLEMENT 门控）。
> 核心一句话：**任何会改文件的 workflow 阶段，动手前必先产出一个 schema 强制的 spec 块**——把「Agent understands」固化成可审的工件，卡在「implements」之前，让 human 在对的高度 guide/review。

### 铁律

- **file-modifying 阶段 = 必须有 Plan gate**：任一 phase 的 agent 会 Edit/Write 文件 → 该 phase **之前**必须有一个 **Plan 阶段**，agent **只读+规划、绝不改文件**，按 `PLAN_GATE_SCHEMA` 产出 spec 块。spec 块 schema 强制（缺字段 = 该 agent FAIL），是可 grep 的工件（`/workflows` + 最终输出可见）。
- **read-only workflow 豁免**：只查/只审/只跑测的不改文件 → 无需 plan gate。在册中 `architecture-drift-review`、`expert-panel-review`、`stress-verify`、`host-cli-go-e2e`（只跑脚本不改 src）**豁免**；`dual-track-fix`、`refactor-spec-then-build`、以及任何 implement/fix/refactor 编制**必须**有。
- **openQuestions 非空 → 不动手**：spec 块里若有「需人类拍板」的未决项，该分片**停在 plan、不进 implement**，回主对话（对齐「Human guides」——歧义先澄清，不默默填）。
- **implement 阶段按已出 spec 实现**：fix/build agent 拿到自己刚出的 spec，严格按 `filesToChange + approach` 改；偏离 spec 要在返回里说明（spec 是活契约，不是一次性摆设）。

### `PLAN_GATE_SCHEMA`（可复用·新 file-modifying 编制照抄）

字段对齐 `spec-driven-development` skill 的 6 核心域 + 任务模板（Files/Acceptance/Verify）：

| 字段 | 含义（对应 spec skill） |
|---|---|
| `objective`（必）| 改什么 + 为什么（理解到的 intent，可证伪）= Objective |
| `filesToChange[]{file,what}`（必）| 改哪些文件·每文件改什么（须在白名单内）= 任务模板 Files |
| `approach`（必）| 怎么实现（技术方案，不是做什么）= Plan |
| `assumptions[]` | 浮出的假设（默默填=最危险）= skill「先列假设」|
| `acceptance[]{criterion,verify}`（必）| 可测成功条件 + 验证命令 = Acceptance/Verify |
| `boundaries` | 绝不碰什么 + 风险 = Boundaries「Never do」|
| `openQuestions[]` | 需人类拍板的（非空→回主对话）= Open Questions |

> 参考实现：`dual-track-fix.workflow.js`（Plan→Fix→Review→Repair，Plan 阶段 schema 强制、openQuestions 非空则 `blocked-on-questions` 不动手）；`refactor-spec-then-build.workflow.js`（Spec→Review→Build，先出 spec.md+mmd 再实现）。
> 主对话（非 workflow）侧的等价物 = `spec-driven-development` skill + 计划模式：出 spec → 用户 review → 再改。

## 约束与边界

- workflow **不做 git merge / commit**（项目 §6 铁律：merge 由主对话做前置冲突分析后执行）。`dual-track-fix` 只产出 reviewed 改动 + merge-plan，主对话跑 `scripts/gate.sh` 终验后分 commit。
- 编译/测试 cadence：workflow agent = autonomous 模式，**自跑验证**（根 `CLAUDE.md §3`）；与主对话单任务 prompt「输出命令让用户决定」不冲突。
- 脚本无 `Date.now()/Math.random()`（会 break resume）；按 index/label 区分 agent。
- **⚠️ args 以 JSON 字符串注入**（本运行时实测 `typeof args === "string"`，非已解析对象）。三个在册脚本顶部都有归一化 shim（`typeof a === "string" → JSON.parse`），所以 `Workflow({ name, args: {...} })` 直接传对象即可，脚本内一律用 `ARGS.xxx` 取值（不要直接 `args.xxx`）。**新写脚本必须照抄这段 shim**，否则 `args.target` 在字符串上取属性返回 undefined → guard 直接报 `args.xxx required`（2026-06-12 踩坑实锤：diag workflow 实测 `typeofArgs:"string"`）。
- 改/加 workflow 后跑语法校验：`node --check`（需先包进 async fn，见 git 历史里的校验片段）。
