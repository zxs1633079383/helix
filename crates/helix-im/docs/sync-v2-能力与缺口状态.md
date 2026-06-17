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
| **C 正确性 + EventBus** | ✅ 实现（`20334d2`，单测+对抗审绿）| C1 echo 对账（`handle_echo` 激活）✅ · C2 phantom/离线落库/readBits ✅ · C3 mutation 事件(post:read/updated/deleted)+`im:__bus__` 信封 ✅。**残留**：echo 真帧形态待核（按 posted+temporary_id 识别）；功能性事件(schedule/member 等)暂缓 |
| **D 真 Go smoke + golden fixtures** | ⚠️ 半通过 | ✅ 连接/握手/0断连/schema/连接层事件真 Go 验过；❌ **数据路径真连未验**（账号已追平+host-cli 无发送→无消息流）；golden fixtures 未建 |

## 3. 缺口清单（2026-06-17 更新）

1. ✅ **离线历史消息上屏已实现**（`20334d2` C2）：sync `SyncEntry.messages`(Post map)→message 内容落库；phantom(msgId∉map)不落行但 cursor 照推。**真连验待数据流**（见 §5 今日必办）。
2. ✅ **echo 对账已激活**（`20334d2` C1）：`handle_echo` 接进 Inbound，tmp_id→server_id 覆写 message.id+send_status。**echo 真帧形态待核** + 真连验待发送。
3. ⚠️ **im:__bus__ 信封已接线**（`20334d2` C3，`to_bus_envelope`）；emit 补 post:read/updated/deleted；host-cli 无窗口→以 EventSink 计数验，真 app.emit 留 M2 Tauri 接缝。功能性事件(schedule/member/nickname/todo)暂缓。
4. ⚠️ **真 Go 数据路径真连未验**：连接/握手/0断连/schema/连接层事件已验（2026-06-17 :8065 25s）；但 message/channel 0 行、仅 2 DomainEvent——账号已追平+host-cli 无发送→**无消息流，C1/C2/C3 数据路径真连待补**（§5 今日必办）。
5. **心跳 ping 占位帧**：长连稳定待 ping 实装（真 Go 25s 没断是 Go 容忍）。
6. **channel 表建了没人写**：increment 的 channel 元数据只 emit 不落库。
7. **架构 smell（drift-review 登记）**：✅ event_to_upsert_op 热路径(cff383f)/✅ channel_event_cursor 死表(cff383f)；⏳[D1] FFI 句柄类型(接 FFI 冒烟前)；⏳[D3] native·FFI registry 不对称(接 reconnect 前)；2 份 eventbus 稿保留。
8. **host-cli DB 路径 quirk**：`sqlite:{path}?mode=rwc` 把 `?mode=rwc` 当文件名一部分（DB 实际落 `xxx.db?mode=rwc`）——功能正常，路径丑，低优先修。

## 4. 一句话

引擎三件套(HTTP/WS/Storage)齐 + 678 持久化 + sync-v2 正确性层(C1/C2/C3)+B2 **实现完成**（单测+对抗审+回归全绿）。**现在代码能「连上·发出去对账·离线补齐(含 phantom/已读/编辑/撤回)·前端经 im:__bus__ 消费」**——但**数据路径的真 Go 端到端验证待真实消息流**（账号已追平，需制造消息活动）。

---

## 5. 待办

### 🔴 今日必办（用户拍板 2026-06-17）—— sync-v2 数据路径真 Go 端到端验证

> C1/C2/C3/B2 代码已实现 + 单测/对抗审/回归全绿；**唯缺「真 Go 数据路径」验证**（账号已追平、host-cli 无发送 → 25s 冒烟无消息流）。**用户解决 DNS 问题后回来验证。**
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
