# PRD — helix-im-testbed：三端自动化测试轻量 IM 客户端

> 状态：ready-for-agent（issue tracker 未配置，见文末「发布说明」）
> 来源：2026-06-24 grilling 会话综合（12 决策点 + 4 项收尾澄清）
> 关联：helix 接管方案 `crates/helix-im/docs/takeover-plan/` · 投影契约 `crates/helix-im/docs/projection-schema.md` · outbound 真源 `crates/helix-im/docs/csesapi-migration/真机curl真源.md`

---

## Problem Statement

现网 cses-client 没办法做自动化测试。根因全在 UI/Tauri 那一层的耦合：single-instance 进程锁、窗口池 hide-on-close 的跨用例状态污染、overlay 子进程无条件 spawn、WS 生命周期被 Rust AuthState 独占、20+ 散落 RxJS Subject 无单一真源、SurrealDB 无事务回滚。结果是：验证 IM 行为只能靠人工点击、靠猜，或者只能从代码层（单测）层面验证——**跑起来的真实 app + UI 这一层始终没有自动化覆盖**。

而 helix 已经把业务逻辑从前端抽干净了：前端降级为纯渲染层，89 出站/写命令 + 19 WS live 事件 + 21 个 `im:__bus__` 投影工厂（`projection-schema.md` 已冻结 + 强制对账测试），core 确定性可回放（45+ replay 单测 + 真 Go smoke 8/8 wire-correct）。**缺的不是代码层验证，是"真实 Tauri 外壳 + 真实 DOM 渲染"这一整层的自动化**，以及把它接成「自动测试 → 自动修复 → 自动验证」闭环的能力。

## Solution

做一个 **UI 精简、但功能五脏俱全的轻量 IM 成品客户端**（工作名 `helix-im-testbed`），它：

1. **内嵌真实 helix 引擎**（`helix-driver-native` + `helix-im`）作为 Tauri Rust 后端——测试验证的是 helix↔go 本身，与接管生产路线同源。这同时成为 **helix M2 Tauri 四接缝的首个落地实现**。
2. **UI 用简化语义 DOM**：一条消息 = 一个带 `data-*` 属性的节点，属性直映投影字段，不追求像素/组件保真。
3. **被测性是一等公民**：单窗口、无窗口池、无 overlash 子进程、debug 构建内嵌 `tauri-plugin-webdriver`（跨平台 W3C WebDriver server，mac 已验证），WebdriverIO 直连真实 WKWebView 跑端到端。
4. **四面契约断言**作 oracle，**双轨确定性源**（金标帧日常 + 真 go 夜间）喂自动修复闭环。

它**不取代 cses-client**：cses-client = 生产渲染宿主，helix-im-testbed = 最小可测宿主，两者消费同一个 helix 引擎 + 同一份冻结 `projection-schema.md`，testbed 是「契约一致性自动化夹具」，cses-client 是「生产渲染端」，同在冻结契约下游，不重复不打架。

## User Stories

