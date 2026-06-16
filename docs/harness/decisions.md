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

## ADR-008 — sync-v2：helix 适配现网 Go 真实 wire，不改 Go server
- **决策**：sync-v2 落地时，helix（parser/sync_session/to_effect/im handler）**适配现网 Go 的真实 wire**——increment 走「`POST /api/cses/channels/load/increment` HTTP 触发 → 后台 goroutine → WS 单播 `increment_channel`×N + `increment_channel_end`」push 模型；离线对账走「`POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}` → `entries[]`」pull 模型；游标字段为 `event_seq`（三级 fallback：data.event_seq → props.channel_event_seq → 顶层 channel_event_seq）；鉴权 `cookieId` header = userId。**Go server 零改动。**
- **理由**：现网 Go `mattermost/server/channels/csesapi` 已服务生产 iOS/cses-client，反向让 Go 适配 helix 抽象会动生产服务端、影响现网 iOS，代价大。GOAL §4.5 定「现网 cses-client/Go 为行为真源」。helix 当前把 increment（push）与 sync/notify（pull）误揉成单一抽象 `GET /sync/notify` 拉模型 + 字段名 `seq`，是错位，须按真 wire 改回两条链路。
- **代码事实（2026-06-15 map 实测）**：helix-im parser 仅读 `data.seq` 一级、`GET /sync/notify`、不认 increment/hello 帧；on_start 不发 hello；全仓 0 `im:connection:established`；echo→reconcile 死代码；engine_loop Send 占位无 transport 路由表。详见 [`../过会方案/helix-sync-v2-real-go-wire-设计.md`](../过会方案/helix-sync-v2-real-go-wire-设计.md)。
- **配套未决**：connect 触发归属（`Effect::Connect` 意图 vs host-cli 主动 connect）放 Phase A2 拍；「运维消息」是否新立 `im:ops:*` 类待拍。
- **状态**：已定（2026-06-15）。落 `dev-zlc` 分支，A→B→C→D 分阶段全量实现。

## ADR-009 — mattermost ID（ChannelId 等）用 `[u8;26]` Copy newtype（Id26），非 u64/String
- **决策**：helix-im 的 ChannelId（及后续 MsgId/ActorId 等 mattermost ID）底层用定长 `Id26([u8;26])` Copy newtype 表示，不用 u64（抽象 wire 残留）、不用 String（热路径分配）。`ChannelId(Id26)` 保持 newtype 不透明；`Id26::from_str` 边界零信任（仅恰 26 字节 ASCII 才 Some），`as_str` 走 `from_utf8_unchecked`（构造已验证 ASCII 的 SAFETY 不变量，零拷贝）。
- **理由**：真 Go `channelId` 是 26 字符 base32（如 `5ffd465e263864787b87aa01`，真帧 broadcast.userId 实测印证）；helix 旧 `ChannelId(u64)` 是当初抽象简化 wire 的产物，对真 Go 不可用（Phase B 冒烟暴露）。ChannelId 是热路径 key（每事件命中、gates `HashMap<ChannelId,_>`、进每个 Effect），mattermost ID 恒为 26 char → 定长 `[u8;26]` 是 Copy、零分配（memcpy 26B，无 clone 成本）、FFI 跨界干净，对齐 HX-C005 热路径零分配；String 每次 clone 分配违背该铁律，Arc<str> 有原子引计微成本且非真 Copy。用户拍板取 `[u8;26]`。
- **代码事实**：仅在 helix-im（core 无 channel 概念，GOAL §3 grep channel=0，迁移不碰 core 零运行时）。测试经 `ChannelId::from_test_u64` 桥接（数值 id 确定性映射 26 字符），纯类型替换零行为变更。
- **状态**：已定（2026-06-16）。Phase B0 迁移子相落地；MsgId/ActorId 等同款 Id26 在 Phase C（phantom/read_echo）按需扩。
