# Helix — 项目指令（每次会话开局必读）

> Helix = 一个 **runtime-agnostic（运行时无关）确定性业务内核基座**。一次实现，产品目标集合为三端：
> **native（Tauri 桌面 / PC）· FFI（Flutter 手机端）· Web**。
> 当前交付事实：native/FFI 已产品化；Web 的 A1-A5 Rust 平台壳、最小浏览器 glue、正式 gate 与真浏览器→真 Go 产品链均已落地并完成本地环境验收。新的三端 Rust 业务门面与平台适配架构已于 2026-07-14 拍板，当前决策以 [`docs/harness/decisions.md`](docs/harness/decisions.md) 的 ADR-014 与 [`docs/ROADMAP.md`](docs/ROADMAP.md) M4 为准；原 W0 浏览器探针与 W0→W4 路线继续失效，历史 `helix-driver-wasm` 已删除。
> IM 只是跑在它上面的**第一个业务模块**；core layer 本身不认识任何业务。
> 概念祖先：Bun（runtime 跑 JS）/ Dioxus core（headless 核跑 6 端）。

> 🧭 **北极星（设计理念不变量，不可偏离）→ [`docs/GOAL.md`](docs/GOAL.md)**　每次会话开局必读。
> 任何改动若与 GOAL.md §1 设计理念 / §2 硬约束冲突 → **先停下问，不偏离**。

> 📋 **实现阶段待办 → [`docs/ROADMAP.md`](docs/ROADMAP.md) §二 P0/P1/P2 活账**
> （原「helix-架构评审-待办清单」已随 docs 熵管理缩减并入 ROADMAP：B 正确性 / C 约束 / D roadmap 项多已落地，剩余开放项见 ROADMAP P 列。Web 实施路线见 ADR-014 + ROADMAP M4 A1-A5。）**进实现阶段前先扫一眼 ROADMAP 活账。**

---

## 0. 三层架构（铁律，不可变）

```
业务模块层   [helix-im] [将来 presence / notify ...]   只依赖 helix-core 的 ports，不碰 tokio/sqlx/web-sys
    ▲ 实现 Module trait + register()
core layer   ★ helix-core ★   ports(FrameSender/Storage/Clock/EventSink/IdSource) + Module 宿主 + Effect/事件 + sans-IO 执行壳
    ▲ 实现 ports（依赖方向单向：adapter → core，永不反向）
平台驱动层   helix-driver-host(shared rusqlite Storage/StorageOp) · helix-driver-native(PC/Tauri 壳)
             helix-driver-ffi(Flutter FFI 壳)
             helix-driver-web(Rust Browser Fetch + session store + 浏览器 WS bridge/glue)
```

**依赖铁律**：`helix-core` 永不直接 import `tokio` / `sqlx` / `reqwest` / `web-sys` / `tauri` —— **只定义 trait**。
驱动在边缘实现。违反 = 多端复用立刻碎。验证见 harness `HX-C001`。

---

## 1. 核心开发理念

1. **Sans-IO / 六边形**：core 不做任何 I/O、不持有任何运行时。纯状态机：喂入(入站帧 / now_ms / 端口响应) → 吐出(Effect: Persist/Send/ScheduleTimer/Emit)。真正的网络/存储/线程由驱动做。
2. **确定性**：时间 / 随机 / ID 一律由 host 注入（不在 core 调 `Instant::now` / `rand` / 生成 id）。同输入同输出，可回放、可测试。
3. **并发不进 core**：tokio multi-thread / Flutter FFI 宿主线程模型都属于驱动层。core 必须能在单线程跑。
4. **YAGNI 但边界要对**：通用性靠「≥1 真实模块跑通 + 第二模块插得进」挣，不预建插件框架。但 **YAGNI 边界必须在「PC Tauri + Flutter + Web 目标集合」下画**——共享的是协议/业务语义，不是强迫三端复制相同的持久化数据量或物理连接实现（见 ADR-014）。
5. **可观测性逐跳完整（多跳诊断前提·强约束）**：事件穿过的**每个 port/层边界**（入站帧 → core → Effect 出站/Emit → 渲染）必须有结构化 hop 探针（corr_key 串联）。**任何绕过装饰器 / `Recording` 的旁路（`Tick::Inbound` 直泵 / `Tick::Command` / 内核自驱 effect）新增即同步补探针**——缺一跳 ≠ 丢 1/N 信息，而是丢掉**大部分**诊断力（歧义横跨多假设）。**铁律：诊断难度与逻辑复杂度不成比例时，先怀疑可观测性盲点，再怀疑逻辑。** 真实代价 + 机制 + 可执行验证见 loopforge harness `docs/harness/C015-observability-hop-coverage.md`（2026-06-29 echo gate-gap：WS 入站走 `Tick::Inbound` 绕过 `Recording::recv` tee → ②盲点 → send-echo 误诊 5 agent → 补 HOP2/HOP3 后同类一遍定位）。