1. 作为 helix 引擎开发者，我想要一个能 push-button 跑起来的真实 Tauri app，以便验证「投影 → DOM 渲染」「DOM 操作 → 出站命令」这两个接缝，而不是只能跑 headless 单测。
2. 作为 helix 引擎开发者，我想在改完 helix Rust 后一条命令重建 testbed 并重跑端到端，以便拿到「真实客户端栈是否仍正确」的即时反馈。
3. 作为测试工程师，我想要每个 UC 一份声明式的四面期望文件（出站命令体 / 投影 envelope / DOM data-* / DB 落库行），以便机器逐面断言、任一面偏离即 fail。
4. 作为测试工程师，我想要每个 UC run 产出结构化 JSONL hop 事件日志（带 run_id / uc_id / corr_key / facet / hop / payload），以便把四面按 corr_key 聚成一束并和期望 diff。
5. 作为测试工程师，我想要一个 reducer 把四面 diff 成「断在哪一跳（出站/ws-recv/投影/落库/DOM）」的报告，以便不用人工啃原始日志就能定位。
6. 作为自动修复 agent，我想读「断在哪一跳」的 diff 报告，以便精准定位是 helix 哪一层错并去改实现。
7. 作为自动修复 agent，我被允许改 helix 引擎实现（Rust）和 testbed 渲染壳，但**不允许改冻结契约**（projection-schema / 真机curl真源 / golden tape / 四面期望文件），以便红转绿只能靠改实现而非 gaming oracle。
8. 作为自动修复 agent，当我判定是契约本身过时（go 真改了 wire）时，我不自己改契约，而是产出「契约变更提案 + 证据」交人审，以便契约变更始终有人把关。
9. 作为测试工程师，我想要双轨确定性源：日常闭环跑录好的金标帧（确定性·秒级·agent 不被 go flaky 干扰），真 go 跑夜间/按需（专抓 go 后端行为漂移）。
10. 作为测试工程师，我想要金标帧经过真实 Tauri + WKWebView DOM + WebdriverIO（只是输入端用录好的 go 帧），以便它验的是整个客户端栈，而不是又一次 headless 代码层验证。
11. 作为测试工程师，我想要在每个 L1 UC 开跑前先等到「就绪/静默」（`increment_channel_end` 收齐 + HTTP inflight==0 + cursor 稳），以便窗口内的帧/投影能干净归属到这个 UC。
12. 作为测试工程师，我想要用领域键（channelId + tmp_id/server_id + event_seq）在静默窗口内把四面同一逻辑事件精确绑定，以便不改协议就能认领异步 WS 推送。
13. 作为产品负责人，我想要先用「发消息 round-trip」一条竖切打通整台机器（四面 + 装饰器录放 + tmp_id 认领 + 自动修复闭环），以便规模化前先验透 walking skeleton。
14. 作为产品负责人，我想要竖切验证通过后按 UC 顺序铺开，逐步覆盖全部 HTTP+WS 与命令集（89 命令 / 19 WS 事件 / 27+ UC 台账），以便最终五脏俱全。
15. 作为发送者，我想在输入框打字并点发送，以便消息走 UI→invoke send→出站 HTTP→go→WS post echo→parser→gate→tmp_id→server_id 对账→投影→DOM 行 + DB 行整条链路。
16. 作为发送者，我想看到我发的消息以 temporaryId 乐观上屏，并在 echo 回流后用 server_id 覆写，以便验证 tmp_id→server_id 对账。
17. 作为接收者，我想要别人发的消息经 WS post 帧落到我的 message 表并上屏，以便验证在线推送链路。
18. 作为重连后的用户，我想要离线期间的消息经 sync 回放补齐（含 phantom 不落但 cursor 推进、已读位、编辑、撤回），以便验证离线同步（注：作为后续 UC，非竖切）。
19. 作为 helix 维护者，我想要 testbed 独立成 repo、dev 期 Rust 用本地 path dep 指向 helix checkout，以便自动修复循环秒级、release 再 pin git rev 求隔离。
20. 作为 helix 维护者，我想要仪表化用 `Recording<P>` 装饰器包在 port 边界（独立 crate `helix-driver-instrument`），以便 helix-core/im/driver-native 零改、sans-IO 守住。
21. 作为 helix 维护者，我想要金标帧录制/回放只活在测试侧（test-only），以便 production 构建不带任何录放代码、不带 webdriver server。
22. 作为开发流程的使用者，我想要 loop 驱动器（helix-loop-engine / Workflow / /loop）协助全自动开发，跨「改 helix 仓 → 重建 testbed 仓 → 重跑 → 读 diff」两 repo 协调闭环。

## Implementation Decisions

### 模块

- **新 repo `helix-im-testbed`**（独立仓）：
  - 前端：Angular 20 + Tauri 2 薄壳，抄 `chat-virtual-demo` 已验证的 `tauri-plugin-webdriver` + WebdriverIO 接线，渲染换成 IM 语义 DOM（消息行 `data-msg-id` / `data-read-bits` / `data-event-seq` / `data-send-status` 等直映投影字段）。
  - `src-tauri`：内嵌 `helix-driver-native` + `helix-im` 作 Tauri 后端；dev 用本地 path dep（`{ path = "../helix/crates/..." }`），release pin git rev。debug 构建注入 `tauri-plugin-webdriver`（监听 4445，release 绝不带）。
