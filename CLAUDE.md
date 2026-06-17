# Helix — 项目指令（每次会话开局必读）

> Helix = 一个 **runtime-agnostic（运行时无关）确定性业务内核基座**。一次实现，当前产品化维护两端：
> **native（Tauri 桌面 / PC）· FFI（Flutter 手机端）**。
> 网页端暂不做；`helix-driver-wasm` 保留为历史实验/归档代码，不进入 roadmap、CI 必跑矩阵和交付验收（见 `docs/harness/decisions.md` ADR-006）。
> IM 只是跑在它上面的**第一个业务模块**；core layer 本身不认识任何业务。
> 概念祖先：Bun（runtime 跑 JS）/ Dioxus core（headless 核跑 6 端）。

> 🧭 **北极星（设计理念不变量，不可偏离）→ [`docs/GOAL.md`](docs/GOAL.md)**　每次会话开局必读。
> 任何改动若与 GOAL.md §1 设计理念 / §2 硬约束冲突 → **先停下问，不偏离**。

> 📋 **架构评审待办（实现阶段提醒）→ [`docs/过会方案/helix-架构评审-待办清单.md`](docs/过会方案/helix-架构评审-待办清单.md)**
> 过会评审揪出的必办项：B 正确性遗漏（gate_inflight / event_seq 三级 fallback / type=6 readBits / phantom）· C 约束澄清（Storage scan / Module 同步约束）· D roadmap 调整。注意 D 中 wasm/IndexedDB 相关项已被 ADR-006 取代：当前只维护 PC Tauri + Flutter。**进实现阶段前先扫一眼，防遗忘。**

> 🔁 **指令文件同步铁律**：本仓所有同目录的 `CLAUDE.md` 与 `AGENTS.md` 是镜像文件；任何修改必须同时落到两者，内容保持完全同步。新增 / 删除某一级指令文件也必须成对处理。

---

## 0. 三层架构（铁律，不可变）

```
业务模块层   [helix-im] [将来 presence / notify ...]   只依赖 helix-core 的 ports，不碰 tokio/sqlx/web-sys
    ▲ 实现 Module trait + register()
core layer   ★ helix-core ★   ports(Transport/Storage/Clock/EventSink/IdSource) + Module 宿主 + Effect/事件 + sans-IO 执行壳
    ▲ 实现 ports（依赖方向单向：adapter → core，永不反向）
平台驱动层   helix-driver-host(shared rusqlite Storage/StorageOp) · helix-driver-native(PC/Tauri 壳)
             helix-driver-ffi(Flutter FFI 壳)
             helix-driver-wasm = archived/experimental（网页端暂停维护，不作为交付目标）
```

**依赖铁律**：`helix-core` 永不直接 import `tokio` / `sqlx` / `reqwest` / `web-sys` / `tauri` —— **只定义 trait**。
驱动在边缘实现。违反 = 双端复用立刻碎。验证见 harness `HX-C001`。

---

## 1. 核心开发理念

1. **Sans-IO / 六边形**：core 不做任何 I/O、不持有任何运行时。纯状态机：喂入(入站帧 / now_ms / 端口响应) → 吐出(Effect: Persist/Send/ScheduleTimer/Emit)。真正的网络/存储/线程由驱动做。
2. **确定性**：时间 / 随机 / ID 一律由 host 注入（不在 core 调 `Instant::now` / `rand` / 生成 id）。同输入同输出，可回放、可测试。
3. **并发不进 core**：tokio multi-thread / Flutter FFI 宿主线程模型都属于驱动层。core 必须能在单线程跑。
4. **YAGNI 但边界要对**：通用性靠「≥1 真实模块跑通 + 第二模块插得进」挣，不预建插件框架。但 **YAGNI 边界必须在「PC Tauri + Flutter 手机端目标集合」下画** —— 单平台假设仍会砍错双端必需的抽象（见 harness `HX-C001`/`HX-C002` 教训）。

---

## 2. 双端边界条件 checklist（写 core / 改 port / 加 driver 前过一遍）

> 完整规则 + 代码 + 坑词典见 [`.claude/rules/runtime-agnostic-core.md`](.claude/rules/runtime-agnostic-core.md)（按需 Read）。
> 下面是开局必记的速查：

