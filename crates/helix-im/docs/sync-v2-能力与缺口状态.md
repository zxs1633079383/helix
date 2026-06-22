# helix-im · sync-v2 能力与缺口状态（落地真相）

> 快照日期：2026-06-22（main HEAD `d401d55`；sync-v2 已并入 main，`feat/dev-zlc` 已删）。
> 真源核对：cses-client `features/im/`（行为）+ cses-im-server `internal/app/sync.go`（协议）+ 678 真库 schema。
> 设计全文：[`docs/过会方案/helix-sync-v2-设计.md`](../../../docs/过会方案/helix-sync-v2-设计.md)。
> 本卡只记「现在到哪了 + 缺什么 + 明天干什么」，不重复设计。

---

## 1. 三大能力（引擎层 ✅ / 业务落库 ⚠️ 部分）

| 能力 | 状态 | 实情 |
|---|---|---|
| HTTP | ✅ | NativeHttp 薄壳→`SharedHttpClient`(ADR-010) + 全局身份头 + increment 触发 + sync/notify |
| WS | ✅ | NativeTransport 薄壳→`SharedWsClient`(ADR-010，tungstenite 降 dev-dep) + connect + 入站 `Tick::Inbound`(A2) + hello 握手(A4) + 重连 |
| Storage | ✅ 引擎 / ⚠️ 业务 | rusqlite WAL + 读写分离 + 678 对齐 schema 真建表(message/channel/channel_member) + 多租户 Application Support 路径 + **发送路径**落 message + 真 DB 集成测试 |

> ⚠️ **IM cursor 真源 = `channel_event_cursor`**（byte 对齐现网 678 `tables/channel_event_cursor.rs`，三列 channel_id/last_event_seq/updated_at + MAX guard；写路径 `acl/to_effect::advance_cursor` 走 `MonotonicUpsert{table=channel_event_cursor}`，读路径 `on_start` 吐 `Scan(channel_event_cursor)`→`handle_scan_reply` 解析，见 `schema.rs:128-141`）。
> `_helix_monotonic` 是 **core 通用单调原语**（driver `migrate()` 自建），仅 stress/conformance 用，**不是 IM cursor 表**。
> 沿革：`cff383f` 曾删占位 `channel_event_cursor` 死表，后 `9ed9d8d`（cursor 改表）将其复活为权威落库表以对齐 678——本注脚与 §3.7 据此更正（曾误记为 `_helix_monotonic`/已删）。

## 2. sync-v2 落地表（约 90%，2026-06-22 更新；A/B/C/D 全 phase 入 main）

| Phase | 状态 | 缺什么 |
|---|---|---|
| A 基础贯通（hello/身份头/transport/入站） | ✅ | — |
| B increment 真 wire | ✅（B3 sync/notify 真 wire + **B2 冷启动自愈** `heal_incomplete_backfill_once`，commit `20334d2`）| — |
| **C 正确性 + EventBus** | ✅ 实现 + 真帧修正已落地（45a621c/7270b6b 入 main）| C1 echo 对账 · C2 phantom/离线落库/readBits · C3 mutation 事件 已实现单测绿。**✅ 真帧形态已纠偏并验证**：parser 原按猜测 action `"posted"` 识别，真 Go 新消息广播 action 是 **`"post"`**（Go 常量 `WebsocketEventPost="post"` ≠ `WebsocketEventPosted="posted"`）→ 已改 parser 认 `post`→gate apply；并补 `reconcile_post_echo`（CancelTimer）解 self-echo 误标 unsend。真机 e2e 8/8 wire-correct（见 §5 DONE + `真机curl真源.md`）。功能性事件(schedule/member)暂缓 |
| **D 真 Go smoke + golden fixtures** | ✅ 数据路径通过（2026-06-17）| ✅ 连接/握手/schema/连接层 + **数据路径真 Go 端到端验过**：真实 deviceId 解 grpc → increment 257 channel → 并发闸解 sync 风暴 → 真消息(curl 发 rmibkwkj event_seq=40114)经 WS `post` 帧 → parser 认 `post`(修) → gate apply(cursor+1) → **message 表落真实内容行 + temporary_id↔server_id 对账 + read_bits=100 + cursor 推进 40114 + post:received DomainEvent**。golden fixtures 工具链仍未建（P1） |

## 3. 缺口清单（2026-06-17 更新）

1. ✅ **离线历史消息上屏已实现**（`20334d2` C2）：sync `SyncEntry.messages`(Post map)→message 内容落库；phantom(msgId∉map)不落行但 cursor 照推。**真连验待数据流**（见 §5 今日必办）。
2. ✅ **echo 对账已激活并真机验过**（`20334d2` C1 + outbound 体对齐 `7270b6b`）：`handle_echo` 接进 Inbound，tmp_id→server_id 覆写 message.id+send_status；真机 send → Go 回 `post` 帧回声 → 对账打通（e2e 8/8 wire-correct）。**注**：旧"send 无回声"真因 = outbound 请求体不对（非 Pulsar 兜底），已按 `真机curl真源.md` 逐字对齐修复。
3. ⚠️ **im:__bus__ 信封已接线**（`20334d2` C3，`to_bus_envelope`）；emit 补 post:read/updated/deleted；host-cli 无窗口→以 EventSink 计数验，真 app.emit 留 M2 Tauri 接缝。功能性事件(schedule/member/nickname/todo)暂缓。
4. ✅ **真 Go 数据路径端到端验过（三层 blocker 全清，详见 §5 DONE）**：
   - **layer-1 grpc ✅ 解**：本机派生 deviceId（≠真实设备）→ paas session/deviceId 校验拒 → grpc `IsSuccess=false`（业务级拒绝非 DNS）。设 `HELIX_DEVICE_ID=<真实设备>` → `Increment worker completed`，257 `increment_channel` 帧 + end 全落地、DomainEvent emit。
   - **layer-2 sync 风暴 ✅ 已修（并发闸，wf_cc5ecd60）**：`increment_channel_end` → 每 needSync channel 一条 proactive sync/notify 无并发上限 → 冷启动 257 并发齐发全 `Network`。修法：`engine_loop.rs` 给 Http effect dispatch 加 `Arc<Semaphore>`（permit 在 spawned task 内 acquire，泵不阻塞），N=`HELIX_HTTP_MAX_INFLIGHT`(默认8) 经 EngineConfig。复验 Network 88→0、SyncChannels 0→257。
   - **layer-3 WS post 真帧 ✅ 已修**：见 §2 Phase C（parser 认 `post` + reconcile_post_echo）。
