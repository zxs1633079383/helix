# helix-driver-ffi 局部指令（就近优先，全局见根 CLAUDE.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 是什么（C++ 直连薄壳 pump）

Flutter 手机端 FFI driver（C ABI）。**复用 `helix-driver-host` 的泛型 pump 核**（`run_engine_loop`），只装配 FFI 特化 ports + 线程/lifecycle：

- **pump 线程**（`pump.rs`）：`helix_create_with_network` 起专用 `std::thread`，线程内建 **current_thread** tokio runtime（`.enable_all()`）`block_on` host `run_engine_loop`。core 单线程跑（GOAL §1.3）。装配 storage（`HostStorage`）/ http（`FfiHttpRequester`）/ ws（`SharedWsClient::with_inbound_tick` 入站直发 `Tick::Inbound`）/ `FfiEventSink` / `FfiClock` + 注册 `ImModule`。
- **egress 批处理**（`event_sink.rs` + `batch.rs`）：`FfiEventSink` 缓冲本 step 的 Emit，host pump 每 step 调 `BatchSink::flush` → 整批编码成单 buffer（`batch.rs` len-prefixed wire layout）→ **一次** `CppHostVtable::on_domain_events_batch`（HX-C007，绝不逐条过边界）。
- **时间注入**（`clock.rs`）：`FfiClock` 调 `vtable.now_ms(ctx)`（确定性，host 注入）；None 时 fallback SystemTime + warn。
- **lifecycle**（`api.rs`）：`helix_command → tick_tx`（pump 消费 → step → Effect）；`helix_destroy` = drop tick_tx + send shutdown + **join pump 线程**（保证最后一批回调在 ctx 释放前返回）。
- **`helix_query`**：honest stub（`FFI_UNIMPLEMENTED`）。infra（runtime handle + storage）已在 `HelixHandle.query` 就位；缺 `query_name→Spec` 映射（IM 领域语义），填充会泄漏 schema 进 driver → 暂不实现（见函数 doc）。

模块：`api`（C ABI 导出 + HelixHandle）· `pump`（线程/runtime/装配）· `event_sink`（批 egress）· `batch`（wire 编解码）· `clock`（host 时间）· `vtable`（C++ 回调表）· `http`/`storage`/`transport`（port 适配；pump 直接用 Shared*/HostStorage，这三是备用包装）。

## FFI 铁律（HX-C006）

1. 解引用裸指针的导出函数必须 `pub unsafe extern "C"` + `# Safety` doc（clippy 强制）
2. 每个 unsafe 块必配 `// SAFETY:` 注释（undocumented_unsafe_blocks = deny）
3. opaque ptr 单 owner：`Box::into_raw`/`from_raw` 各一次 + null-check
4. panic 策略二选一：生产 cdylib `panic="abort"`（无需 catch_unwind）/ dev unwind 逐导出 catch_unwind——禁混说
5. 同步 SQLite C 调用必包 `spawn_blocking`（MAJ-4，落地前 todo 不许偷偷删）
6. C 侧 buffer 立即拷贝不持有（ACL-3）；读查询走独立同步 `helix_query`，不混 SendPort 事件流

## 改动验收

```bash
cargo check -p helix-driver-ffi && cargo clippy -p helix-driver-ffi --all-targets
cargo test -p helix-driver-ffi   # 含 command→Emit→批回调 e2e + pump lifecycle drain
# 改 pump 核（host engine/BatchSink）后 native 零回归安全网必跑：
cargo test -p helix-driver-native -p helix-driver-host
```