- **新 crate `helix-driver-instrument`**（建议落在 helix workspace 或 testbed repo，只依赖 helix-core 的 port trait）：提供 `Recording<P>` 装饰器 + tape 读写 + LogSink。
- **helix-core / helix-im / helix-driver-native：零改**（守 HX-C001 sans-IO；装饰器只在组装根 debug 构建包一层）。

### 主缝 = port 装饰器（唯一新缝，最高点）

四面要看的每一面都已是 helix 的一个 port，仪表化 = 用 Decorator 模式包现有 port：

| 面 | port | 装饰器职责 |
|---|---|---|
| ① 出站命令体 + 录/放 go 帧 | `Transport` | Live 透传真 go + tee 日志；Record 旁路存 tape；Replay 不调 inner、从 tape 喂 |
| ② 投影 envelope | `EventSink` | tee 日志（投影抵达 Tauri emit 之前就抓到，比 IPC 层更早全） |
| ④ DB 落库行 | `Storage` | tee 日志（或直接查库，二选一） |
| 回放确定性 | `Clock` / `IdSource` | Replay 喂录好的时钟/id，helix 才能字节级复现 |
| ③ DOM | （非 port）WebdriverIO | 复用 `tauri-plugin-webdriver` 内嵌 W3C server 拿 DOM |

装饰器形状（来自 grilling 原型草图，仅记决策不是可运行 demo）：

```rust
enum Mode { Live, Record, Replay }
struct Recording<P> { inner: P, sink: LogSink, mode: Mode, tape: Option<Tape> }
// impl Transport for Recording<P>：Live 透传+tee；Record +存 tape；Replay 从 tape 供、不触 inner
// EventSink / Storage / Clock / IdSource 各一个同款 impl
```

组装根（testbed src-tauri 的 EngineConfig 拼装处，debug 构建）把真实 port 用 `Recording` 包一层再交给 engine；production 交裸 port。

### oracle = 四面契约断言

- 每 UC 一份**声明式期望文件**，同一 `corr_key` 串起四面：① 出站命令体（vs `真机curl真源.md`）② 投影 envelope（vs `projection-schema.md`）③ DOM data-* 状态 ④ DB 落库行。任一面偏离即 fail。
- **认领策略（L1）**：串行 + 静默窗口 + 领域键。
  - 静默/就绪 probe = `increment_channel_end` 收齐 **且** HTTP inflight==0 **且** cursor 稳（cursor 稳是最后确认步）。
  - 窗口内所有帧/投影归该 UC；窗口内再用领域键（channelId + tmp_id/server_id + event_seq）把四面同一逻辑事件精确绑定。不改协议。

### 双轨确定性源

- **金标帧（日常·确定性）**：先真 go 跑一次，经 `Transport`/`Clock`/`IdSource` 装饰器把 go 回的 WS/HTTP 帧 + 时钟/id 录成 tape（落 `tests/fixtures/...`）；之后回放 tape，经真实 Tauri + WKWebView + WebdriverIO 跑——验整个客户端栈，输入端确定性。**顺手把 helix 仓 P1 golden fixtures 工具链做掉。**
- **真 go（夜间/按需）**：连真 go 抓后端行为漂移。
- 金标帧化 **test-only**，production 不带。

### 自动修复护栏

- **引擎可改 · 契约只读 · 改契约需人审**：agent 只能改 helix 引擎实现（Rust）+ testbed 渲染壳；冻结契约（projection-schema / 真机curl真源 / golden tape / 四面期望文件）一律只读。判定契约过时 → 出「契约变更提案 + 证据」交人审，不自改。对齐 HX-C009（预期对账闭环）/ HX-C011（测试可证伪）/ HX-C012（开局写 EXPECT）。

### 日志

- 结构化 **JSONL hop 事件**：每个 port 装饰器在每跳吐一行 `{run_id, uc_id, corr_key, facet, hop, payload}`；**reducer** 把同 corr_key 四面聚成一束 + 与期望 diff，出「断在哪一跳」报告（hop ∈ outbound / ws-recv / projection / storage / dom）。原始流保留备深挖。agent 读 diff 报告，不啃原始日志。

