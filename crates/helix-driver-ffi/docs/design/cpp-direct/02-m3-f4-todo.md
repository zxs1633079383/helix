# M3-F4 已落地实录（pump 主循环接入）

> ✅ 本清单原为「边界层留诚实 TODO」；M3-F4 pump 阶段已在本分支
> **commit d8c3093..630a713 全部真实实装并通过 e2e 测试**
> （`command_emits_domain_event_through_batch_callback`）。
> 下面逐项记录落地形态。**唯一仍为真 TODO 的是 T5（`helix_query`）**，
> 见末节说明。

---

## ✅ 已落地（pump 接通后功能可用）

### T1 — pump 线程 + tokio runtime（✅ 已落地）
- `helix_create_with_network`（`api.rs`）起专用 `std::thread`，线程内建
  **current_thread** tokio runtime（`tokio::runtime::Builder::new_current_thread().enable_all()`），
  `block_on` host `run_engine_loop`（`pump.rs::run`）。core 在此专属单线程跑（GOAL §1.3）。
- `HelixHandle` 持 `shutdown_tx` / `pump_thread`（`JoinHandle`）/ `query`（QueryDeps）。
- tick channel：`helix_command → tick_tx`；入站帧经 `SharedWsClient::with_inbound_tick` 直发 `Tick::Inbound`。

### T2 — `helix_destroy` join pump 线程（✅ 已落地）
- 关闭顺序（`api.rs::helix_destroy`）：① `Box::from_raw` 取回独占所有权 → ② take + send `shutdown_tx`
  （与 drop tick_tx 双触发冗余，host biased select 任一即 graceful drain）→ ③ **join `pump_thread`**
  （阻塞到 pump 退出，保证最后一批 `on_domain_events_batch` 在 vtable.ctx 释放前返回）。
- pump 线程 panic 由 `std::thread` 在线程边界隔离，`join()` 返回 `Err` 而非 unwind 跨 FFI。

### T3 — pump 主循环（✅ 已落地，复用 host 泛型核）
- **未在 FFI crate 内重写循环**：`pump.rs::run` 装配 FFI 特化 ports 后调
  `helix_driver_host::engine::run_engine_loop`（host 泛型泵核）。tick 消费 / `step()` / Effect 分发
  全由 host 核统一驱动（与 native driver 共用同一核）。

### T4 — Effect 分发（✅ 已落地，下沉 host）
- `Effect::Emit` → `FfiEventSink` 缓冲本 step → host 每 step 调 `BatchSink::flush`
  → `batch.rs` 整批编码成单 buffer → **一次** `CppHostVtable::on_domain_events_batch`（HX-C007，禁逐条）。
- `Effect::Http` / `Effect::Persist` / `Effect::ScheduleTimer` / `Effect::CancelTimer`
  全走 host dispatch（`run_engine_loop` 内统一解释，含 `spawn_blocking` 包同步 SQLite 写，HX-C006 §5）。

### T6 — HTTP status 映射（✅ 真实契约，host 内落地）
- 完成响应（任意 HTTP status）→ `PortOutcome::Ok(ReplyBytes(json{status,...}))`；
  只有传输级错误（连接超时 / 连接断）→ `Err`（`classify_port_error`）。映射在 host engine 内，FFI 不重复实现。

---

## ⏳ 仍为真 TODO

### T5 — `helix_query` 实现（⏳ honest stub，`FFI_UNIMPLEMENTED`）
- **运行时基础设施已就位**：`HelixHandle.query` = pump runtime `Handle` + 共享 `HostStorage`
  （可经 `spawn_blocking` 同步读，禁 `block_on` 同线程死锁）。
- **缺的是 `query_name → ScanSpec/GetSpec` 映射**——那是 IM 领域语义，直接写进 FFI driver
  会把 schema 泄漏进驱动层（破 Storage trait 不泄漏业务 schema 的约束）。
- 故保留 honest stub，待 IM 暴露通用 query 路由（或经 command + Emit 回读）后填充，不在此越界。
- 成功路径契约：`spawn_blocking` 读 → 序列化 → `into_boxed_slice` → `Box::into_raw` 写 out_ptr/out_len，
  C++ 侧配对 `helix_query_free`。

---

## 落地后仍需遵守的契约

- `PortError` 真源是 `helix_core::error::PortError`（`Storage(String)` / `Transport(String)` / `Http(String)`），
  **不是** `tick.rs` 的 tick-level `PortError`（Timeout/Http(u16)/...）。
- `ExecutionShell::step` 签名：`(&mut self, tick: Tick, now_ms: u64) -> Result<&[Effect], CoreError>`
  （返回借用的 scratch slice）。
- 单 step 内多个 `Effect::Emit` 必须聚合后整批回传（HX-C007）。
