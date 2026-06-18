# Helix — 项目指令（每次会话开局必读）

> Helix = 一个 **runtime-agnostic（运行时无关）确定性业务内核基座**。一次实现，当前产品化维护两端：
> **native（Tauri 桌面 / PC）· FFI（Flutter 手机端）**。
> 网页端暂不做；`helix-driver-wasm` 保留为历史实验/归档代码，不进入 roadmap、CI 必跑矩阵和交付验收（见 `docs/harness/decisions.md` ADR-006）。
> IM 只是跑在它上面的**第一个业务模块**；core layer 本身不认识任何业务。
> 概念祖先：Bun（runtime 跑 JS）/ Dioxus core（headless 核跑 6 端）。

> 🧭 **北极星（设计理念不变量，不可偏离）→ [`docs/GOAL.md`](docs/GOAL.md)**　每次会话开局必读。
> 任何改动若与 GOAL.md §1 设计理念 / §2 硬约束冲突 → **先停下问，不偏离**。

> 📋 **实现阶段待办 → [`docs/ROADMAP.md`](docs/ROADMAP.md) §二 P0/P1/P2 活账**
> （原「helix-架构评审-待办清单」已随 docs 熵管理缩减并入 ROADMAP：B 正确性 / C 约束 / D roadmap 项多已落地，剩余开放项见 ROADMAP P 列。wasm/IndexedDB 项已被 ADR-006 取代——当前只维护 PC Tauri + Flutter。）**进实现阶段前先扫一眼 ROADMAP 活账。**

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
| HX-C009 | 预期对账闭环完整性（DRIFT-PRED 真接入 · EXPECT 可证伪 · 偏离不自产自判）| active |
| HX-C010 | 集合迭代→有序 Effect 生产必须确定性有序（有序集合/迭代前 sort，防 SipHash 随机序）| active |

> 索引不变量：本表行数 == `ls docs/harness/HX-C*.md | wc -l`。不等 = 漂移，立即修。

---

## 4.5 行为真源（Bun Zig-as-reference 同款）

实现 / 修复 IM 行为有争议时，**现网 `~/workspace/angular/cses-client/src-tauri/src/features/im/` 对应 handler 是 source of truth**；机制级对齐标注见 [`docs/过会方案/helix-core-im-架构设计.md`](docs/过会方案/helix-core-im-架构设计.md) 各节。

### 4.5.1 csesapi → cses-im-server 迁移全量图谱 + sync-v2 落地真相（接口/事件/用例 索引 · 按需 Read）

> 现网（mattermost csesapi 后端 + cses-client 前端）的**接口/WS 事件/用户用例/渲染逻辑/Tauri 交互**全量映射 + 三态缺口矩阵 + sync-v2 落地状态——做迁移/对齐/排查时的**消费视角真源**。本表只挂索引，正文按需 Read（渐进式披露）。

| 文档 | 视角 | 一句话 |
|---|---|---|
| [`crates/helix-im/docs/sync-v2-能力与缺口状态.md`](crates/helix-im/docs/sync-v2-能力与缺口状态.md) | **落地真相（现在到哪了）** | 三件套引擎 ✅ + sync-v2 ~85% + C1/C2/C3/B2 实现完成；§5 = 今日必办（真 Go 数据路径端到端验证） |
| [`crates/helix-im/docs/csesapi-migration/full-map/00-README-index.md`](crates/helix-im/docs/csesapi-migration/full-map/00-README-index.md) | **全量图谱总索引** | backend-\*（源端 HTTP/WS 清单）+ client-\*（前端用例/渲染/Tauri）+ gap-\* 三类导读 |
| [`crates/helix-im/docs/csesapi-migration/full-map/30-capability-gap-matrix.md`](crates/helix-im/docs/csesapi-migration/full-map/30-capability-gap-matrix.md) | **缺口矩阵** | covered 86 / partial 30 / missing 7 + 替代 mattermost 阻塞能力优先级 |
| `…/full-map/partials/1-…posts` · `2-…channel-sync` · `3-…user-misc` · `4-…bot-agent` | **后端 HTTP** | 124 接口四域全清单 + wire 契约字段 + 命名陷阱 |
| `…/full-map/partials/5--backend-ws-events.md` | **后端 WS 推送** | 19 事件常量真源（`websocket_message.go`）+ `event_seq` 注入铁律 |
| `…/full-map/partials/6--client-usecases.md` · `7--client-ui-rendering.md` · `8--client-tauri-im.md` | **前端用例/渲染/Tauri** | message-v3 service 42 用例 + UI readBits/字段契约 + src-tauri Rust IM 核心 14 用例集 |
| `…/full-map/partials/9..12-gap-*.md` | **三态缺口** | posts/channel/user-misc/bot-agent 各域 covered/partial/missing 对照 |
| [`crates/helix-im/docs/csesapi-migration/01-接口可达性清单.md`](crates/helix-im/docs/csesapi-migration/01-接口可达性清单.md) · [`02-迁移蓝图.md`](crates/helix-im/docs/csesapi-migration/02-迁移蓝图.md) | **可达性 + 蓝图** | 接口可达性逐条 + 迁移蓝图 |

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

