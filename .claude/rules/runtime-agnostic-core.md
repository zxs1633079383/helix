# Rule — Runtime-Agnostic Core Layer 开发规约（Helix）

> 适用：helix-core / helix-im / helix-driver-{native,wasm,ffi}。
> 这是 [`../../CLAUDE.md`](../../CLAUDE.md) §2 三栖边界 checklist 的完整展开（代码 + 坑词典）。
> 核心一句话：**core 定义 ports（trait），驱动在边缘实现；依赖方向单向 adapter → core，永不反向。**

---

## 1. 三栖编译目标矩阵

| target | std/fs/线程 | 时间 | 网络 | 存储 | 随机 |
|---|---|---|---|---|---|
| `wasm32-unknown-unknown`（浏览器）| 无 OS / 无 fs / **单线程** / std 子集 | `performance.now`（host） | web-sys WebSocket / fetch | OPFS / IndexedDB（OPFS 仅 Worker 内） | `getrandom` js feature |
| `aarch64-apple-darwin` / `x86_64-pc-windows-msvc`（native）| 完整 | `tokio::time` / `Instant` | tokio-tungstenite / reqwest | sqlx + SQLite | OS rng |
| FFI（移动，经 native 编译 + C ABI）| 完整（host 进程）| host 提供 | host 回调 / rusqlite | rusqlite / host | host |

**铁律**：`#[cfg(target_arch="wasm32")]` 的分支**只在编译该 target 时才被类型检查**。所以 CI 必须对**每个 target** 各跑一次 `cargo check --target ...`，否则 wasm 分支的类型错误 native CI 永远发现不了（见 `HX-C003`）。

---

## 2. 依赖倒置（ports & adapters / hexagonal）

```rust
// helix-core：只定义 trait，永不 import tokio/sqlx/reqwest/web-sys/tauri
pub trait Transport { /* connect/send/recv/close */ }
pub trait Storage   { /* batch_upsert/update/delete/get/cursor_upsert（通用 spec，不暴露 IM schema）*/ }
pub trait Clock     { fn now_ms(&self) -> u64; /* set_timer/clear_timer */ }
pub trait EventSink { fn emit(&self, ev: DomainEvent); }
pub trait IdSource  { fn next_id(&self) -> Id; }   // 确定性：id 由 host 注入，core 不自生成
```

- core 通过这些 trait 与外界对话；驱动 `impl Transport for TokioTungstenite { ... }`。
- **验证**（`HX-C001`）：`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 必须空；`grep -rnE 'use (tokio|sqlx|reqwest|web_sys|tauri)' crates/helix-core/src` 必须 0。

---

## 3. Executor-agnostic async（最易踩）

- **不在 core 硬依赖 tokio**。共同基底是 `Future` trait。spawn 注入或 cfg 选：
  ```rust
  #[cfg(not(target_arch="wasm32"))]
  pub fn spawn<F: Future<Output=()> + Send + 'static>(f: F) { tokio::spawn(f); }
  #[cfg(target_arch="wasm32")]
  pub fn spawn<F: Future<Output=()> + 'static>(f: F) { wasm_bindgen_futures::spawn_local(f); } // 无 Send
  ```
- **Send vs !Send**（`HX-C002`，Dioxus 同款坑）：native multi-thread 要 `Send + 'static`；wasm 单线程 `!Send`。
  - 默认 **Unsync**（`!Send`），跨线程才 **opt-in Sync**（Dioxus = UnsyncStorage 默认 / SyncStorage opt-in）。
  - port 的 `async fn` **不要无脑加 Send bound**：加了 wasm 的 `!Send` future 冲突；不加则 native 把 core 跑在**专属单线程**（`spawn_local` + `LocalSet`），驱动负责线程。
  - 决策已拍：见 `docs/harness/decisions.md`。
- **never block 浏览器主线程**：wasm 无 `block_on`，只有 `spawn_local`。native `block_on` 可以。
- **async in trait**：AFIT/RPITIT 稳定但**非 dyn-safe**；要 `Box<dyn Trait>` 用 `#[async_trait]`。core 的 port 若需对象安全 + 跨端 → 权衡 async-trait 开销 vs 泛型静态分发。
- 时间/通道中立化：`futures-channel`（中立）优于 `tokio::sync`；时间用注入的 `Clock`，不在 core 调 `tokio::time` / `Instant::now`。

---

## 4. 状态所有权（生命周期地狱预防）

- **要进 async/闭包/跨 step 的状态，用 owned / Copy handle / arena id，绝不用 `&'a`。**
- 本仓实测 blocker：`Tick<'a>` 的借用满足不了 `Sender<Tick<'static>>`，timer 路径断 → 改 `bytes::Bytes`（owned）消除 `'a`。
- Dioxus 同源教训：0.5 弃 `Scope<'a>`（hook 返 `&'a T`）→ Copy `Signal` + `GenerationalBox`（arena slot + generation 校验防 use-after-free）。Helix 状态进闭包/async 同理：owned 或 arena id，不传引用。

---

## 5. Features 必须 additive（`HX-C004`）

```toml
[features]
default = ["eventbus"]
eventbus = []
storage  = ["dep:sqlx"]            # optional dep，不自动建 implicit feature
http     = ["dep:reqwest"]
ws       = ["dep:tokio-tungstenite"]

[target.'cfg(target_arch="wasm32")'.dependencies]
wasm-bindgen = "0.2"
getrandom = { version = "0.2", features = ["js"] }   # wasm 随机必加
[target.'cfg(not(target_arch="wasm32"))'.dependencies]
tokio = { version = "1", features = ["rt-multi-thread"] }
```

