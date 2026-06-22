# helix-driver-ffi — 设计图索引

> Flutter FFI 端 driver（C-ABI / uniffi）的结构与数据流沉淀。
> 图以 mermaid `.mmd` 落盘，真源 = `crates/helix-driver-ffi/src/{api.rs,pump.rs,event_sink.rs,http.rs,lib.rs}`。
> F4 已落地：FFI pump 线程直接装配 shared host `run_engine_loop`、`HostStorage`、`SharedHttpClient`
> 与 `SharedWsClient`，C++ 侧只通过 C ABI 发命令和接收批量事件。crate 行为真源 = [`../../CLAUDE.md`](../../CLAUDE.md)。
> ⚠️ 下方 `.mmd` 图为 F4 落地前快照，「左现状断头 / engine 不被驱动」部分已过时，待重绘（图例见文末）。

## 图清单

| 图 | 文件 | 一句话 |
|---|---|---|
| **设计思想** | [`cpp-direct/00-设计思想.md`](cpp-direct/00-设计思想.md) | FFI crate 角色宣言：为什么消灭 Dart 中间层 / Rust 持 WS 持 SQLite / FfiEventSink 批处理 / helix_query blocked-on-IM 来龙去脉 |
| ① C-ABI 边界全景 | [`01-c-abi-boundary.mmd`](01-c-abi-boundary.mmd) | C++/Flutter host ↔ C-ABI 导出函数 ↔ HelixHandle / pump；create/destroy/header/command/query stub 已接，storage 由 pump 直接装配 `HostStorage`（`helix_on_ws_frame/on_http_response` 已删，入站走 `with_inbound_tick`） |
| ② F4 后数据流 | [`02-dataflow-current-vs-f4.mmd`](02-dataflow-current-vs-f4.mmd) | C++ 命令 / shared WS 入站 → Tick → shared host engine → Persist/Http/Send/Emit，Persist 经 `HostStorage`，Emit 经批量 C++ vtable |
| ③ FFI 内存安全边界（HX-C006） | [`03-ffi-memory-safety.mmd`](03-ffi-memory-safety.mmd) | ① opaque ptr 单 owner（into_raw/from_raw 各一次 + null-check）② panic=abort 不跨 extern C unwind ③ C 侧 buffer 立即 to_vec 拷贝不持有（ACL-3）④ ffi_str 零信任守卫（null/utf8/invalid 错误码） |

## ✅ 已落地（src 已实装，含 F4）

- **C-ABI 导出**：`helix_create_with_network` / `helix_destroy` / `helix_set_header` / `helix_remove_header` / `helix_command` / `helix_query`（honest stub）/ `helix_query_free`。
  > 注：旧 `helix_on_ws_frame` / `helix_on_http_response` **已删**——入站帧由 `SharedWsClient::with_inbound_tick` 在 Rust 内部直发 `Tick::Inbound`，不再经 C++（见 `pump.rs`）。
- **engine 泵（F4 ✅）**：`pump.rs` 起专用 `std::thread` + current_thread tokio runtime，`block_on` host `run_engine_loop`（复用 `helix-driver-host` 泛型泵核）；装配 storage/http/ws/eventsink/clock + `register(ImModule)`（FFI 身兼 composition root）。
- **共享 host ports**：pump 直接装配 `HostStorage`（StorageOp 单一真源，含 BatchDelete）、`SharedWsClient`、`FfiHttpRequester(SharedHttpClient)`、`FfiEventSink`。
- **egress 批处理（F4.3 ✅，HX-C007）**：`FfiEventSink impl EventSink + BatchSink`，每 step `flush` 整批编码成单 buffer（`batch.rs` len-prefixed）→ **一次** `CppHostVtable::on_domain_events_batch`，绝不逐条过边界。
- **C++ vtable**：`CppHostVtable { on_domain_events_batch, now_ms, ctx }`，事件按 step 批量回调（HX-C007）。
- **FFI 内存安全（HX-C006）**：opaque ptr 单 owner + panic=abort + C 侧 buffer 立即拷贝（ACL-3）+ null / invalid-utf8 拒绝。

## 仍未实装

1. **同步读路径 `helix_query`** —— runtime handle + `Arc<HostStorage>` 已就位，但 query name → spec 映射属于 IM 域（填进 driver 会泄漏 schema），当前返回 `FFI_UNIMPLEMENTED`；待 IM 通用 query 路由后接 `spawn_blocking + oneshot`（ROADMAP P2 FFI MAJ-4）。
2. **Flutter 真机 smoke / F5 parity** —— 见根 [`docs/ROADMAP.md`](../../../../docs/ROADMAP.md) §一 M3 F4.5/F5。

## 维护约定

- 图为 source of truth 的**衍生视图**；F4 已落地，`01/02` 图的「断头 / engine 不被驱动」部分**已过时待重绘**；改 `src/*.rs` 接缝后须同步更新对应 `.mmd` 与本表。
- ✅ 已落地用绿色 / ⏳ honest stub 用黄色 / ❌ 未接用红色（虚线）——图例三色一致。