---

## 2. 多端边界条件 checklist（写 core / 改 port / 加 driver 前过一遍）

> 下面是开局必记的速查；可执行护栏见对应 `HX-C00*` 卡与 `scripts/gate.sh`。

- [ ] **helix-core 零运行时依赖**：grep 不出 tokio/sqlx/reqwest/web-sys/tauri（`HX-C001`）
- [ ] **async 抽象 Send 边界**：native 可 `Send+'static`，FFI/单线程宿主可能 `!Send` → cfg 切 / 不硬加 Send（`HX-C002`）
- [ ] **状态用 owned/Copy/arena id，不用 `&'a`**：要进 async/闭包别带生命周期（Dioxus `Scope<'a>` 地狱 + 本仓 `Tick<'a>` blocker 教训，`HX-C002`）
- [x] **Web 方案已落地**：A1-A5、最小浏览器 glue、正式 Web gate/HX-C003、历史 wasm 清理与真浏览器→真 Go 的鉴权、send/echo、断连恢复均已验收。FFI `mobile_query` 同步旁路已删除；业务查询统一走 `helix_command` → Effect/PortReply → DomainEvent，`helix_query` 只保留元信息。
- [ ] **Web 持久化可降级**：PC/Flutter 可完整 SQLite；Web 可以无本地持久化或只保留明确受限的缓存。禁止消息/投影未持久化却单独持久化一个表示已完整应用的 cursor。
- [ ] **WebSocket 边界**：core port 只保留 outbound frame；三端共享 `helix-im` 的帧处理与协议生命周期。PC/Flutter 复用 Rust 默认连接实现，Web 的真实 socket、重连执行、浏览器生命周期与鉴权接线由 Web 平台适配器兑现。
- [ ] **features 必须 additive**：禁互斥 feature（feature unification 会爆）；optional dep + `[target.'cfg(...)'.dependencies]`（`HX-C004`）
- [ ] **热路径零分配 O(1)**：`&'static str` 不 `String`、复用 buffer、单 INSERT/UPDATE WHERE PK、禁 read-modify-write（`HX-C005`）
- [ ] **FFI 边界铁律**：不许 unwind 跨 `extern "C"`——策略二选一：生产 cdylib `panic="abort"`（无需 catch_unwind）/ dev unwind 时逐导出 `catch_unwind`，禁两者混说；opaque ptr 单一 owner（`Box::into_raw`/`from_raw` 各一次）；同步 SQLite C 调用包 `spawn_blocking`（`HX-C006`）
- [ ] **Effect/事件跨 Web/FFI 边界批处理**：以 step/batch 过 JS/FFI 边界，不逐条回调（`HX-C007`）
- [ ] **Headless 语义交互边界**：`helix-im` 可统一语义 view tree/action/directive，不能退成只吐 ViewModel；`helix-core` 不认识 UI schema，具体 DOM/Component/Widget/像素仍在 shell（`HX-C018`）

---

## 3. 编译 / 测试 cadence

### 3.0 完成态自动提交授权

- 每完成一个可独立验收的 Task、Phase 或任务，验证通过后自动创建分类清晰的 Conventional Commit，无需再次询问。
- 未完成、blocked、partial、验证失败或仍混有其他任务改动时禁止提交；先继续完成或拆分工作树。
- commit body 遵循 `docs/COMMIT_CONVENTION.md`，写明影响范围、协议/依赖边界、真实验证和 Harness 更新；不得自动 push。

- 改完一个完整模块 / commit 前：`cargo check -p <crate>` 一次
- **三端 Rust 验收**（phase 完成前 / 改 port 或 driver 后必跑；真浏览器 E2E 另行执行）：
  ```bash
  cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web
  cargo test -p helix-core
  cargo build --target wasm32-unknown-unknown -p helix-driver-web
  ```
