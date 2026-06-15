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
