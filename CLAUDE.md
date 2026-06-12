# Helix — 项目指令（每次会话开局必读）

> Helix = 一个 **runtime-agnostic（运行时无关）确定性业务内核基座**。一次实现，编译到三端：
> **native（Tauri / FFI 桌面）· FFI（移动 Swift/Kotlin）· WASM（浏览器）**。
> IM 只是跑在它上面的**第一个业务模块**；core layer 本身不认识任何业务。
> 概念祖先：Bun（runtime 跑 JS）/ Dioxus core（headless 核跑 6 端）。

> 🧭 **北极星（设计理念不变量，不可偏离）→ [`docs/GOAL.md`](docs/GOAL.md)**　每次会话开局必读。
> 任何改动若与 GOAL.md §1 设计理念 / §2 硬约束冲突 → **先停下问，不偏离**。

> 📋 **架构评审待办（实现阶段提醒）→ [`docs/过会方案/helix-架构评审-待办清单.md`](docs/过会方案/helix-架构评审-待办清单.md)**
> 过会评审揪出的必办项：B 正确性遗漏（gate_inflight / event_seq 三级 fallback / type=6 readBits / phantom）· C 约束澄清（Storage scan / Module 同步约束）· D roadmap 调整（MVP1 加 IndexedDB 兜底 / MVP2 拆 2a-2b / +3 周）。**进实现阶段前先扫一眼，防遗忘。**

---

## 0. 三层架构（铁律，不可变）

```
业务模块层   [helix-im] [将来 presence / notify ...]   只依赖 helix-core 的 ports，不碰 tokio/sqlx/web-sys
    ▲ 实现 Module trait + register()
core layer   ★ helix-core ★   ports(Transport/Storage/Clock/EventSink/IdSource) + Module 宿主 + Effect/事件 + sans-IO 执行壳
    ▲ 实现 ports（依赖方向单向：adapter → core，永不反向）
平台驱动层   helix-driver-native(tokio/sqlx/tungstenite/reqwest) · helix-driver-wasm(wasm-bindgen/web-sys/OPFS) · helix-driver-ffi(C ABI/uniffi/rusqlite)
```

**依赖铁律**：`helix-core` 永不直接 import `tokio` / `sqlx` / `reqwest` / `web-sys` / `tauri` —— **只定义 trait**。
驱动在边缘实现。违反 = 三栖立刻碎。验证见 harness `HX-C001`。

---

## 1. 核心开发理念

1. **Sans-IO / 六边形**：core 不做任何 I/O、不持有任何运行时。纯状态机：喂入(入站帧 / now_ms / 端口响应) → 吐出(Effect: Persist/Send/SetTimer/Emit)。真正的网络/存储/线程由驱动做。
2. **确定性**：时间 / 随机 / ID 一律由 host 注入（不在 core 调 `Instant::now` / `rand` / 生成 id）。同输入同输出，可回放、可测试。
3. **并发不进 core**：WASM 单线程，tokio multi-thread 在 wasm32 不可用 → 并发模型属于驱动层。core 必须能在单线程跑。
4. **YAGNI 但边界要对**：通用性靠「≥1 真实模块跑通 + 第二模块插得进」挣，不预建插件框架。但 **YAGNI 边界必须在「三端目标集合」下画** —— 单平台假设会砍错三栖必需的抽象（见 harness `HX-C001`/`HX-C002` 教训）。

---

## 2. 三栖边界条件 checklist（写 core / 改 port / 加 driver 前过一遍）

> 完整规则 + 代码 + 坑词典见 [`.claude/rules/runtime-agnostic-core.md`](.claude/rules/runtime-agnostic-core.md)（按需 Read）。
> 下面是开局必记的速查：

- [ ] **helix-core 零运行时依赖**：grep 不出 tokio/sqlx/reqwest/web-sys/tauri（`HX-C001`）
- [ ] **async 抽象 Send 边界**：native 要 `Send+'static`，wasm 单线程 `!Send` → cfg 切 / 不硬加 Send（`HX-C002`）
- [ ] **状态用 owned/Copy/arena id，不用 `&'a`**：要进 async/闭包别带生命周期（Dioxus `Scope<'a>` 地狱 + 本仓 `Tick<'a>` blocker 教训，`HX-C002`）
- [ ] **wasm-only 代码必须进 CI 三栖 type-check**：`#[cfg(wasm32)]` 在 native CI 不被检查 → CI 必跑 `--target wasm32-unknown-unknown` + 误链 `compile_error!` 守护（`HX-C003`）
- [ ] **features 必须 additive**：禁互斥 feature（feature unification 会爆）；optional dep + `[target.'cfg(...)'.dependencies]`（`HX-C004`）
- [ ] **热路径零分配 O(1)**：`&'static str` 不 `String`、复用 buffer、单 INSERT/UPDATE WHERE PK、禁 read-modify-write（`HX-C005`）
- [ ] **FFI 边界铁律**：不许 unwind 跨 `extern "C"`——策略二选一：生产 cdylib `panic="abort"`（无需 catch_unwind）/ dev unwind 时逐导出 `catch_unwind`，禁两者混说；opaque ptr 单一 owner（`Box::into_raw`/`from_raw` 各一次）；同步 SQLite C 调用包 `spawn_blocking`（`HX-C006`）
- [ ] **Effect 跨边界批处理**：FFI/wasm 边界**批量**过 effect，不逐条（Dioxus sledgehammer 教训，`HX-C007`）
- [ ] **getrandom js feature**：任何 wasm target 加 `getrandom = { features=["js"] }`
- [ ] **wasm size**：release profile `opt-level="z"` + `lto` + `panic="abort"` + `strip`

---

## 3. 编译 / 测试 cadence

- 改完一个完整模块 / commit 前：`cargo check -p <crate>` 一次
- **三栖验收**（phase 完成前 / 改 port 或 driver 后必跑）：
  ```bash
  cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
  cargo check --target wasm32-unknown-unknown -p helix-core -p helix-driver-wasm
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
| HX-C002 | async port Send 边界 + 状态禁 `&'a`（三栖 + 生命周期）| active |
| HX-C003 | wasm-only 代码必须进 CI 三栖 type-check + compile_error 守护 | active |
| HX-C004 | features 必须 additive，禁互斥 | active |
| HX-C005 | 热路径零分配 O(1)（`&'static str` 不 String）| active |
| HX-C006 | FFI 边界铁律（no unwind / 单 owner / spawn_blocking）| active |
| HX-C007 | Effect 跨 wasm·FFI 边界批处理，不逐条 | active |

> 索引不变量：本表行数 == `ls docs/harness/HX-C*.md | wc -l`。不等 = 漂移，立即修。

---

## 4.5 行为真源（Bun Zig-as-reference 同款）

实现 / 修复 IM 行为有争议时，**现网 `~/workspace/angular/cses-client/src-tauri/src/features/im/` 对应 handler 是 source of truth**；机制级对齐标注见 [`docs/过会方案/helix-core-im-架构设计.md`](docs/过会方案/helix-core-im-架构设计.md) 各节。

## 5. 代码风格

- 类型驱动：make-invalid-states-unrepresentable；tagged union(enum) 优于一堆 Option/bool；newtype 不透明
- 错误：`thiserror` 边界类型 / 不滥 `anyhow`；外部输入（WS 帧 / sync 响应 / FFI 入参）**零 `unwrap`/`expect`**，走 `Result`
- 命名/模块：高内聚低耦合，many small files；mod.rs 只 re-export
- 注释只写「为什么」(invariant / SAFETY / 三栖偏差)，不写「做什么」
