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
4. **真 Go 直连冒烟已通过**（2026-06-16 :8065 22s：握手/increment/0 断连/入站达泵）；但 **UC-1 端到端**（发→上屏→echo→正式 id）仍未验，依赖 C 阶段。
5. **心跳 ping 占位帧**：间隔与 hello.pingInterval(60s) 未对齐（真 Go 22s 没断是 Go 侧容忍；长连稳定待 ping 实装）。
6. **channel 表建了没人写**：increment 的 channel 元数据(mention/urgent/lastEventSeq) 只 emit 不落库。
7. **架构 smell（drift-review 登记，部分已修）**：✅[medium] `event_to_upsert_op` 热路径重解析（cff383f 字段提取上移 parser 已修）；✅[low] channel_event_cursor 死表（cff383f 已删）；⏳[medium·D1] FFI 句柄类型不一致（接 FFI 冒烟前修）；⏳[medium·D3] native·FFI registry 不对称（接 WS reconnect 前修）；driver-host→helix-im 反向 dev-dep（test-only）；2 份 eventbus 设计稿**已拍板保留**。
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
  - ✅ [medium] HX-C005 热路径回归已先修（cff383f：PostFields 提取上移 parser，event_to_upsert_op 零再解析）。

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
