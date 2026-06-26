# helix-driver-ffi 性能不变量（可证伪压测清单 · PI-1..11）

> 派生自 [`SPEC-full-takeover.md`](SPEC-full-takeover.md) §6。所有断言用**计数 / 结构 / 单调性**，
> **禁墙钟下界断言**（HX-C011）。纳入根 CLAUDE.md §4.7 drift-review lens。
>
> 一句话：FFI 边界的性能保证不靠「跑得快」的墙钟，而靠**结构性不变量 + 破坏即 fail 的证伪测试**——
> 任何「破坏机制后测试仍绿」的 PI 都是假达成。

| # | 不变量 | 校验方式（可证伪 + mutation） | 当前状态 |
|---|---|---|---|
| **PI-1** | 命令热路径 payload 仅 1 次必要拷贝（ACL-3）；name 命中闭集时 0 分配 | alloc 计数：命中 name 的 `helix_command` 净堆分配 == 1（payload）。破坏：去掉 name 闭集映射 → 分配==2 → fail | payload ✅ / name ✅（G-6 已落地：`canonical_command_name` 命中走 `Cow::Borrowed`）|
| **PI-2** | egress 全程零额外字节拷贝（`Bytes` Arc clone，非深拷贝） | 类型断言 `AppCommand.payload`/`DomainEventBytes` 是 `Bytes`；链路无 `.to_vec()`/`copy_from_slice`（除 ACL-3 入口） | ✅ 已达成 |
| **PI-3** | 一 step 的 N 条 Emit **恰好 1 次**过 FFI 边界（HX-C007） | 压测断言 `on_domain_events_batch` 调用次数 == **非空 step 数**（非事件数）。破坏：flush 改逐条回调 → 次数==事件数 → fail | ✅ 守门测试已落（`tests/pi3_pi6_gate.rs`）|
| **PI-4** | encode 缓冲稳态零分配（scratch 复用；依赖借出契约，非 ownership-transfer）| alloc 计数：稳态 step 编码路径净分配 == 0；scratch capacity 不增长。破坏：改回每 step 新 Vec → 稳态分配>0 → fail | ✅ 已落地：`encode_batch_into(events, &mut scratch)`，FfiEventSink 持 `scratch: Mutex<Vec<u8>>` 复用 |
| **PI-5** | 批 buffer 容量单次预估，零 realloc | 代码断言 `Vec::with_capacity(精确值)`；编码中无再分配 | ✅ `batch.rs` 预估，勿退化 |
| **PI-6** | 命令背压：channel 满返 `FFI_BUSY`，零静默丢弃，零无限缓冲 | 压测 `FFI_OK + FFI_BUSY == 发送总数`；cap 有界(256)。破坏：try_send 改 send / cap 改 ∞ → BUSY 永不出现 → fail | ✅ 守门测试已落（`tests/pi3_pi6_gate.rs`）|
| **PI-7** | core step 主循环不等 I/O（Persist/Http 入队即返回） | native stress_engine 守 Block 死锁安全；ffi 共用核 | ✅ |
| **PI-8** | 阻塞 I/O 全 `spawn_blocking` 卸载，不占 core 线程 | grep：`storage.rs` 所有 rusqlite 调用包 `spawn_blocking`（HX-C006 闸门）| ✅ |
| **PI-9** | blocking pool 上限显式封顶（防 DNS 风暴线程暴涨）| 代码断言 `Builder::new_current_thread().max_blocking_threads(N)` 显式设置 | ✅ 已落地：`pump.rs` `max_blocking_threads(32)` |
| **PI-10** | p99 单 step CPU 时间有预算上限（防单线程被慢 step 卡死）| 压测采 step **CPU 时间**分布（非墙钟），p99 < 预算作回归基线 | 待建基线（M2 锚）|
| **PI-11** | 生命周期内存稳态：create/destroy 多轮 RSS 斜率≈0，Arc/Box 计数归零 | Arc strong_count 断言 + 多轮 create/destroy 不 panic/泄漏。破坏：注释 `helix_destroy` 的 `Box::from_raw` → 泄漏 | ✅ 守门测试已落（`tests/pi11_lifecycle_leak.rs`，多轮 create/destroy 干净）|

## 已知边界（诚实声明）

- **PI-10 未建基线**：单 step CPU 预算回归锚 M2，本轮不实现（无可信压测床）。
- **PI-3 / PI-6 守门测试是 Rust 层** `tests/`：用 mock vtable 计回调次数 / 计返回码，**非真 C++ 压测**——
  真 C++ 端到端由 `scripts/cpp-smoke.sh` 覆盖（单 step 单批，不压并发）。并发承压留 M2。
- **G-3 lagged parity**：FFI 单线程下无 broadcast lagged 语义；native(resync_needed) vs ffi 对等信号
  走 `im_request_resync` 经 `helix_command` 回灌（不破 ABI），实现延 M2（SPEC §7 G-3）。
