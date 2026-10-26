# helix-driver-ffi 局部指令（就近优先，全局见根 AGENTS.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 是什么（C++ 直连薄壳 pump）

Flutter 手机端 FFI driver（C ABI）。**复用 `helix-driver-host` 的泛型 pump 核**（`run_engine_loop`），只装配 FFI 特化 ports + 线程/lifecycle：

- **pump 线程**（`pump.rs`）：推荐 `helix_create_with_endpoints(go, java, ws, vtable)` 显式注入双 HTTP origin；兼容 `helix_create_with_network(go, ws, vtable)` 只提供 Go/WS、Java 为空且不推测。两者都起专用 `std::thread`，线程内建 **current_thread** tokio runtime（`.enable_all()`）`block_on` host `run_engine_loop`。core 单线程跑（GOAL §1.3）。装配 storage（`HostStorage`）/ http（`FfiHttpRequester`）/ ws（`SharedWsClient::with_inbound_tick` 入站直发 `Tick::Inbound`）/ `FfiEventSink` / `FfiClock` + 注册 `ImModule`。
- **egress 批处理**（`event_sink.rs` + `batch.rs`）：`FfiEventSink` 缓冲本 step 的 Emit，host pump 每 step 调 `BatchSink::flush` → 整批编码成单 buffer（`batch.rs` len-prefixed wire layout）→ **一次** `CppHostVtable::on_domain_events_batch`（HX-C007，绝不逐条过边界）。
- **时间注入**（`clock.rs`）：`FfiClock` 调 `vtable.now_ms(ctx)`（确定性，host 注入）；None 时 fallback SystemTime + warn。
- **lifecycle**（`api.rs`）：`helix_command → tick_tx`（pump 消费 → step → Effect）；`helix_destroy` = drop tick_tx + send shutdown + **join pump 线程**（保证最后一批回调在 ctx 释放前返回）。
- **`helix_query`**：只提供 `helix_capabilities` / `helix_version` 元信息；业务查询必须经 `helix_command` 进入共享异步状态机，并从事件批次取结果。

> **本 crate 身兼 driver + composition root**：手机端无独立 app crate（C++ 直接 `helix_create`），故 `pump.rs` 在装配 `ExecutionShell` 时 `register(ImModule)`——这是合法装配点，FFI driver 装配业务模块不违反依赖铁律（对照：native 端由 `helix-host-cli` 做这件装配）。

模块：`api`（C ABI 导出 + HelixHandle）· `pump`（线程/runtime/装配 + composition root）· `event_sink`（批 egress）· `batch`（wire 编解码）· `clock`（host 时间）· `vtable`（C++ 回调表）· `http`（`FfiHttpRequester`，pump 实际使用的 HttpRequester 适配）。

## FFI 铁律（HX-C006）

1. 解引用裸指针的导出函数必须 `pub unsafe extern "C"` + `# Safety` doc（clippy 强制）
2. 每个 unsafe 块必配 `// SAFETY:` 注释（undocumented_unsafe_blocks = deny）
3. opaque ptr 单 owner：`Box::into_raw`/`from_raw` 各一次 + null-check
4. panic 策略二选一：生产 cdylib `panic="abort"`（无需 catch_unwind）/ dev unwind 逐导出 catch_unwind——禁混说
5. 同步 SQLite C 调用必包 `spawn_blocking`（MAJ-4，落地前 todo 不许偷偷删）
6. C 侧 buffer 立即拷贝不持有（ACL-3）；业务读查询走 `helix_command` + 批量 DomainEvent，`helix_query` 不得直查 Storage

## 改动验收

```bash
cargo check -p helix-driver-ffi && cargo clippy -p helix-driver-ffi --all-targets
cargo test -p helix-driver-ffi   # 含 command→Emit→批回调 e2e + pump lifecycle drain
# 改 pump 核（host engine/BatchSink）后 native 零回归安全网必跑：
cargo test -p helix-driver-native -p helix-driver-host
# C++ 直连冒烟（真 C++ link cdylib + #include helix.hpp，cpp→rust→cpp 端到端，CI 可调）：
bash scripts/cpp-smoke.sh   # cargo build cdylib → cmake build smoke/ → ctest
```