- `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` → **必须空**（零运行时证明）
- **autonomous / sub-agent 模式**：自跑验证并回报；**单任务 prompt 模式**：输出命令让用户决定（除非用户说"你跑"）

### 3.1 派 helix Rust agent / workflow 必注入的 Cargo 纪律（🔴 硬性 · 防反复编译空耗）

> **触发**：任何派去改 Rust 的 agent / workflow，prompt **必须**含下面这段。
> **Why**：cargo 编译是分钟级物理成本；不约束则 agent 陷入「改一行→cargo check→报错→改→再 check」循环——
> 实测一个 FixA agent 因此耗 ~17min，70%+ 全在编译循环空耗（非思考）。机器没法 gate「数 cargo 次数」，
> 故靠**强制 prompt 注入 + 本节**双落点硬化。

```
## Cargo 纪律（强约束·必守）
- 中途编码**禁跑 cargo**：靠类型签名 + rust-analyzer 式自审，不每改一处就 check。
- cargo **只在两处跑**：① 一个完整模块改完、准备 commit 前 `cargo check -p <crate>` 一次；
  ② phase/任务完成 `cargo test`（或双端验收命令）一次。
- **禁「失败→改→再 check」单点试错循环**：报错一次性读完、批量修完，再跑一次。
- 目标：单 agent 全程 cargo 调用 **≤ 2-3 次**（不是 5-6 次）。
```

> 配套「共享编译缓存」：helix 是非 Tauri 纯 Rust → 用 **sccache**（artifact 缓存·分支无关·不 thrash）；
> **禁 `CARGO_TARGET_DIR` 跨 worktree 共享**（不同分支不同代码 → 来回切全量重编，反向变慢，见全局 performance.md）。

---

## 4. 长运行任务约束 / Harness Engineering

> 本项目长运行任务（跨会话 / 跨 Phase / autonomous 模式）的工程化约束体系。
> 完整方法论见 [`/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md`](/Users/mac28/workspace/angular/cses-client/docs/harness-Engineer最佳实战.md)
> （**不要把全文拷进本 AGENTS.md** —— 按下方触发条件**按需** Read）。

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
| HX-C003 | Web 双目标代码须 native test + wasm final build，禁止 host runtime 泄漏 | active |
| HX-C004 | features 必须 additive，禁互斥 | active |
| HX-C005 | 热路径零分配 O(1)（`&'static str` 不 String）| active |
| HX-C006 | FFI 边界铁律（no unwind / 单 owner / spawn_blocking）| active |
| HX-C007 | Effect/事件跨 Web/FFI 边界批处理，不逐条 | active |
| HX-C008 | sync/notify pull 契约：cursor 锚 nextSeq（非 max event）+ 续拉看 len≥500（真 wire 无显式 flag）| active |
| HX-C009 | 预期对账证据完整性（EXPECT 可证伪 · DRIFT-PRED 如实记录 · 不阻断开发）| active |
| HX-C010 | 集合迭代→有序 Effect 生产必须确定性有序（有序集合/迭代前 sort，防 SipHash 随机序）| active |
| HX-C011 | 测试可证伪性铁律（压测/并发必带破坏即fail 证伪 · 禁墙钟下界断言，结构化/计数证不变量）| active |
| HX-C012 | 实质任务（含 test/CI/harness ≥50 行或新源文件）须开局写 🎯 EXPECT，禁事后补记（堵 DRIFT-PRED 永空 audit 后门）| active |
| HX-C013 | Workflow/fork agent 会动主仓 git ref（checkout/merge/branch 删）——派发前后校验 HEAD/分支/ref + 分片即时 commit | active |
| HX-C014 | 分支身份固化进 SESSION §0（当前分支+upstream+该不该删）——`branch-identity.sh` 自陈不靠记忆，杜绝「你在哪个分支」反复问 | active |
| HX-C015 | workflow/后台 agent 完成靠 task-notification 自动回灌，禁手工「等…报告结果」轮询；只外部不可追踪态才 /loop | active |
| HX-C016 | 真Go e2e:WS hello 依赖 tp/gRPC 隧道会随时间断(恢复 playbook·sudo密码sh)+全UC用单脚本双持久连接 wait_for(~3min)非LLM-workflow+快smoke(~13)vs精确per-UC(全33)取舍 | active |
| HX-C017 | 性能证据保持逐跳 correlation、确定性顺序与负载/观察窗口边界；冷热领域键分测 | active |
| HX-C018 | Headless 不是只吐 ViewModel：统一语义 view tree/action/directive，禁止具体平台 UI 进入合同 | active |

