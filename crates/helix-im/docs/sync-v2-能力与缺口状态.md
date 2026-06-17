# helix-im · sync-v2 能力与缺口状态（落地真相）

> 快照日期：2026-06-16（commit 区间 8c69f25..97a33a8）。
> 真源核对：cses-client `features/im/`（行为）+ cses-im-server `internal/app/sync.go`（协议）+ 678 真库 schema。
> 设计全文：[`docs/过会方案/helix-sync-v2-real-go-wire-设计.md`](../../../docs/过会方案/helix-sync-v2-real-go-wire-设计.md)。
> 本卡只记「现在到哪了 + 缺什么 + 明天干什么」，不重复设计。

---

## 1. 三大能力（引擎层 ✅ / 业务落库 ⚠️ 部分）

| 能力 | 状态 | 实情 |
|---|---|---|
| HTTP | ✅ | NativeHttp 薄壳→`SharedHttpClient`(ADR-010) + 全局身份头 + increment 触发 + sync/notify |
| WS | ✅ | NativeTransport 薄壳→`SharedWsClient`(ADR-010，tungstenite 降 dev-dep) + connect + 入站 `Tick::Inbound`(A2) + hello 握手(A4) + 重连 |
| Storage | ✅ 引擎 / ⚠️ 业务 | rusqlite WAL + 读写分离 + 678 对齐 schema 真建表(message/channel/channel_member) + 多租户 Application Support 路径 + **发送路径**落 message + 真 DB 集成测试 |

> ⚠️ **cursor 真源 = `_helix_monotonic`**（通用单调表）。`channel_event_cursor` 曾作占位死表，已于 `cff383f` 从 IM_SCHEMA 删除（M2 接 cses-client tenant.db 时再引入 + 写迁移）。

## 2. sync-v2 落地表（约 85%，2026-06-17 更新）

| Phase | 状态 | 缺什么 |
|---|---|---|
| A 基础贯通（hello/身份头/transport/入站） | ✅ | — |
| B increment 真 wire | ✅（B3 sync/notify 真 wire + **B2 冷启动自愈** `heal_incomplete_backfill_once`，commit `20334d2`）| — |
| **C 正确性 + EventBus** | ✅ 实现 / 🔧 真帧修正中（2026-06-17 live 注入定位）| C1 echo 对账 · C2 phantom/离线落库/readBits · C3 mutation 事件 已实现单测绿。**🔴 真帧形态实测纠偏**：parser 原按猜测 action `"posted"` 识别（parser.rs:236），但真 Go 新消息广播 action 是 **`"post"`**（live 帧实证 event_seq=40113 + 全 message 内容；Go 常量 `WebsocketEventPost="post"` L28 ≠ `WebsocketEventPosted="posted"` L27）→ 真帧落 `Some(_)=>Ignored` 被丢、不进 gate、不落库。修复中（workflow wf_cc9003c9：classify_frame 认 `post`→gate apply；channel cursor 已正确种子=lastEventSeq，认了即 cursor+1 落库）。功能性事件(schedule/member)暂缓 |
| **D 真 Go smoke + golden fixtures** | ✅ 数据路径通过（2026-06-17）| ✅ 连接/握手/schema/连接层 + **数据路径真 Go 端到端验过**：真实 deviceId 解 grpc → increment 257 channel → 并发闸解 sync 风暴 → 真消息(curl 发 rmibkwkj event_seq=40114)经 WS `post` 帧 → parser 认 `post`(修) → gate apply(cursor+1) → **message 表落真实内容行 + temporary_id↔server_id 对账 + read_bits=100 + cursor 推进 40114 + post:received DomainEvent**。golden fixtures 工具链仍未建（P1） |

## 3. 缺口清单（2026-06-17 更新）

