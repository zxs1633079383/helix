# M3-F4 已落地实录（pump 主循环接入）

> ✅ 本清单原为「边界层留诚实 TODO」；M3-F4 pump 阶段已在本分支
> **commit d8c3093..630a713 全部真实实装并通过 e2e 测试**
> （`command_emits_domain_event_through_batch_callback`）。
> 下面逐项记录落地形态。T5 最终选择删除同步业务查询旁路：`helix_query`
> 只保留元信息，业务查询走共享异步主路径。

---

## ✅ 已落地（pump 接通后功能可用）

### T1 — pump 线程 + tokio runtime（✅ 已落地）
- `helix_create_with_endpoints`（推荐）或兼容 `helix_create_with_network`（`api.rs`）起专用
  `std::thread`；前者分别接收 Go/Java/WS，后者不注入也不推测 Java。线程内建
  **current_thread** tokio runtime（`tokio::runtime::Builder::new_current_thread().enable_all()`），
  `block_on` host `run_engine_loop`（`pump.rs::run`）。core 在此专属单线程跑（GOAL §1.3）。
- `HelixHandle` 持 `shutdown_tx` / `pump_thread`（`JoinHandle`）；不持业务查询 runtime/storage 句柄。
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

## ✅ T5 最终决策

`helix_query` 只提供 capabilities/version，并继续用 `helix_query_free` 释放返回 buffer。
业务查询经 `helix_command` → Effect/PortReply → DomainEvent 批次完成；同步直查 Storage 的
`QueryDeps`/`mobile_query` 路径已删除，避免形成第二条查询真源。

---

## 落地后仍需遵守的契约

- `PortError` 真源是 `helix_core::error::PortError`（`Storage(String)` / `Transport(String)` / `Http(String)`），
  **不是** `tick.rs` 的 tick-level `PortError`（Timeout/Http(u16)/...）。
- `ExecutionShell::step` 签名：`(&mut self, tick: Tick, now_ms: u64) -> Result<&[Effect], CoreError>`
  （返回借用的 scratch slice）。
- 单 step 内多个 `Effect::Emit` 必须聚合后整批回传（HX-C007）。
