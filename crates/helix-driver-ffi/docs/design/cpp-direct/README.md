# helix-driver-ffi — C++ 直连模型 · 边界面文档

> 本目录记录 C++ 直连模型的边界面设计 + M3-F4 pump 落地实录。
> pump 主循环已实装（commit d8c3093..630a713，e2e 通过）；`helix_query` 仅保留 capabilities/version，业务查询走异步主路径。
> 源码真源：`crates/helix-driver-ffi/src/`；头文件：`crates/helix-driver-ffi/helix.hpp`。

---

## 模型变更（Dart-host → C++ 直连）

| 维度 | 旧模型（Dart-host）| 新模型（C++ 直连）|
|---|---|---|
| 网络持有方 | Dart host 持网络，喂帧给 Rust | **Rust 持有** SharedWsClient + SharedHttpClient |
| 帧注入 | `helix_on_ws_frame` / `helix_on_http_response` | **已删**（Dart-host 遗物）|
| host 回调表 | `HelixCallbacks`（WS send + HTTP send + event + now_ms）| **`CppHostVtable`**（仅 on_domain_events_batch + now_ms）|
| 事件回传 | 逐条 `on_domain_event` | **整批** `on_domain_events_batch`（HX-C007）|

---

## 边界面表

见 [01-boundary-table.md](01-boundary-table.md)。

---

## M3-F4 落地实录

见 [02-m3-f4-todo.md](02-m3-f4-todo.md)（pump 各项落地过程；当前事实以源码为准）。

---

## helix_query 契约

见 [`../../移动端Command-Query-Event契约SPEC.md`](../../移动端Command-Query-Event契约SPEC.md)；实现以 `src/api.rs` 为准。