1. ✅ **离线历史消息上屏已实现**（`20334d2` C2）：sync `SyncEntry.messages`(Post map)→message 内容落库；phantom(msgId∉map)不落行但 cursor 照推。**真连验待数据流**（见 §5 今日必办）。
2. ✅ **echo 对账已激活**（`20334d2` C1）：`handle_echo` 接进 Inbound，tmp_id→server_id 覆写 message.id+send_status。**echo 真帧形态待核** + 真连验待发送。
3. ⚠️ **im:__bus__ 信封已接线**（`20334d2` C3，`to_bus_envelope`）；emit 补 post:read/updated/deleted；host-cli 无窗口→以 EventSink 计数验，真 app.emit 留 M2 Tauri 接缝。功能性事件(schedule/member/nickname/todo)暂缓。
4. 🔶 **真 Go 数据路径两轮复验（2026-06-17）：increment 已真连验过 ✅，sync 数据路径受阻于冷启动 sync 风暴 🔴**：
   - **layer-1 grpc ✅ 解**：第一轮 helix 发本机派生 deviceId（≠真实设备）→ paas session/deviceId 校验拒 → grpc `xpa-12-grpc` 返 `IsSuccess=false`（grpcx9.go:480，**业务级拒绝非 DNS**：consul 已发现实例 0→1）→ `cses_channel.go:177 parallel load failed`。**第二轮设 `HELIX_DEVICE_ID=ED380414…`（真实设备）→ `Increment worker completed`(cses_channel.go:234)，257 `increment_channel` 帧 + end 全落地、DomainEvent emit（62KB payload）。increment 数据路径真 Go 端到端验过。**
   - **layer-2 sync 风暴 🔴**：`increment_channel_end` → 每 needSync channel 一条 proactive sync/notify HTTP（state.rs:233，**无并发上限**）→ 冷启动 257 channel ~88+ 并发齐发 → **全 `error=Network`**（reqwest connect 失败，~30ms 内，未到 30s timeout；Go 侧 0 sync arrival）→ 0 消息回填 → message 表 0 行。state.rs:236 注释已预警「sync 风暴」。**修法：proactive sync fan-out 加并发闸**（semaphore/顺序/批量）；修完重跑走 C2/C3 判据。
5. **心跳 ping 占位帧**：长连稳定待 ping 实装（真 Go 25s 没断是 Go 容忍）。
6. **channel 表建了没人写**：increment 的 channel 元数据只 emit 不落库。
7. **架构 smell（drift-review 登记）**：✅ event_to_upsert_op 热路径(cff383f)/✅ channel_event_cursor 死表(cff383f)；⏳[D1] FFI 句柄类型(接 FFI 冒烟前)；⏳[D3] native·FFI registry 不对称(接 reconnect 前)；2 份 eventbus 稿保留。
8. **host-cli DB 路径 quirk**：`sqlite:{path}?mode=rwc` 把 `?mode=rwc` 当文件名一部分（DB 实际落 `xxx.db?mode=rwc`）——功能正常，路径丑，低优先修。

## 4. 一句话

引擎三件套(HTTP/WS/Storage)齐 + 678 持久化 + sync-v2 正确性层(C1/C2/C3)+B2 **实现完成**（单测+对抗审+回归全绿）。**现在代码能「连上·发出去对账·离线补齐(含 phantom/已读/编辑/撤回)·前端经 im:__bus__ 消费」**——但**数据路径的真 Go 端到端验证待真实消息流**（账号已追平，需制造消息活动）。

---

## 5. 待办

### ✅ 今日必办（用户拍板 2026-06-17）—— sync-v2 数据路径真 Go 端到端验证【DONE】

