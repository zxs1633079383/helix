# helix-driver-ffi — C++ 直连模型 · 边界面文档

> 本目录记录 C++ 直连模型的边界面设计 + M3-F4 pump 落地实录。
> pump 主循环已实装（commit d8c3093..630a713，e2e 通过）；仅 `helix_query` 仍为 honest stub。
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

见 [02-m3-f4-todo.md](02-m3-f4-todo.md)（pump 各项已落地形态 + `helix_query` 仍为真 TODO 的依据）。

---

## helix_query 设计（blocked-on-IM）

见 [03-helix-query-design.md](03-helix-query-design.md)（为什么不在 driver 层硬编 name→Spec + 正解路由设计 + 实装解封条件）。
