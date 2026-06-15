# Helix 关键架构拍板（ADR-lite）

> 长运行项目里"已经定过的事"，防止反复推导。每条：决策 / 理由 / 状态。

## ADR-001 — Send/!Send：默认 Unsync，跨线程 opt-in Sync
- **决策**：helix-core 默认 `!Send`（单线程可跑）；port async 签名不无脑加 Send；native 驱动把 core 跑在专属单线程（spawn_local+LocalSet），并发归驱动；确需跨线程的状态才 opt-in Sync 存储。
- **理由**：wasm 单线程 `spawn_local` 接 `!Send`；硬加 Send 则 wasm 碎。Dioxus 同款（UnsyncStorage 默认 / SyncStorage opt-in）已 6 端验证。
- **状态**：已定。配套 HX-C002。

## ADR-002 — 状态载荷 owned，不用 `&'a`
- **决策**：跨 step / 进 async / 进 channel 的载荷与模块状态用 owned（如 `bytes::Bytes`）/ Copy handle / arena id，禁生命周期参数。
- **理由**：本仓 BLK-2（`Tick<'a>` 满足不了 `Sender<Tick<'static>>`）+ Dioxus `Scope<'a>` 生命周期地狱教训。
- **状态**：已定。配套 HX-C002。

## ADR-003 — wasm driver 误链守护 + CI 三栖 matrix
- **决策**：helix-driver-wasm 顶部 `#[cfg(not(wasm32))] compile_error!`；CI 必 per-target check（workspace 排除 wasm crate + wasm crate 单独 --target）。
- **理由**：cfg 分支只在对应 target type-check（MAJ-2）；防 wasm 符号误链 native。
- **状态**：已归档（2026-06-15 被 ADR-006 覆盖）。配套 HX-C003 作为历史护栏保留；当前产品 CI 不要求 wasm target。

## ADR-004 — CursorUpsert 是否算 core 通用原语（已闭环）
- **背景**：review 后实测 `grep channel/message/temporary crates/helix-core` = 0（纯 IM 概念隔离干净），但 `cursor`（`CursorUpsert` effect）进了 core（5 处）。
- **选项**：A) 保留为 core 通用「增量同步基建原语」（多数 sync 类业务都需要，可辩护）；B) 折进通用 `Upsert`，core 彻底不含 cursor 语义（纯粹派）。
- **拍板（纪要拍板 #1）**：取 **A 的精神 + 改名落地**——cursor 是 IM 概念归 `helix-im`，**不进 core**；core 只保留与业务无关的通用单调写原语，并把 `CursorUpsert`/`cursor_upsert` **更名为 `MonotonicUpsert`/`monotonic_upsert`**（MAX guard，类比 CAS），使 core 公开面不出现 "cursor" 一词。
- **代码事实（2026-06-10 起核实）**：`crates/helix-core` 内零处 `cursor_upsert`/`CursorUpsert`；`grep -i cursor crates/helix-core/src` 仅命中注释举例（非 API 名）。口径护栏：grep「零业务概念」的承诺对象只含 channel/message/temporaryId 三词，**不含 cursor**（不要声称 `grep cursor = 0`）。
- **状态**：已定。配套 [架构设计 §3.2/§4.9](../过会方案/helix-core-im-架构设计.md) + 纪要拍板 #1。GOAL §3 同步。

## ADR-005 — FFI 同步调用必须 spawn_blocking（实现待补）
- **决策**：FFI driver 的同步 SQLite C 调用包 `spawn_blocking`；不许阻塞 tokio worker。
- **理由**：MAJ-4。
- **状态**：契约已定（HX-C006），实现当前 `todo!()`，FFI 落地前必补（P1）。

