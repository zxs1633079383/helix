# helix-im · sync-v2 能力与缺口状态（落地真相）

> 快照日期：2026-06-16（commit 区间 8c69f25..97a33a8）。
> 真源核对：cses-client `features/im/`（行为）+ cses-im-server `internal/app/sync.go`（协议）+ 678 真库 schema。
> 设计全文：[`docs/过会方案/helix-sync-v2-real-go-wire-设计.md`](../../../docs/过会方案/helix-sync-v2-real-go-wire-设计.md)。
> 本卡只记「现在到哪了 + 缺什么 + 明天干什么」，不重复设计。

---

## 1. 三大能力（引擎层 ✅ / 业务落库 ⚠️ 部分）

| 能力 | 状态 | 实情 |
|---|---|---|
| HTTP | ✅ | NativeHttp(reqwest) + 全局身份头 + increment 触发 + sync/notify |
| WS | ✅ | NativeTransport(tungstenite) + connect + 入站→Tick(A2) + hello 握手(A4) + 重连 |
| Storage | ✅ 引擎 / ⚠️ 业务 | rusqlite WAL + 读写分离 + 678 对齐 schema 真建表(message/channel/channel_member/channel_event_cursor) + 多租户 Application Support 路径 + **发送路径**落 message + 真 DB 集成测试 |

> ⚠️ **cursor 落库纠正（drift-review wf_ee61a139-68b 揪出）**：运行时 cursor 实际写 **`_helix_monotonic`**（通用单调表），**不是** `channel_event_cursor`——后者是 schema.rs 建的**死表占位**（零读零写，M2 前不用）。见 ledger 待收敛 todo [low]。

## 2. sync-v2 落地表（约 50%）

| Phase | 状态 | 缺什么 |
|---|---|---|
| A 基础贯通（hello/身份头/transport/入站） | ✅ | — |
| B increment 真 wire | ✅ 主体（B3 sync/notify 真 wire 本会话）| B2 `increment_end` 冷启动自愈 未做 |
| **C 正确性 + EventBus** | ❌ 大头未做 | C1 echo→reconcile 死代码(`handle_echo #[allow(dead_code)]`)；C2 phantom/readBits 未做；C3 emit 只 **6/24** 事件、`im:__bus__` 信封未接（driver 只到 broadcast）|
| **D 真 Go smoke + golden fixtures** | ❌ | 从未真连 Go 冒烟；fixtures 工具链未建 |

## 3. 8 个易被忽略的真相（缺口清单）

1. **离线历史消息当前不会上屏**：B5 OUT —— sync `events` 只推 cursor，`SyncEntry.messages`(Post 快照) 不落 message 表；只有自己发的消息进 message。**持久化打通最大剩余缺口。**
2. **echo 对账死代码**：`handle_echo`(module.rs:801 `#[allow(dead_code)]`) 没接 Inbound → temporary_id→正式 id 不覆写。
3. **im:__bus__ 信封没接通**：前端「零改动跑通」前提未满足；emit 仅 6/24 事件。
4. **真 Go 从未冒烟**：绿测试全是单测/本地 WS mock，UC-1 端到端没验。
5. **心跳 ping 占位帧**：间隔与 hello.pingInterval(60s) 未对齐 → 握手后 ~8s 断连疑似此因（真连 Go 潜在 blocker）。
6. **channel 表建了没人写**：increment 的 channel 元数据(mention/urgent/lastEventSeq) 只 emit 不落库。
7. **架构 smell（drift-review 登记）**：[medium] `event_to_upsert_op` 热路径 serde 重解析 + per-event alloc（违 HX-C005，6d83ad5 引入）；[low] channel_event_cursor 死表；driver-host→helix-im 反向 dev-dep；2 份 workflow 越权设计稿待复核（ad1c932/3c5324f）。
8. **持久化只到「建表+发送+cursor」**，≠「IM 数据全链路落库」。

## 4. 一句话

引擎三件套(HTTP/WS/Storage)齐 + 持久化骨架按 678 对齐建起来了；但 sync-v2 正确性层(C：echo 对账/phantom/全量 EventBus) 和真 Go 联调(D) 没做，离线历史上屏(B5)也没做。**现在能「连上、发出去、推游标、建对表」，还不能「完整离线对账 + 前端零改动消费」。**

---

## 5. 明天代办（优先级排序）

> 排序原则：先解锁「真 Go 端到端可验」（UC-1 前置），再补离线完整性，最后清架构债。

### P0 — 解锁 UC-1 真 Go 冒烟（D 的前置）
- [ ] **心跳 ping 对齐**（缺口 #5）：ping 帧格式 + 间隔对齐 hello.pingInterval(60s)，修 ~8s 断连 → 真连 Go 第一道坎。
- [ ] **echo→reconcile 激活**（C1，缺口 #2）：`handle_echo` 接进 `Tick::Inbound` 分支，解析 echo 帧 temporary_id→server_id 覆写 message.id。
- [ ] **B5 phantom + 离线消息上屏**（C2 + 缺口 #1）：解析 `SyncEntry.messages`(Post map) → 落 message 内容；phantom(msgId 不在 map)cursor 推进但不渲染。
  - **并轨修 [medium] HX-C005 热路径回归**（ledger todo）：字段提取上移 parser，EventEnvelope 携 owned typed 视图，消除 event_to_upsert_op 重解析 + reserialize（同源改造）。

### P1 — EventBus 全量 + 离线完整性
- [ ] **C3 emit 补全 6→24 事件 + `im:__bus__` 信封接线**（缺口 #3）：NativeEventSink→app.emit 信封，对齐现网前端总线契约。
- [ ] **B2 increment_end 冷启动自愈**。
- [ ] **B3 readBits**（type=6 已读位）+ store:* 对齐（Tauri 接缝预备）。

### P2 — 架构债收敛（drift-review 登记）
- [ ] **[low] 删 channel_event_cursor 死表**（或立「_helix_monotonic 唯一真源」不变量 + grep gate）。
- [ ] **复核 2 份 workflow 越权设计稿**（ad1c932 eventbus 契约 / 3c5324f IM 下游域卡）：保留/修订/revert。
- [ ] **driver-host→helix-im 反向 dev-dep**：评估是否换成更轻的真 DB 测试方式。

### P3 — D 收官
- [ ] **golden fixtures 工具链**（im_trace 导出 → Tick 剧本，确定性 id/时间注入）。
- [ ] **UC-1 端到端真 Go smoke**（起 Go 服务端 + 真 cookieId，发消息→上屏→echo→正式 id；离线>500→续拉至 no_change，cursor==maxSeq）。