### loop 编排

- 由 loop 驱动器（`helix-loop-engine` skill / Workflow / `/loop`）协助全自动开发：跨「改 helix 仓 → 重建 testbed 仓 → 重跑端到端 → 读 reducer diff → agent 改实现 → 重跑」闭环；编排须知道两 repo（helix checkout + testbed）。

### 落地顺序

1. **竖切：发消息 round-trip**——打通整台机器（四面 + 装饰器录放 + tmp_id 认领 + 自动修复闭环 + JSONL/reducer），helix 这条已 8/8 wire-correct、金标帧好录。
2. 竖切验证通过 → **按 UC 顺序铺开**，逐步覆盖全部 HTTP+WS + 命令集（89 命令 / 19 WS 事件 / 27+ UC），逐 UC 金标帧化 + 四面期望文件。

## Testing Decisions

- **好测试的定义**：只断言外部行为（四面 = 出站命令体 / 投影 envelope / DOM / DB 落库行），不断言实现细节；断言锚在**冻结契约**（projection-schema / 真机curl真源），不锚实现内部结构。每个测试可证伪（HX-C011）：破坏即 fail，禁墙钟下界断言，用结构化/计数证不变量。
- **被测模块**：helix 引擎（`helix-im` + `helix-driver-native`）经真实 Tauri + WKWebView 端到端；`helix-driver-instrument` 装饰器自身的录/放/透传正确性单测。
- **测试形态（v1 = L1）**：per-UC 契约层——串行 + 静默窗口 + 领域键认领 + 四面断言；日常跑金标帧（确定性），真 go 夜间。
- **prior art（仓内已有同类）**：
  - `crates/helix-im/tests/`（剧本工具：构造 ImModule → 喂 Tick 序列 → 断言 Effect 序列）——四面里「投影/出站」面的 headless 前身。
  - `crates/helix-im/tests/projection_schema_test.rs`（投影 envelope 强制对账）——②面的契约源。
  - `crates/helix-im/docs/csesapi-migration/真机curl真源.md`——①面的契约源。
  - `chat-virtual-demo`（`wdio.conf.mjs` + `test/specs/*.e2e.mjs` + `tauri-plugin-webdriver`）——③面 DOM 驱动的 prior art（mac 已验证）。
  - 当前分支 `feat/cses-round3-conformance-e2e` 的 27UC 真 Go e2e 串行台账——UC 铺开顺序与认领串行性的 prior art。

## Out of Scope

- **L2 并发场景层**（冷启动风暴 / 跨频道交错收发 / 重连补洞的不变量断言）——v1 跳过，L1 闭环稳定后再叠（不变量：cursor 每频道单调 / inflight 有界 / 不丢帧 / 终态收敛 / 无死锁）。
- **真实 message-v3 组件渲染保真**（像素/组件级）——由组件单测 / `chat-virtual-demo` 虚拟滚动 demo 覆盖，testbed 只做简化语义 DOM。
- **多后端可切换抽象**——只有一个后端（go）；「轻量级消息系统」即本产品自身，非要打的第二后端。
- **取代 cses-client 生产渲染**——testbed 与 cses-client 并存、共享冻结契约。
- **release 构建带 webdriver / 录放**——一律 test/debug-only。

## Further Notes

- **与 helix 路线对齐**：testbed = M2「Tauri 四接缝接入」首个落地处（helix P0 活账）；金标帧工具链 = helix P1 golden fixtures 工具链。两个开放项被本 PRD 顺手收口。
- **架构守恒证明**：净增 1 条新缝（port 装饰器）+ 复用 1 条 WebDriver 缝；helix-core/im/driver-native 零改，HX-C001 sans-IO 守住，HX-C004 additive（装饰器 feature/构建门控），HX-C009/C011/C012 落到修复闭环护栏。
- **命名**：`helix-im-testbed` 为工作名；它是一个真实轻量 IM 成品（五脏俱全），可后续正式命名。

---

## 发布说明

本环境未配置 issue tracker 与 triage label 词表（需 `/setup-matt-pocock-skills`）。本 PRD 已落盘为文档；如需发到 tracker 并打 `ready-for-agent`，配置后我可补发。