> 索引不变量：本表行数 == `ls docs/harness/HX-C*.md | wc -l`。不等 = 漂移，立即修。

---

## 4.5 行为真源（Bun Zig-as-reference 同款）

实现 / 修复 IM 行为有争议时，**现网 `~/workspace/angular/cses-client/src-tauri/src/features/im/` 对应 handler 是 source of truth**；机制级对齐标注见 [`docs/过会方案/helix-core-im-架构设计.md`](docs/过会方案/helix-core-im-架构设计.md) 各节。

### 4.5.1 前端 UC 投影 + Strict Matrix + 迁移图谱（接口/事件/用例 索引 · 按需 Read）

> 现网（cses-im-server + cses-client）的**UC / command / projection / HTTP / WS / 远端 DB / Helix storage**综合映射，以及 fail-closed 发布证据合同。已删除的 takeover-plan 与 sync-v2 阶段快照不再作入口。本表只挂索引，正文按需 Read。

| 文档 | 视角 | 一句话 |
|---|---|---|
| [`docs/specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`](docs/specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md) | **多端 UI 交互目标合同（ADR-015）** | Helix 新版本方向：UiEvent/HostObservation → Domain/Effect/Persist → InteractionFrame(ViewMutation+ActionBinding+UiDirective)；三端机械 apply/ack，具体组件仍属平台 |
| [`crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`](crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md) | **UC 消费视角真源** | Gate、command、Effect、投影字段、HTTP、WS、Go DB、Helix storage 和 UI VM 的最新代码分析；SessionStart 必读 |
| [`crates/helix-im/docs/Strict Matrix/README.md`](crates/helix-im/docs/Strict%20Matrix/README.md) | **UC 发布证据入口** | 机器合同、生成矩阵、远端存储真源与持久证据包；任一证据面缺失即 Red |
| [`crates/helix-im/docs/projection-schema.md`](crates/helix-im/docs/projection-schema.md) | **投影 payload 契约真源（C2 冻结）** | 21 个 `im:__bus__` 投影 emit 工厂字段集 + 信号/透传/控制三类 + 命名陷阱词典；配 `tests/projection_schema_test.rs` 强制对账 |
| [`crates/helix-im/docs/csesapi-migration/真机curl真源.md`](crates/helix-im/docs/csesapi-migration/真机curl真源.md) | **outbound 请求体/头 ground truth（真抓包 > 源码推断）** | 现网 cses-client 实抓 6 个 curl（posts/create·makeTopic·revoke·channel/create·member/change·close）逐字 body+头；helix-im outbound 命令体必须对齐此处（send 无回声真因=体不对·非 Pulsar；普通发消息 WS 推 `post` 非 `posted`）|
| [`crates/helix-im/docs/csesapi-migration/full-map/00-README-index.md`](crates/helix-im/docs/csesapi-migration/full-map/00-README-index.md) | **全量图谱总索引** | backend-\*（源端 HTTP/WS 清单）+ client-\*（前端用例/渲染/Tauri）+ gap-\* 三类导读 |
| [`crates/helix-im/docs/csesapi-migration/full-map/30-capability-gap-matrix.md`](crates/helix-im/docs/csesapi-migration/full-map/30-capability-gap-matrix.md) | **缺口矩阵** | covered 86 / partial 30 / missing 7 + 替代 mattermost 阻塞能力优先级 |
| `…/full-map/partials/1-…posts` · `2-…channel-sync` · `3-…user-misc` · `4-…bot-agent` | **后端 HTTP** | 124 接口四域全清单 + wire 契约字段 + 命名陷阱 |
| `…/full-map/partials/5--backend-ws-events.md` | **后端 WS 推送** | 19 事件常量真源（`websocket_message.go`）+ `event_seq` 注入铁律 |
| `…/full-map/partials/6--client-usecases.md` · `7--client-ui-rendering.md` · `8--client-tauri-im.md` | **前端用例/渲染/Tauri** | message-v3 service 42 用例 + UI readBits/字段契约 + src-tauri Rust IM 核心 14 用例集 |
| `…/full-map/partials/9..12-gap-*.md` | **三态缺口** | posts/channel/user-misc/bot-agent 各域 covered/partial/missing 对照 |
| [`crates/helix-im/docs/csesapi-migration/01-接口可达性清单.md`](crates/helix-im/docs/csesapi-migration/01-接口可达性清单.md) · [`02-迁移蓝图.md`](crates/helix-im/docs/csesapi-migration/02-迁移蓝图.md) | **可达性 + 蓝图** | 接口可达性逐条 + 迁移蓝图 |

