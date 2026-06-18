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

## ADR-005 — FFI 同步调用必须 spawn_blocking（storage 已落地，engine 接线待 M3-F4）
- **决策**：FFI driver 的同步 SQLite C 调用包 `spawn_blocking`；不许阻塞 tokio worker。
- **理由**：MAJ-4。
- **状态**：storage 端 `spawn_blocking` **已落地**于 `helix-driver-host/src/storage.rs` ×7（native/FFI 共享 host 存储栈，一次满足两端继承，ADR-007）；FFI 经 `FfiStorage` 薄壳委托即继承。**C-ABI 导出层 engine_loop 接线仍 stub（ROADMAP M3-F4，非架构偏移）**。契约 HX-C006 已守。

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
- **代码事实（2026-06-15 map 实测）**：helix-im parser 仅读 `data.seq` 一级、`GET /sync/notify`、不认 increment/hello 帧；on_start 不发 hello；全仓 0 `im:connection:established`；echo→reconcile 死代码；engine_loop Send 占位无 transport 路由表。详见 [`../过会方案/helix-sync-v2-设计.md`](../过会方案/helix-sync-v2-设计.md)（合并自 real-go-wire + 完成-spec）。
- **配套未决**：connect 触发归属（`Effect::Connect` 意图 vs host-cli 主动 connect）放 Phase A2 拍；「运维消息」是否新立 `im:ops:*` 类待拍。
- **状态**：已定（2026-06-15）。落 `dev-zlc` 分支，A→B→C→D 分阶段全量实现。

## ADR-009 — mattermost ID（ChannelId 等）用 `[u8;26]` Copy newtype（Id26），非 u64/String
- **决策**：helix-im 的 ChannelId（及后续 MsgId/ActorId 等 mattermost ID）底层用定长 `Id26([u8;26])` Copy newtype 表示，不用 u64（抽象 wire 残留）、不用 String（热路径分配）。`ChannelId(Id26)` 保持 newtype 不透明；`Id26::from_str` 边界零信任（仅恰 26 字节 ASCII 才 Some），`as_str` 走 `from_utf8_unchecked`（构造已验证 ASCII 的 SAFETY 不变量，零拷贝）。
- **理由**：真 Go `channelId` 是 26 字符 base32（如 `5ffd465e263864787b87aa01`，真帧 broadcast.userId 实测印证）；helix 旧 `ChannelId(u64)` 是当初抽象简化 wire 的产物，对真 Go 不可用（Phase B 冒烟暴露）。ChannelId 是热路径 key（每事件命中、gates `HashMap<ChannelId,_>`、进每个 Effect），mattermost ID 恒为 26 char → 定长 `[u8;26]` 是 Copy、零分配（memcpy 26B，无 clone 成本）、FFI 跨界干净，对齐 HX-C005 热路径零分配；String 每次 clone 分配违背该铁律，Arc<str> 有原子引计微成本且非真 Copy。用户拍板取 `[u8;26]`。
- **代码事实**：仅在 helix-im（core 无 channel 概念，GOAL §3 grep channel=0，迁移不碰 core 零运行时）。测试经 `ChannelId::from_test_u64` 桥接（数值 id 确定性映射 26 字符），纯类型替换零行为变更。
- **状态**：已定（2026-06-16）。Phase B0 迁移子相落地；MsgId/ActorId 等同款 Id26 在 Phase C（phantom/read_echo）按需扩。

## ADR-010 — PC/Flutter 共享 Rust host driver HTTP/WS client
> 📌 merge 注：本 ADR 在 main 分支原编号 ADR-008，与 dev-zlc 的 ADR-008（sync-v2）号撞；2026-06-16 merge main→dev-zlc 时改号 ADR-010（内容不变，避免双 008）。
- **决策**：HTTP client 与 WebSocket client 不属于 IM 模块，也不按端各写一套；它们是 `helix-driver-host` 的共享 host I/O 能力。`helix-driver-native` 与 `helix-driver-ffi` 只负责平台壳职责（Tauri command/event、FFI ABI、路径、生命周期、打包、灰度开关），通过同一套 shared HTTP/WS runtime 兑现 core 的 `HttpRequester` / `Transport` port。Flutter/Dart 与 Tauri 前端都只调用命令、订阅事件、渲染，不直接写业务 HTTP、IM sync、WS 长连、cursor、echo 对账或消息 SQL。
- **初始化边界**：shared HTTP/WS client 的代码只有一份，但初始化材料由端壳或登录模块提供：`api_base_url`、`ws_url`、默认 headers、WS 握手 headers、user-agent、timeout/proxy/TLS 等平台配置。Helix 不负责登录/刷新 token，只提供 `HostNetworkConfig` 与 header registry/update API（如 `set_header` / `remove_header` / `replace_headers`），由 shared client 在 HTTP 请求与 WS connect/重连时统一读取并注入。
- **边界**：业务模块（如 `helix-im`，未来 presence/meeting/notify）只组装业务请求意图、解析业务帧、维护业务状态机；它们不拥有网络 client，也不直接读写 token/header。登录/鉴权不纳入 Helix 业务边界，Helix 只消费外部已获得的 token/header/session 配置，并由 shared HTTP/WS client 统一注入到后续请求/连接。平台必须使用系统网络栈、证书代理或后台能力时，可保留 host callback 作为逃生口，但不是 PC/Flutter 产品主路径。
- **理由**：IM 只是第一个业务模块，若每个模块或每个端都各写 HTTP/WS，会导致重连、超时、header 注入、连接复用、错误分类和事件回灌语义漂移。共享 client 让“一个逻辑”落到真实工程结构：业务协议一份、I/O 执行一份、平台壳多份。
- **状态**：已定并阶段落地（2026-06-16）。`helix-driver-host` 已提供共享 `HostNetworkConfig`、header registry、`SharedHttpClient`、`SharedWsClient`；`helix-driver-native` 改为薄壳复用 shared client；`helix-driver-ffi` 的 Rust 侧主路径复用 shared client，并暴露 `helix_set_header` / `helix_remove_header`。保留项：WS reconnect/backoff、proxy/TLS/cert 等平台 knobs、旧 callback fallback 的完整接线，以及真 Go smoke。
> ✅ transport.rs 已适配本 ADR（6b2fd52）：`NativeTransport` 薄壳复用 `SharedWsClient`，tokio-tungstenite 降为 dev-dep（仅集成测试）；真 Go 硬修保住（Text 帧 / 握手头灌 registry / 不回客户端 hello / 入站 `Tick::Inbound` 推送 via `with_inbound_tick`），真 Go :8065 直连 22s 回归通过（0 个 8s 断连）。保留项：WS reconnect/backoff、proxy/TLS/cert knobs、旧 callback fallback、native·FFI 共享 registry 对称化（drift D3 待修）。