## ADR-006 — 产品维护目标收缩为 PC Tauri + Flutter，wasm/web 暂停维护
- **决策**：Helix 当前产品化目标只覆盖两类客户端：PC 端 Tauri（`helix-driver-native`）与手机端 Flutter（`helix-driver-ffi`）。网页端暂不做，`helix-driver-wasm` 保留为历史实验/归档代码，不进入 roadmap、CI 必跑矩阵、交付验收或日常维护。
- **理由**：用户已拍板优先保证 PC + 手机端同步更新并降低维护成本；浏览器端 IndexedDB/OPFS/Worker/wasm 包体积会引入额外平台复杂度，当前不是产品需求。业务逻辑仍保持 runtime-agnostic、sans-IO、可回放，避免把未来 Web 可能性彻底写死，但不为未维护端付实现和验收成本。
- **状态**：已定（2026-06-15）。后续代码变更不得为了 wasm/web 额外扩 scope；如重新启动网页端，需新增 ADR 重新激活 `helix-driver-wasm` 和对应验收。

## ADR-007 — PC/Flutter 共享 Rust host driver 存储层
- **决策**：新增 `helix-driver-host` 作为 PC Tauri 与 Flutter FFI 的共享 Rust host driver 层。SQLite schema/migration、rusqlite Storage 实现、`StorageOp -> PortOutcome` 兑现、`rows_to_reply_bytes` / `rows_from_reply_bytes` 统一维护在该 crate；`helix-driver-native` 与 `helix-driver-ffi` 只保留平台壳与边界 API。
- **理由**：Flutter 手机端选择 Rust 侧全包 I/O，Dart 只负责 UI、FFI 命令与事件流。若 native 继续 sqlx、ffi 单独 rusqlite，messages SQL、monotonic 表、StorageOp 语义和 row 编解码会出现双维护漂移。统一 rusqlite + blocking worker/shared queue 优先维护性，并直接服务 PC + Flutter 两端。
- **状态**：已定（2026-06-15）。当前已落地共享 Storage/schema/StorageOp/rows 编解码；HTTP/WS 与更通用 engine loop 共享继续按不破坏平台壳边界增量推进。

## ADR-008 — PC/Flutter 共享 Rust host driver HTTP/WS client
- **决策**：HTTP client 与 WebSocket client 不属于 IM 模块，也不按端各写一套；它们是 `helix-driver-host` 的共享 host I/O 能力。`helix-driver-native` 与 `helix-driver-ffi` 只负责平台壳职责（Tauri command/event、FFI ABI、路径、生命周期、打包、灰度开关），通过同一套 shared HTTP/WS runtime 兑现 core 的 `HttpRequester` / `Transport` port。Flutter/Dart 与 Tauri 前端都只调用命令、订阅事件、渲染，不直接写业务 HTTP、IM sync、WS 长连、cursor、echo 对账或消息 SQL。
- **初始化边界**：shared HTTP/WS client 的代码只有一份，但初始化材料由端壳或登录模块提供：`api_base_url`、`ws_url`、默认 headers、WS 握手 headers、user-agent、timeout/proxy/TLS 等平台配置。Helix 不负责登录/刷新 token，只提供 `HostNetworkConfig` 与 header registry/update API（如 `set_header` / `remove_header` / `replace_headers`），由 shared client 在 HTTP 请求与 WS connect/重连时统一读取并注入。
- **边界**：业务模块（如 `helix-im`，未来 presence/meeting/notify）只组装业务请求意图、解析业务帧、维护业务状态机；它们不拥有网络 client，也不直接读写 token/header。登录/鉴权不纳入 Helix 业务边界，Helix 只消费外部已获得的 token/header/session 配置，并由 shared HTTP/WS client 统一注入到后续请求/连接。平台必须使用系统网络栈、证书代理或后台能力时，可保留 host callback 作为逃生口，但不是 PC/Flutter 产品主路径。
- **理由**：IM 只是第一个业务模块，若每个模块或每个端都各写 HTTP/WS，会导致重连、超时、header 注入、连接复用、错误分类和事件回灌语义漂移。共享 client 让“一个逻辑”落到真实工程结构：业务协议一份、I/O 执行一份、平台壳多份。
- **状态**：已定（2026-06-15）。当前 native HTTP/WS 已在 `helix-driver-native` 内有实现雏形，FFI HTTP/WS 仍有 host callback/占位历史口径；后续实现应把可共享的 HTTP/WS client、header registry、WS runtime、effect/engine loop 骨架增量下沉到 `helix-driver-host`，再由 native/ffi 薄壳复用。