5. ✅ **心跳 ping 已完成**（commit `488e094`，06-16，**本卡原标"待实装"是过时文档**）：self-arm 周期 timer（hello 起 arm + fire 重 arm + Disconnected 清）+ 带单调 seq 的 `ping_frame`（Go websocket_router 拒 seq≤0）；`lifecycle_test` 的 `ping_timer_fires_*` 绿。
6. **channel 表建了没人写**：increment 的 channel 元数据只 emit 不落库。
7. **架构 smell（drift-review 登记）**：✅ event_to_upsert_op 热路径(cff383f)；**`channel_event_cursor` 已由 `9ed9d8d` 复活为 IM 权威 cursor 表（对齐 678），非死表**（§1 注脚已更正）；⏳[D1] FFI 句柄类型(接 FFI 冒烟前)；⏳[D3] native·FFI registry 不对称(接 reconnect 前)；2 份 eventbus 稿保留。
8. **host-cli DB 路径 quirk**：`sqlite:{path}?mode=rwc` 把 `?mode=rwc` 当文件名一部分（DB 实际落 `xxx.db?mode=rwc`）——功能正常，路径丑，低优先修。

## 4. 一句话

引擎三件套(HTTP/WS/Storage)齐 + 678 持久化 + sync-v2 正确性层(C1/C2/C3)+B2 **实现完成并入 main**（单测+对抗审+回归全绿）。**代码能「连上·发出去对账·离线补齐(含 phantom/已读/编辑/撤回)·前端经 im:__bus__ 消费」**，且 **数据路径真 Go 端到端已验过**（curl 注入真消息 event_seq=40114 落 message 行，三层 blocker 全清，真机 e2e 8/8 wire-correct）。剩 golden fixtures 工具链(P1) + 功能性事件(schedule/member)。

---

## 5. 待办

### ✅ 今日必办（用户拍板 2026-06-17）—— sync-v2 数据路径真 Go 端到端验证【DONE】

> ✅ **DONE sync-v2 数据路径真 Go 端到端验证 @2026-06-17 ~12:45 | helix dev-zlc | 真消息 rmibkwkj event_seq=40114 落 message 行 + cursor 推进 + post:received emit | 三层 blocker 全清（grpc-deviceId / sync-风暴-并发闸 / WS-post-action-parser）**
>
> **最终验证链（live curl 注入实证）**：curl 发真消息→Go `post sender success`→WS 广播 `post` 帧(event_seq=40114)→helix 收→parser 认 `post`(修)→gate apply(==cursor+1)→PersistFire→**message 表落真实内容行**(temporary_id `helixverify…`↔server id `hjh19t4t…` 对账 / message 内容 / read_bits=100)+ `channel_event_cursor` cursor→40114 + post:received DomainEvent。0 error / 0 sync Network fail。
>
> **三层 blocker 收口**：
> - **L1 grpc（deviceId）**：本机派生 deviceId≠真实设备→paas `IsSuccess=false`；设 `HELIX_DEVICE_ID=ED380414…` 真实设备→`Increment worker completed`，increment 257 channel 全通。
> - **L2 sync 风暴（并发闸，workflow wf_cc5ecd60）**：`increment_channel_end` 每 channel 一条 proactive sync 无并发上限→冷启动 257 并发齐发全 `Network`；`engine_loop.rs` 加 `Arc<Semaphore>`(permit 在 spawned task 内 acquire，泵不阻塞)，N=`HELIX_HTTP_MAX_INFLIGHT`(默认8) 经 EngineConfig；双对抗审 pass→Network 88→0，SyncChannels 0→257。
> - **L3 WS post 真帧（parser，workflow wf_cc9003c9）**：parser 原按猜测 action `"posted"` 识别，真 Go 新消息广播是 `"post"`(Go 常量 L28，≠`posted` L27)→真帧 `Some(_)=>Ignored` 被丢；`classify_frame` 加 `Some("post")=>FrameClass::Post(EventEnvelope)`→`ch.ingest` gate；现网对齐审 pass + 对抗审揪出「self-echo 不 reconcile→15s 后误标 unsend」blocking→Fix 补 `reconcile_post_echo`(CancelTimer)→cargo test 全绿(post_action_inbound 6/6 等 18 bin)。
>
> ✅ **已提交并入 main**（L2 并发闸 + L3 parser post 帧 + outbound 体对齐 `45a621c`/`7270b6b`；`feat/dev-zlc` 已合并删除）。golden fixtures 工具链(P1) 仍未建。
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
- [x] 心跳 ping（缺口 #5）✅ 已完成（commit `488e094`，06-16）：self-arm 周期 timer + 单调 seq `ping_frame`，间隔走 config（默认 8s）。
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
