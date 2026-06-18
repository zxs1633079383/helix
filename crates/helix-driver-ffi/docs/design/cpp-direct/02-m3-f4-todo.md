# M3-F4 TODO 清单（pump 主循环接入）

> 本轮（边界层）100% 留诚实 TODO，不写一行 pump 代码。
> 以下清单是 M3-F4 pump 阶段的完整接入指南。

---

## 必做（pump 接入前无功能）

### T1 — pump 线程 + tokio runtime
- 在 `helix_create_with_network` 内 `tokio::runtime::Builder::new_current_thread().build()` 建运行时，
  `std::thread::spawn` 启动 pump 线程。
- pump 线程持 runtime Handle + `_tick_rx`（从 HelixHandle 取出，改 `tick_rx: Option<mpsc::Receiver<Tick>>`）。
- HelixHandle 增 `pump_thread: Option<JoinHandle<()>>`。

### T2 — `helix_destroy` join pump 线程
```rust
// 取出并 drop tick_tx → pump 的 rx.recv().await 返回 None → pump 主循环退出
drop(boxed.tick_tx.lock().take());
// join pump 线程，确保 on_domain_events_batch 回调全部完成后再返回
if let Some(h) = boxed.pump_thread.take() { h.join().ok(); }
// 此后 vtable.ctx 才可安全释放
drop(boxed);
```

### T3 — pump 主循环（`helix-driver-ffi/src/pump.rs`，新文件）
```rust
// 伪代码骨架
async fn pump_loop(mut rx: mpsc::Receiver<Tick>, vtable: CppHostVtable, shell: ExecutionShell) {
    while let Some(tick) = rx.recv().await {
        let now_ms = vtable.now_ms.map(|f| f(vtable.ctx)).unwrap_or(0);
        match shell.step(tick, now_ms) {
            Ok(effects) => dispatch_effects(effects, &vtable),
            Err(e) => tracing::error!("step error: {e:?}"),
        }
    }
}
```

### T4 — Effect 分发（`dispatch_effects`）
- `Effect::Emit { event }` → 收集为 `Vec<DomainEventBytes>`，序列化整批为单个 `[u8]` buffer，
  **一次调用** `vtable.on_domain_events_batch`（HX-C007，禁逐条）。
- `Effect::Http { req, corr }` → `SharedHttpClient::request(req)` → 结果包成 `Tick::PortReply{ corr, outcome: Ok(ReplyBytes(json{status,...})) }` 送回 rx（注意：任意 HTTP status 都是 Ok，只有传输错才是 Err）。
- `Effect::Persist { ops, corr }` → `spawn_blocking` 跑 `HostStorage` 同步写（HX-C006 §5）→ 结果送 `Tick::PortReply`。
- `Effect::ScheduleTimer { id, delay_ms }` → tokio::time::sleep → 发 `Tick::Timer(id)` 到 tx 端。
- `Effect::CancelTimer { id }` → abort 对应 sleep task。

### T5 — `helix_query` 实现
```rust
// 经 pump runtime Handle spawn_blocking（禁 block_on）
// 序列化结果 → into_boxed_slice → Box::into_raw 写 out_ptr/out_len
// 配对 helix_query_free
```
**禁止**在 helix_query 函数体内 `block_on`（若 tokio rt 在同一线程，block_on 死锁）。

### T6 — HTTP status 映射（真实契约）
```rust
// run_http 完成响应（任意 status）→ PortOutcome::Ok
match resp.status().as_u16() {
    _ => PortOutcome::Ok(ReplyBytes(serde_json::to_vec(&json!({
        "status": status_code,
        "body": body_bytes,
    })).unwrap_or_default().into())),
}
// 只有传输层错（connect timeout / connection reset）→ PortOutcome::Err
// 用 classify_port_error(reqwest::Error) → PortError 翻译
```

---

## 注意事项

- `PortError` 真源是 `helix_core::error::PortError`（`Storage(String)` / `Transport(String)` / `Http(String)` 等 String 承载），**不是** `tick.rs` 中的 `PortError`（Timeout/Http(u16)/Storage(u32)/Network/Other(u32)，那是 tick-level enum）。
- `ExecutionShell::step` 真签名：`(&mut self, tick: Tick, now_ms: u64) -> Result<&[Effect], CoreError>`（返回借用的 scratch slice，不是 `Vec`）。
- 单 step 内多个 `Effect::Emit` 必须聚合后整批回传（HX-C007），不在 dispatch_effects 里对每个 Emit 单独回调。