- [ ] **helix-core 零运行时依赖**：grep 不出 tokio/sqlx/reqwest/web-sys/tauri（`HX-C001`）
- [ ] **async 抽象 Send 边界**：native 可 `Send+'static`，FFI/单线程宿主可能 `!Send` → cfg 切 / 不硬加 Send（`HX-C002`）
- [ ] **状态用 owned/Copy/arena id，不用 `&'a`**：要进 async/闭包别带生命周期（Dioxus `Scope<'a>` 地狱 + 本仓 `Tick<'a>` blocker 教训，`HX-C002`）
- [ ] **wasm/web 暂停维护**：不要新增或修复 `helix-driver-wasm` 作为交付项；若未来重启网页端，先新增 ADR 重新激活 wasm CI 与 HX-C003（当前 HX-C003 仅作为历史护栏保留）。
- [ ] **features 必须 additive**：禁互斥 feature（feature unification 会爆）；optional dep + `[target.'cfg(...)'.dependencies]`（`HX-C004`）
- [ ] **热路径零分配 O(1)**：`&'static str` 不 `String`、复用 buffer、单 INSERT/UPDATE WHERE PK、禁 read-modify-write（`HX-C005`）
- [ ] **FFI 边界铁律**：不许 unwind 跨 `extern "C"`——策略二选一：生产 cdylib `panic="abort"`（无需 catch_unwind）/ dev unwind 时逐导出 `catch_unwind`，禁两者混说；opaque ptr 单一 owner（`Box::into_raw`/`from_raw` 各一次）；同步 SQLite C 调用包 `spawn_blocking`（`HX-C006`）
- [ ] **Effect 跨 FFI 边界批处理**：FFI 边界**批量**过 effect，不逐条（Dioxus sledgehammer 教训，`HX-C007`）

---

## 3. 编译 / 测试 cadence

- 改完一个完整模块 / commit 前：`cargo check -p <crate>` 一次
- **双端验收**（phase 完成前 / 改 port 或 driver 后必跑）：
  ```bash
  cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
  cargo test -p helix-core
  ```
- `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` → **必须空**（零运行时证明）
- **autonomous / sub-agent 模式**：自跑验证并回报；**单任务 prompt 模式**：输出命令让用户决定（除非用户说"你跑"）

---

## 4. 长运行任务约束 / Harness Engineering

> 本项目长运行任务（跨会话 / 跨 Phase / autonomous 模式）的工程化约束体系。
> 完整方法论见 [`/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md`](/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md)
> （**不要把全文拷进本 CLAUDE.md** —— 按下方触发条件**按需** Read）。

**三个即时同步铁律**：
1. 踩坑 → 即时写 `/workspace/java/logs/{YYYY-MM-DD}.json`
2. 同根因 ≥ 3 次 / 用户明确 / Spec 拍板 → 即时新建 `docs/harness/HX-C{NNN}-*.md`（同会话内）
3. harness/ 增删 → 即时同步本表 + `docs/harness/README.md` + 一次 commit

**必须 Read 完整手册的触发**：autonomous 任务开始前（协议 P1-P5）/ 第一次建 harness（§4 模板 + §5 触发器）/ 升级 merged·deprecated（§3.3）/ 用户问"怎么不再踩同一个坑"（§0 + §8）。

**在册 harness 索引**（详见 [`docs/harness/README.md`](docs/harness/README.md)）：

| 编号 | 标题（一句话） | 状态 |
|---|---|---|
| HX-C001 | helix-core 零运行时 / 零平台依赖（grep gate）| active |
| HX-C002 | async port Send 边界 + 状态禁 `&'a`（双端 + 生命周期）| active |
| HX-C003 | wasm-only 代码必须进 CI type-check + compile_error 守护 | archived（wasm 暂停维护，见 ADR-006） |
| HX-C004 | features 必须 additive，禁互斥 | active |
| HX-C005 | 热路径零分配 O(1)（`&'static str` 不 String）| active |
| HX-C006 | FFI 边界铁律（no unwind / 单 owner / spawn_blocking）| active |
| HX-C007 | Effect 跨 FFI 边界批处理，不逐条 | active |
| HX-C008 | sync/notify pull 契约：cursor 锚 nextSeq（非 max event）+ 续拉看 len≥500（真 wire 无显式 flag）| active |

> 索引不变量：本表行数 == `ls docs/harness/HX-C*.md | wc -l`。不等 = 漂移，立即修。

---

## 4.5 行为真源（Bun Zig-as-reference 同款）

实现 / 修复 IM 行为有争议时，**现网 `~/workspace/angular/cses-client/src-tauri/src/features/im/` 对应 handler 是 source of truth**；机制级对齐标注见 [`docs/过会方案/helix-core-im-架构设计.md`](docs/过会方案/helix-core-im-架构设计.md) 各节。