- **禁互斥 feature**：feature unification 只会「加」，互斥设计在 diamond dependency 下必爆。"disable X" 是反模式。
- `default-features = false` 给最小构建；`dep:` 引用 crate 不建 implicit feature；`pkg?/feat` 弱激活。

---

## 6. Storage / Transport / HTTP：一个 trait，cfg 选实现

- **sqlx 不能编 `wasm32-unknown-unknown`**（拉 tokio/mio/getrandom）。native 用 sqlx；wasm 用 `sqlite-wasm-rs` + OPFS（**OPFS 仅 Worker 内可初始化**，主线程会崩 → 用 `DedicatedWorkerGlobalScope` probe，本仓 MAJ-3）。
  ```rust
  pub trait Storage { /* ... */ }
  #[cfg(not(target_arch="wasm32"))] pub struct SqlxSqlite { pool: sqlx::SqlitePool }
  #[cfg(target_arch="wasm32")]      pub struct OpfsSqlite { /* sqlite-wasm-rs + opfs-sahpool */ }
  ```
- HTTP：native `reqwest` / wasm `gloo-net` 或 reqwest 的 wasm fetch 后端（无 streaming / 无自定义 tls）。
- WS：native `tokio-tungstenite` / wasm `web-sys WebSocket` 或 `ws_stream_wasm`。
- **Storage trait 不许泄漏业务 schema**（如 `ON CONFLICT(temporary_id)`）—— 用通用 `UpsertSpec`/`GetSpec`，IM 专属语义留 helix-im。

---

## 7. FFI 边界铁律（`HX-C006`）

- `#[no_mangle] pub extern "C" fn`；`repr(C)` 结构有定义 layout；`cbindgen` 生成头文件；或 `uniffi`（Swift/Kotlin）。
- Rust 对象跨边界 = **opaque pointer**：`Box::into_raw` 交出、`Box::from_raw` 收回，**各一次**（单一 owner，防 double-free，null-check）。
- 错误 = error code / out-param。**绝不让 panic unwind 跨 `extern "C"`** → 策略二选一不混用：生产 cdylib `panic="abort"`（panic 即 abort，catch_unwind 无效也不需要）；dev/test panic=unwind 时每个导出包 `catch_unwind` 转错误码。`panic="abort"` 设在最终 cdylib profile，不设 library crate 级。
- **同步阻塞调用（rusqlite C 调用）必须包 `spawn_blocking`**，否则阻塞 tokio worker（本仓 MAJ-4，FFI driver 落地前必补，当前 `todo!()`）。

---

## 8. Effect 跨边界批处理（`HX-C007`）

- core 吐出的 Effect 跨 wasm/FFI 边界要**批量**传，不逐个过边界。Dioxus 用 sledgehammer 把多条 mutation 编码成单个 byte buffer 一次过 wasm 边界（而非逐条 web-sys 调用）→ 性能数量级差异。
- Helix：一个 step 产出的 `Vec<Effect>` 应整批交给 driver 解释，不要每个 effect 单独 invoke / postMessage。

---

## 9. 确定性 / 热路径 / 边界输入

- **确定性**：core 不调 ambient `Date::now` / `rand`；time/random/id 由 host 经 port 注入。可回放测试。
- **热路径 O(1)**（`HX-C005`）：每事件命中的写 = 单 INSERT / UPDATE WHERE PK；禁 read-modify-write（读 JSON 数组改回写）；固定字符串用 `&'static str` 不 `"x".to_string()`（本仓 MAJ-5）；复用 scratch buffer。
- **边界输入零信任**：WS 帧 / sync 响应 / FFI 入参一律 `Result`，零 `unwrap`/`expect`；数值先验 NaN/范围再 cast；字符串注意编码。

---

## 10. wasm size + 测试

- release profile：`opt-level="z"` + `lto=true` + `codegen-units=1` + `panic="abort"` + `strip=true`；`wasm-opt` 后处理；`twiggy` 分析。
- 测试：`wasm-bindgen-test` + `wasm-pack test`（headless / node）；确定性逻辑用 `proptest` 回放；core 纯逻辑可 native 单测（注入 mock port）。

---

## 坑词典速查

| 症状 | 原因 | 解 |
|---|---|---|
| sqlx 编不过 wasm | sqlx 拉 tokio/mio/getrandom | wasm 走 sqlite-wasm-rs+OPFS（trait 后面），sqlx 锁 `cfg(not(wasm32))` |
| getrandom wasm build error | 缺 js feature | `getrandom = { features=["js"] }` |
| future is not Send（wasm）| 硬加了 Send bound | cfg 去 Send，wasm 单线程 |
| panic 跨 FFI 崩 | unwind 越过 extern C | 生产 cdylib `panic="abort"`；dev unwind 时逐导出 `catch_unwind`（二选一） |
| feature 改成互斥后炸 | feature unification | 改 additive |
| 巨大 .wasm | 没优化 | opt-level=z + lto + panic=abort + wasm-opt + twiggy |
| FFI double-free | 两边都 own | 只一侧 own，`from_raw` 一次，null-check |
| wasm 分支类型错 native 没发现 | cfg 分支只在该 target type-check | CI 跑 `--target wasm32-unknown-unknown` + compile_error 守护 |
| OPFS 主线程崩 | OPFS 仅 Worker | `DedicatedWorkerGlobalScope` probe |