## 4.6 专家 Agent / Skill 注册表（按需调用 · 渐进式披露）

> 仓内 agent 路由与约束订阅见 [`.agents/README.md`](.agents/README.md)。
> 阶梯：**L0 就近文件 → L1 Skill 拉知识 → L2 单 Agent 派活/审查 → L3 多 Agent 编制**——能低不高，升级要有触发条件。

| 专家 Agent ↔ Knowledge Skill | 领域一句话 | 何时调 |
|---|---|---|
| `rust-dev` ↔ `/rust-knowledge` | Rust 语言与工程全栈（所有权/async/性能/unsafe） | 借用·Send 报错、热路径审查、trait 设计争议 |
| `rust-tauri-skeleton-dev` ↔ `/rust-tauri-skeleton-knowledge` | Tauri 2 桌面骨架端到端，**参考仓 = 现网 cses-client** | 现网对齐核验（唯一权威）、IPC 契约、M2 Tauri 接缝 |
| `rust-runtime-agnostic-core-dev` ↔ `/rust-runtime-agnostic-core-knowledge` | 一份 core 编 native+FFI+Web（cfg/Send/wasm） | 多端边界守门、Flutter FFI / Web Rust 接入层 |
| `dioxus-dev` ↔ `/dioxus-knowledge` | **仅取 core 内核经验**（step 循环/Effect 批处理/调度），**禁渲染层/RSX 话题** | 内核循环设计对照、批次边界、调度公平性 |

本仓操作手册 Skill（先手册后知识）：`/helix-new-module` · `/helix-replay-test` · `/helix-stress-verify`。

## 4.7 预期对账（非阻塞证据记录）

> 目的：让实质任务在开局声明可证伪预期，收尾记录真实结果，为后续排障和复盘保留证据。
> 本机制不按 commit 数触发 review，不阻断 feature、commit 或 push。

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

**回放闸门**（候选教训 → 新建 HX-C 卡前必过）：拿候选约束回放 `docs/harness/log.md` 历史 trap
+ §0 既往 `⚡ DRIFT-PRED`——能挡住当初那次、且不误伤既往成功，才建卡（防约束过拟合 / 膨胀）。

## 5. 代码风格

- 类型驱动：make-invalid-states-unrepresentable；tagged union(enum) 优于一堆 Option/bool；newtype 不透明
- 错误：`thiserror` 边界类型 / 不滥 `anyhow`；外部输入（WS 帧 / sync 响应 / FFI 入参）**零 `unwrap`/`expect`**，走 `Result`
- 命名/模块：高内聚低耦合，many small files；mod.rs 只 re-export
- 注释只写「为什么」(invariant / SAFETY / 三栖偏差)，不写「做什么」

## 6. 提交规范（commit / PR / tag）

**提交代码前必读 → [`docs/COMMIT_CONVENTION.md`](docs/COMMIT_CONVENTION.md)**：Conventional Commits（中文 description/body）+ 触发条件命中时的 5 段结构化 body（影响范围 / 改动影响面 / 功能改进 / Harness 更新 / 验证）+ PR / Tag 策略。本仓有 `docs/harness/` → Harness 一节必填。配套 `scripts/gate.sh` + pre-push hook。

## Agent skills

> Configured by `setup-matt-pocock-skills` for the mattpocock engineering skill set
> (`to-prd`, `to-issues`, `triage`, `qa`, `domain-modeling`, …).

### Issue tracker

Self-hosted **GitLab** at `git.yundiz.com`, project `cses/go/helix`, via the `glab` CLI over HTTP. External merge requests are **not** a triage surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`); GitLab auto-creates them on first use. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context. ADRs live in `docs/harness/decisions.md` (not `docs/adr/`); domain language in `docs/GOAL.md` + `docs/harness/`. See `docs/agents/domain.md`.
