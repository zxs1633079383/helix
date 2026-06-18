# helix-driver-ffi — 设计图索引

> Flutter FFI 端 driver（C-ABI / uniffi）的结构与数据流沉淀。
> 图以 mermaid `.mmd` 落盘，真源 = `crates/helix-driver-ffi/src/{api.rs, callbacks.rs, transport.rs, storage.rs, http.rs, lib.rs}`。
> **已接 vs 未接** 是本组图的核心：FFI 端 ✅ 网络 / 存储 / header / 内存安全已具备，❌ **engine 泵接线 = M3-F4 缺口**（见根 [`docs/ROADMAP.md`](../../../../docs/ROADMAP.md) §一 M3 F4 拆解）。

## 图清单

| 图 | 文件 | 一句话 |
|---|---|---|
| **设计思想** | [`cpp-direct/00-设计思想.md`](cpp-direct/00-设计思想.md) | FFI crate 角色宣言：为什么消灭 Dart 中间层 / Rust 持 WS 持 SQLite / FfiEventSink 批处理 / helix_query blocked-on-IM 来龙去脉 |
| ① C-ABI 边界全景 | [`01-c-abi-boundary.mmd`](01-c-abi-boundary.mmd) | Dart ↔ 7 个 C-ABI 导出函数 ↔ HelixHandle / 三个 Port 薄壳；✅ create/destroy/set_header/remove_header 已接，⚠️ on_ws_frame/on_http_response 拷贝后断头，❌ engine 泵 / helix_command / helix_query 未接（虚线红） |
| ② 当前数据流 vs 目标 F4 | [`02-dataflow-current-vs-f4.mmd`](02-dataflow-current-vs-f4.mmd) | 左现状（收帧→拷贝→`// TODO` 断头，engine 不被驱动）对比 右目标（→Tick::Inbound→engine step→Vec\<Effect\>→Persist 落库 / Emit→on_domain_event→Dart Stream 闭环） |
| ③ FFI 内存安全边界（HX-C006） | [`03-ffi-memory-safety.mmd`](03-ffi-memory-safety.mmd) | ① opaque ptr 单 owner（into_raw/from_raw 各一次 + null-check）② panic=abort 不跨 extern C unwind ③ C 侧 buffer 立即 to_vec 拷贝不持有（ACL-3）④ ffi_str 零信任守卫（null/utf8/invalid 错误码） |

## ✅ 已具备（src 已落地）

- **C-ABI 导出**：`helix_create` / `helix_create_with_network` / `helix_destroy` / `helix_set_header` / `helix_remove_header` / `helix_on_ws_frame` / `helix_on_http_response`
- **三个 Port 薄壳**：`FfiTransport`（Shared 复用 host WS · Callback 回调 Dart 逃生口）/ `FfiStorage`（一行委托 `HostStorage`，F2 ✅）/ `FfiHttpRequester`（委托 `SharedHttpClient`，F3 ✅）
- **回调表** `HelixCallbacks`：`send_ws_frame` / `send_http` / `on_domain_event` / `now_ms` / `ctx`（repr(C)，已定义）
- **FFI 内存安全（HX-C006）**：opaque ptr 单 owner + panic=abort + C 侧 buffer 立即拷贝（ACL-3）+ null / invalid-utf8 拒绝

## ❌ 未接 = M3-F4 缺口（图中虚线 / 红色标注）

> 拆解见根 [`docs/ROADMAP.md`](../../../../docs/ROADMAP.md) §一 M3 「F4 实装拆解」(F4.1–F4.5)。

1. **FFI 无 engine 泵** —— `helix_on_ws_frame`(api.rs:160) / `helix_on_http_response`(api.rs:187) 仅拷贝 buffer 后 `// TODO`，不喂 engine（F4.2，依赖 F4.1 泵归属 ADR）
2. **无 command 入口** —— `helix_command` / send_message 尚未导出（F4.2）
3. **事件回 Dart Stream 未接** —— `on_domain_event` 回调表已定义，但 `FfiEventSink impl EventSink` 未接（F4.3）
4. **无同步读路径 `helix_query`** —— F1 读路径缺口

## 维护约定

- 图为 source of truth 的**衍生视图**；改 `src/*.rs` 接缝（尤其接上 F4.2/F4.3 断头）后须同步更新对应 `.mmd` 与本表「已接 vs 未接」。
- ✅ 已具备用绿色 / ⚠️ stub 断头用黄色 / ❌ F4 未接用红色（虚线）——图例三色一致。