---

## 过会拍板结论（原 helix-技术评审纪要，2026-06-17 折叠）

> 这 7 条 + 头号风险原是 `docs/过会方案/helix-技术评审纪要.md` 的内容（8 轮逐条拷问 + 三视角合议的拍板结论）。
> 文档大缩减时纪要稿删除，**决策权威源已并入本文件**——本节自此是这些过会结论的唯一事实源。
> 与现有 ADR 的关系：纪要拍板 #1 = ADR-004（cursor 归属）；其余条目是更宽口径的过会级结论，不与 ADR-001..010 冲突。

**评审总结论**：能过会——靠「诚实 + 原点守住」过，不是靠「无懈可击」过。原点（业务逻辑写多份 / Bug 修多处行为分叉 / 业务逻辑测不动）经得起敲，技术核心方向正确，未建完的部分被主动摆上桌、且有明确的最低成本验证路径。

| # | 拍板结论 | 内容 |
|---|---|---|
| 1 | **cursor 归属** | cursor 是 IM 模块的同步游标概念，**归 `helix-im`，不进 core**；core 只保留与业务无关的通用 spec 原语（`CursorUpsert`→`MonotonicUpsert` 更名落地）。= ADR-004。 |
| 2 | **栅栏升级：禁词清单 → 公开面冻结** | 「core 零业务概念」从 `grep 禁词`（denylist）升级为 `cargo public-api` **快照冻结 core 公开类型面**（allowlist）；任何新增具名业务类型即 CI 红——拦的是「新增了类型」这个结构事实，而非某个词。 |
| 3 | **四能力解耦定性 = 行为级（A）** | Tick/Effect 提供的是**行为解耦**（模块只 emit 自己需要的 Effect），**非编译级裁剪**；「feature additive / 单独构建 / 编译级互不依赖」措辞全部校准为「行为级解耦」。唯一保留的编译级口子（B）：wasm 包体积——若 wasm size 真咬人，再为 wasm 单独拆 `Effect` 子枚举。`Effect` 仍是**单一 enum**。 |
| 4 | **重试归 driver** | helix-im 不重试、失败只标 UnSend（源码 `ports/http.rs` 注释为准）；重试 / 退避是 driver 私有责任。 |
| 5 | **DomainEvent 契约必须版本化** | 业务逻辑全进 Rust 后，core 与前端唯一对话面是 DomainEvent 事件流；该契约**必须版本化**（信封带 `v`），否则「业务回调地狱」会换成「事件 schema 对不齐地狱」。 |
| 6 | **三端说法诚实校准** | 措辞从「三端在跑」改为「**两端在跑**（桌面 Tauri/Angular + 移动 Flutter）+ 浏览器在路上」。（注：浏览器端后被 ADR-006 进一步收缩为暂停维护。） |
| 7 | **driver 方法体当前全是 `todo!()`** | 流程图 / 序列图是「编译期类型验证为真的契约意图」，运行时兑现待 M2。（注：本条为 2026-06-17 折叠时的状态；之后 native transport/http/storage 主路径已逐步落地，见 ADR-007/008/010。） |

**头号技术风险（主动上桌）**：乱序增量同步（断线重连 backfill）尚未验证——架构在最干净的发消息链上已证，但在最脏的「乱序帧缓冲 + gap 检测 + 按序补齐」链上一字未证。
**最低成本验证路径（M1 第一优先级）**：**先写乱序 backfill 的 proptest，再写任何 driver**——在 `helix-im` 内用 owned 状态（`pending: BTreeMap<Seq,Frame>` 缓冲、连续水位 cursor、bitmap 已拉区间）+ `ScheduleTimer` 驱动「攒 1s flush」，喂任意乱序 `Tick::Inbound` + `Tick::Timer`，断言落库顺序 == 排序后顺序。范式参考 Telegram `getDifference(pts)`。

> 进度脚注（2026-06-17）：拍板 #4/#5/#7 与后续 sync-v2 落地一致；sync-v2 数据路径已真 Go 端到端验证（见 [`../过会方案/helix-sync-v2-设计.md`](../过会方案/helix-sync-v2-设计.md)），但本条头号风险中的「断线重连 backfill 全脏路径 proptest」仍为 M1 收尾项，未因 sync-v2 数据路径打通而豁免。