## 4.8 预期对账（L1 决策原子 · 试点中，2026-06-17 起）

> 目的：给 §4.7 `architecture-drift-review` 接上「偏离**预期**」这第二根进料管——
> 原本它只吃「偏离**约束**（HX-C/GOAL）」+ commit 计数触发；现加 prediction-error 惊讶信号。
> 试点期 2 周；按校准有效率（`⚡ DRIFT-PRED` → 真 HX-C 卡 / 真修复 占比）决定是否晋升全局 CLAUDE.md。

**触发**（照 §6 实质任务门槛，命中任一才写；typo / 单行 / 纯格式**豁免**，防空转噪声）：
≥3 文件 / 净增删 ≥50 行 / 跨 crate / 改 port 或 driver / 迁移 / 不可逆操作。

**开局**——写一行进 `SESSION.md §0`（EXPECT 必须**可证伪**，堵自评自判漏洞）：

```
🎯 EXPECT <预判结果，至少含 1 条可证伪断言> | 赌注 高/中/低 | 可逆 是/否 | @<ts>
```

- **可证伪断言**举例：预测改动文件集（"只动 parser.rs + sync_session.rs"）/ 数值阈值（"batch_upsert 仍 <6ms"）/ 命名不变量（"不新增 HX-C 违反"）/ 命令 pass-fail（"cargo check -p helix-core 过"）。
- ❌ 禁止 "我觉得能行 / 应该没问题 / 大概顺利" 这类**不可证伪**空话——越具体，DRIFT-PRED 越能被自动检出；写虚 = 自废本机制。

**收尾**——已有 `✅ DONE` = 结果对账方。若实际偏离开局预期，紧跟 DONE 追一行：

```
⚡ DRIFT-PRED <哪里出乎意料> → 候选教训：<一句话>
```

并 `type:surprise` 落 `/workspace/java/logs/{date}.json`。预期命中（无偏离）则不追，`✅ DONE` 即结。

**接力**：`architecture-drift-review` 触发时，除 `<last_reviewed>..HEAD` diff 外，**加扫**上次 review 以来
SESSION §0 的 `⚡ DRIFT-PRED` 行作为 drift 候选输入，对抗证伪后决定是否升 HX-C 卡。

**偏离判定不自产自判（堵 GIGO 后门）**：写 EXPECT 的 agent **不是**「是否偏离」的唯一裁判。`architecture-drift-review`
反向抽检区间内实质 commit：① EXPECT 是否可证伪（模糊化 = 一条 finding）；② DONE 实际是否偏离 EXPECT 却没记
`⚡ DRIFT-PRED`（惊讶被吞没 = 一条 finding）。最终偏离裁定由 drift-review 的独立 lens 给，不由生产者自己说了算。

**回放闸门**（候选教训 → 新建 HX-C 卡前必过）：拿候选约束回放 `docs/harness/log.md` 历史 trap
+ §0 既往 `⚡ DRIFT-PRED`——能挡住当初那次、且不误伤既往成功，才建卡（防约束过拟合 / 膨胀）。

## 5. 代码风格

> 完整结构约束（单文件 ≤300 行 / 设计模式解耦 / 复杂度升序 / DDD 映射 / 每文件夹双文档镜像）见 [`.claude/rules/code-structure-ddd.md`](.claude/rules/code-structure-ddd.md)（按需 Read）。配套闸门 `scripts/structure-gate.sh`（纳入 §4.7 drift lens）。

- 类型驱动：make-invalid-states-unrepresentable；tagged union(enum) 优于一堆 Option/bool；newtype 不透明
- 错误：`thiserror` 边界类型 / 不滥 `anyhow`；外部输入（WS 帧 / sync 响应 / FFI 入参）**零 `unwrap`/`expect`**，走 `Result`
- 命名/模块：高内聚低耦合，many small files；mod.rs 只 re-export
- 注释只写「为什么」(invariant / SAFETY / 三栖偏差)，不写「做什么」

## 6. 提交规范（commit / PR / tag）

**提交代码前必读 → [`docs/COMMIT_CONVENTION.md`](docs/COMMIT_CONVENTION.md)**：Conventional Commits（中文 description/body）+ 触发条件命中时的 5 段结构化 body（影响范围 / 改动影响面 / 功能改进 / Harness 更新 / 验证）+ PR / Tag 策略。本仓有 `docs/harness/` → Harness 一节必填。配套 `scripts/gate.sh` + pre-push hook。

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **helix** (2938 symbols, 7037 relationships, 247 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

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