> ✅ **DONE sync-v2 数据路径真 Go 端到端验证 @2026-06-17 ~12:45 | helix dev-zlc | 真消息 rmibkwkj event_seq=40114 落 message 行 + cursor 推进 + post:received emit | 三层 blocker 全清（grpc-deviceId / sync-风暴-并发闸 / WS-post-action-parser）**
>
> **最终验证链（live curl 注入实证）**：curl 发真消息→Go `post sender success`→WS 广播 `post` 帧(event_seq=40114)→helix 收→parser 认 `post`(修)→gate apply(==cursor+1)→PersistFire→**message 表落真实内容行**(temporary_id `helixverify…`↔server id `hjh19t4t…` 对账 / message 内容 / read_bits=100)+ `_helix_monotonic` cursor→40114 + post:received DomainEvent。0 error / 0 sync Network fail。
>
> **三层 blocker 收口**：
> - **L1 grpc（deviceId）**：本机派生 deviceId≠真实设备→paas `IsSuccess=false`；设 `HELIX_DEVICE_ID=ED380414…` 真实设备→`Increment worker completed`，increment 257 channel 全通。
> - **L2 sync 风暴（并发闸，workflow wf_cc5ecd60）**：`increment_channel_end` 每 channel 一条 proactive sync 无并发上限→冷启动 257 并发齐发全 `Network`；`engine_loop.rs` 加 `Arc<Semaphore>`(permit 在 spawned task 内 acquire，泵不阻塞)，N=`HELIX_HTTP_MAX_INFLIGHT`(默认8) 经 EngineConfig；双对抗审 pass→Network 88→0，SyncChannels 0→257。
> - **L3 WS post 真帧（parser，workflow wf_cc9003c9）**：parser 原按猜测 action `"posted"` 识别，真 Go 新消息广播是 `"post"`(Go 常量 L28，≠`posted` L27)→真帧 `Some(_)=>Ignored` 被丢；`classify_frame` 加 `Some("post")=>FrameClass::Post(EventEnvelope)`→`ch.ingest` gate；现网对齐审 pass + 对抗审揪出「self-echo 不 reconcile→15s 后误标 unsend」blocking→Fix 补 `reconcile_post_echo`(CancelTimer)→cargo test 全绿(post_action_inbound 6/6 等 18 bin)。
>
> **未提交**：L2+L3 代码改动仍在工作区（engine_loop.rs/host-cli/parser.rs/module.rs/sync_session.rs + 测试 + 6 处 EngineConfig 构造点 + docs），待用户确认提交。golden fixtures 工具链(P1) 仍未建。
>
> ---
> 历史诊断过程（留档）：⚠️ **2026-06-17 复验（两轮，DNS 修复后）：blocker 连下两层——layer-1 grpc 已解，layer-2 sync 风暴新现**。
>
> **Layer 1 ✅ 已解（真实 deviceId 修复）**：第一轮 helix 发本机派生 deviceId `86C35B0E…`（≠user 真实设备）→ paas 按 session/deviceId 校验拒 → grpc `xpa-12-grpc` 返 `IsSuccess=false`（grpcx9.go:480）→ `cses_channel.go:177 parallel load failed "grpc service fail"`。**非 DNS/连通性**（consul 已发现实例 0→1，是业务级拒绝）。第二轮 `HELIX_DEVICE_ID=ED380414…`（user 真实设备）→ Go `Increment worker completed`(cses_channel.go:234) → **increment 全通**：257 `increment_channel` 帧 + `increment_channel_end` 落地，DomainEvent emit（含 62KB channel payload）。✅ **increment 数据路径真 Go 端到端验过**。
>
> **Layer 2 ✅ 已修（并发闸，workflow wf_cc5ecd60 落地）**：原症——`increment_channel_end` 对 `(increment_fetched − need_sync_skip)` **每 channel 发一条 proactive sync/notify HTTP**（state.rs:233，无并发上限）→ 冷启动 257 channel 全 `needSync:true` → ~88 并发 sync/notify 在 ~30ms 齐发 → **全 `error=Network`**（reqwest connect 失败，未到 30s timeout）→ 0 回填。**修法**：`helix-driver-native/engine_loop.rs` 给 Http effect dispatch 加 `Arc<Semaphore>` 有界并发闸——permit 在 spawned task 内 `acquire_owned().await`（泵线程只 clone+spawn，永不阻塞→无自死锁），N 由 host-cli 读 `HELIX_HTTP_MAX_INFLIGHT`（默认 8）经 EngineConfig 注入；core/im 零改（sans-IO 守住）。rust-dev + core-dev 双对抗审 pass（死锁/permit 全路径 release/闸真生效/HX-C001-002）。**复验**：N=8/12 重跑 → **sync/notify Network 88→0，Go `SyncChannels` invoke 0→257，全部 round-trip 完成**。✅ sync 风暴消除、sync 数据路径接通。
>
> **当前真相（数据路径全通，缺真实消息内容）**：Go 257 条 sync 全返 `cursor decision kind=no_change`（账号已追平）→ helix `SyncResponse::NoChange` 不落库 → message 表 0 行 = **正确行为**（无离线内容可回填，非 bug）。**全链路已验**：connect→hello→increment(257 channel)→有界 sync fan-out(257 round-trip 0 失败)→no_change 正确不落。**唯缺**：真实消息内容触发 `events`-kind sync → 落 message 行（需制造消息活动，见 §5）。C2 落库逻辑本身单测+对抗审已绿。
>
> **就绪验证配方（二选一）**：
> - **选 1（验 C1 echo + C2，需先加 host-cli 发送）**：给 host-cli 加 `HELIX_SMOKE_SEND=<channelId>` 启动发一条测试消息 → Go echo 回流验 tmp_id→server_id 覆写 + 该消息进 message 表。⚠️ 往真频道发 1 条真消息。
> - **选 2（验 C2 离线落库 + C3 mutation，无副作用）**：用户从 web/另一设备往本账号频道发几条（含编辑/撤回/已读）→ 同时跑 `HELIX_DB=/tmp/v.db?... cargo run -p helix-host-cli`（dev-loop.sh creds）→ 查 `message` 表有内容行 + `read_bits` 正确 + DomainEvent 含 post:read/updated/deleted。
>
> **验收判据**：message 表有真实内容行 + phantom 不落但 cursor 推进 + read_bits 单调 + echo 回正 id + 对应 DomainEvent 流出。绿 → sync-v2 全验收通过。