## 4.6 专家 Agent / Skill 注册表（按需调用 · 渐进式披露）

> 完整调度规则 + 编制配方见 [`.claude/rules/expert-dispatch.md`](.claude/rules/expert-dispatch.md)（按需 Read）。
> 阶梯：**L0 就近文件 → L1 Skill 拉知识 → L2 单 Agent 派活/审查 → L3 Workflow 编制**——能低不高，升级要有触发条件。

| 专家 Agent ↔ Knowledge Skill | 领域一句话 | 何时调 |
|---|---|---|
| `rust-dev` ↔ `/rust-knowledge` | Rust 语言与工程全栈（所有权/async/性能/unsafe） | 借用·Send 报错、热路径审查、trait 设计争议 |
| `rust-tauri-skeleton-dev` ↔ `/rust-tauri-skeleton-knowledge` | Tauri 2 桌面骨架端到端，**参考仓 = 现网 cses-client** | 现网对齐核验（唯一权威）、IPC 契约、M2 Tauri 接缝 |
| `rust-runtime-agnostic-core-dev` ↔ `/rust-runtime-agnostic-core-knowledge` | 一份 core 编 native+FFI（cfg/Send/uniffi；wasm 已归档） | 双端边界守门、Flutter FFI 主力 |
| `dioxus-dev` ↔ `/dioxus-knowledge` | **仅取 core 内核经验**（step 循环/Effect 批处理/调度），**禁渲染层/RSX 话题** | 内核循环设计对照、批次边界、调度公平性 |

本仓操作手册 Skill（先手册后知识）：`/helix-new-module` · `/helix-replay-test` · `/helix-stress-verify`。

## 4.7 架构 review 收敛节律（强制行为）

> 完整规则见 [`.claude/rules/agent-review-cadence.md`](.claude/rules/agent-review-cadence.md)（按需 Read）。台账 [`docs/review/workflow-review-ledger.md`](docs/review/workflow-review-ledger.md) · 闸门 `scripts/review-gap.sh`。

防 agent/workflow 自主改动「跑着跑着偏离架构」无人察觉：**自主改动每累计 10 个 commit，在 commit 检查点过一次架构 review 收敛。**

- **不变量**：`gap = git rev-list --count <last_reviewed_commit>..HEAD`；`gap ≥ 10` → 🔴 强制先派 `Workflow({name:"architecture-drift-review"})`（查 HX-C001..C008 + GOAL §1/§2 + 全流程合理性 → 对抗证伪 → drift 判定），review 通过 + 更新 ledger `last_reviewed_commit` 后，方可继续 feature 开发。
- **检查点（只在此判定，绝不中途任务触发）**：一批 commit 完成+工作树干净后（主）/ 开局新会话 / 派新 feature workflow 前 → 跑 `scripts/review-gap.sh`（退出码 1 = 强制 review）。命中**可自动派**（无需问用户，token 高性价比）。
- **铁律**：review 通过必须更新 ledger（否则 gap 永不归零 = 闸门失效）；🔴 重 drift 先修复回正再把 ledger 指向已收敛 commit。

## 5. 代码风格

- 类型驱动：make-invalid-states-unrepresentable；tagged union(enum) 优于一堆 Option/bool；newtype 不透明
- 错误：`thiserror` 边界类型 / 不滥 `anyhow`；外部输入（WS 帧 / sync 响应 / FFI 入参）**零 `unwrap`/`expect`**，走 `Result`
- 命名/模块：高内聚低耦合，many small files；mod.rs 只 re-export
- 注释只写「为什么」(invariant / SAFETY / 三栖偏差)，不写「做什么」

## 6. 提交规范（commit / PR / tag）

**提交代码前必读 → [`docs/COMMIT_CONVENTION.md`](docs/COMMIT_CONVENTION.md)**：Conventional Commits（中文 description/body）+ 触发条件命中时的 5 段结构化 body（影响范围 / 改动影响面 / 功能改进 / Harness 更新 / 验证）+ PR / Tag 策略。本仓有 `docs/harness/` → Harness 一节必填。配套 `scripts/gate.sh` + pre-push hook。

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **helix** (3096 symbols, 6567 relationships, 239 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "main"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/helix/context` | Codebase overview, check index freshness |
| `gitnexus://repo/helix/clusters` | All functional areas |
| `gitnexus://repo/helix/processes` | All execution flows |
| `gitnexus://repo/helix/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