### P1 — 收尾
- [ ] golden fixtures 工具链（`HELIX_DUMP_FRAMES=1` 抓 echo/sync-with-messages/type2-3-6 帧 → `tests/fixtures/real_go_frames/`）+ echo 真帧形态核对。
- [ ] 心跳 ping 对齐（缺口 #5）：ping 帧 + 间隔对齐 hello.pingInterval(60s)。
- [ ] host-cli DB 路径 quirk（`?mode=rwc` 当文件名，缺口 #8）。

### 历史（已完成，留档）
- ✅ C1 echo→reconcile（`20334d2`）· ✅ C2 phantom/离线落库/readBits（`20334d2`）· ✅ C3 mutation 事件+bus（`20334d2`）· ✅ B2 冷启动自愈（`20334d2`）
- ✅ HX-C005 热路径回归先修（cff383f）

### P1 — EventBus 全量 + 离线完整性
- [ ] **C3 emit 补全 6→24 事件 + `im:__bus__` 信封接线**（缺口 #3）：NativeEventSink→app.emit 信封，对齐现网前端总线契约。
- [ ] **B2 increment_end 冷启动自愈**。
- [ ] **B3 readBits**（type=6 已读位）+ store:* 对齐（Tauri 接缝预备）。

### P2 — 架构债收敛（drift-review 登记）
- [x] **[low] 删 channel_event_cursor 死表** ✅ cff383f。
- [x] **2 份 eventbus 设计稿** ✅ 用户拍板保留（ad1c932 契约 / 3c5324f 域卡）。
- [ ] **[medium·D1] FFI 句柄不透明类型统一**（接 Swift/Kotlin FFI 冒烟前；HX-C006）：api.rs create/destroy/on_* 的 `*mut c_void` 与 `helix_create_with_network` 的 `*mut HelixHandle` 撞型，统一其一。
- [ ] **[medium·D3] native·FFI header registry 对称化**（接 WS reconnect 前）：host-cli 改 `with_registry` 共享单一 registry（HTTP+WS），删 ws_headers 重复路径。
- [ ] **driver-host→helix-im 反向 dev-dep**：评估是否换成更轻的真 DB 测试方式。

### P3 — D 收官
- [ ] **golden fixtures 工具链**（im_trace 导出 → Tick 剧本，确定性 id/时间注入）。
- [ ] **UC-1 端到端真 Go smoke**（起 Go 服务端 + 真 cookieId，发消息→上屏→echo→正式 id；离线>500→续拉至 no_change，cursor==maxSeq）。
