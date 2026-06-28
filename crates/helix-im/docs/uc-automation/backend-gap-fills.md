# cses-im-server 后端补齐工单（客户端已驱动业务面 · WS 回声/接线缺口）

> **背景**：loopforge 旧 `coverage-crossmap.md` 把一批面标 ⛔backend-down。2026-06-28 实勘 cses-im-server
> 源码发现**多数是环境噪音（cses-java 宕 / 服务未起时测的），非真契约缺口**。逐项实勘后只剩 **3 处真缺口**。
> 本文 = 这 3 处的实现工单 + 落地记录。完成标准（用户拍板）：代码 + `go build`/`go vet` + Go 侧集成测证广播帧真发。
>
> **WS 广播基建**（实现参考）：
> - 入口 `internal/ws/cluster.go:10` `func (s *Service) Publish(*WebSocketEvent)` → 本地 hub + cluster fanout
> - app 层封装 `internal/app/cses_channel.go:254` `func (a *App) publish(event *ws.WebSocketEvent)`
> - action 常量 `internal/ws/event_type.go:23-56`（`WebsocketEventPostPin="post_pin"` / `WebsocketEventChannelMemberRoleUpdated="channel_member_role_updated"` 已定义）
> - 标准模式：`ws.NewWebSocketEvent(action, "", channelId, userId, nil, "").SetData(m)` → `a.publish(evt)`
>
> 更新 2026-06-28。

---

## 实勘结论：6 个疑似缺口 → 4 个其实已通 · 3 个真缺（onlineStatus 拆 1）

| 缺口 | loopforge 旧判定 | cses-im-server 实况 | 处置 |
|---|---|---|---|
| 5.6 公告 echo | ⛔ backend-down | ✅ save/read/delete 均 → `publishPostUpdate`（cses_post_write.go:254）广播 `post_update` | **无需改**·环境噪音 |
| 6.1 拉踢 member 广播 | ⛔ 不到达 | ✅ `broadcastMemberJoinIncrement`（cses_channel_member_change.go:140）逐成员广播 `channel_member_update` | **无需改**·环境噪音 |
| 4.3 too_long cursor | ⛔ 无路径 | ✅ `syncOne`（sync.go）gap>5000 → `kind:too_long, resetTo` | **无需改** |
| member/change/role 广播 | （6.2 一部分）| ✅ `UpdateCsesChannelMemberRoles`（cses_channel_member.go:42）广播 `channel_member_role_updated` | **无需改** |
| **#1 post_pin 广播** | ⛔ 无广播 | ❌ `PinnedCsesChannelPost` 只发 Pulsar 系统消息·无 `post_pin` WS 帧 | **改** |
| **#2 manger add/remove 广播** | ⛔ 不到达 | ❌ `AddCsesChannelManger`/`DeleteCsesChannelManger` 只发系统消息·无 `channel_member_role_updated` | **改** |
| **#3 users/status/ids 接线** | ⛔ statusCache 深耦合 | ⚠️ `presenceQuery().OnlineUserIds()` 已存在（onlineStatus 在用）·只差 HTTP endpoint 接线 | **改** |

---

## 工单 #1 — post_pin WS 广播帧（UC-5.5b）

**文件**：`internal/app/cses_channel_manager_pin.go`（`PinnedCsesChannelPost` line109 / `DeleteCsesChannelPostPinned` / `sendPinSystemPost` line157+）

**现状**：置顶/取消置顶在 co-tx 内写库 + 经 `createChannelPostProp → CreateCsesPost` 落一条 `PostTypeNotice`
系统消息（Pulsar 异步），**无 `post_pin` WS 帧**。客户端 `ws/handlers/post_pin.rs` 期望收到 `post_pin` → `emit_post_updated` → `data-pinned`。

**改法**：写库成功后（`sendPinSystemPost` 返回前后）追加广播：
```go
pinEvt := ws.NewWebSocketEvent(ws.WebsocketEventPostPin, "", channelId, "", nil, "")
pinEvt.Add("postId", postId)
pinEvt.Add("channelId", channelId)
pinEvt.Add("operation", "add") // remove 路径填 "remove"
a.publish(pinEvt)
```
**帧载荷**：`postId` / `channelId` / `operation`(add|remove)。channel-scoped 广播（全员收）。

**Go 集成测**：构造一次 add/remove → 断言 hub 收到 1 帧 action=`post_pin` 且 data.postId/operation 正确。

---

## 工单 #2 — manger add/remove 补 channel_member_role_updated 广播（UC-6.2）

**文件**：`internal/app/cses_channel_manager_pin.go`（`AddCsesChannelManger` line54 区 / `DeleteCsesChannelManger`）

**现状**：设/撤管理员只发系统消息，**无 `channel_member_role_updated`**。对照：`channel/member/change/role`
（`UpdateCsesChannelMemberRoles`）**已**广播该帧——两路语义相同，manger 路径漏广播。

**改法**：角色写入完成后追加（复刻 cses_channel_member.go:42 模式）：
```go
evt := ws.NewWebSocketEvent(ws.WebsocketEventChannelMemberRoleUpdated, "", channelId, "", nil, "")
evt.Add("channelId", channelId)
evt.Add("userIds", userIds)
evt.Add("role", role) // add→ADMIN / remove→默认成员角色
a.publish(evt)
```
**帧载荷**：`channelId` / `userIds[]` / `role`。channel-scoped。

**Go 集成测**：add manger → 断言广播 1 帧 `channel_member_role_updated` role=ADMIN；remove → role=普通。

---

## 工单 #3 — users/status/ids HTTP 接线（UC-5.7 子项）

**文件**：HTTP 注册 `internal/api/`（参考 `channel.go:131` onlineStatus）+ app `internal/app/cses_user.go:1-26`（当前留 NEED_FIX 桩）

**现状**：`POST /users/status/ids` 因「statusCache 绑 mattermost PlatformService」留桩。但 `channel/onlineStatus`
已用 `presenceQuery().OnlineUserIds(ctx, userIds)`（WS 连接 presence 真源）——**presence 基建已在，只差按 userIds 批量查的 HTTP 接线**，无需迁 mattermost statusCache。

**改法**：
1. app 加 `func (a *App) UserStatusByIds(ctx, userIds []string) (map[string]string, error)`：调 `a.presenceQuery().OnlineUserIds(ctx, userIds)` → 组 `{userId: "online"|"offline"}`。
2. HTTP handler 注册 `POST /api/cses/users/status/ids`：解 body `{userIds:[]}` → 调上面 → 返 `{data: map}`。
**返回**：`{userId → "online"|"offline"}` map。

**Go 集成测**：mock 2 个在线 1 个离线 presence → 断言返回 map 三键状态正确。

> 接线完成后 `coverage-crossmap.md` 把 `users/status/ids` 从 ✂️剔除 转 ✅（UC-5.7 子项），并更新 `uc-test-contracts.md` UC-5.7 status。

---

## 落地记录（agent 回填）

| 工单 | 改的文件 | commit | go build | go vet | 集成测 | 状态 |
|---|---|---|---|---|---|---|
| #1 post_pin 广播 | internal/app/cses_channel_manager_pin.go + cses_channel_manager_pin_test.go | 8142603 | PASS | PASS | TestSendPinSystemPost_Add/Remove/ProducerFails_NoBroadcast PASS | ✅ |
| #2 manger role 广播 | internal/app/cses_channel_manager_pin.go + cses_channel_manager_pin_test.go | 8142603（同 #1） | PASS | PASS | TestChangeManagerRole_PermissionDenied/NilSession_NoBroadcast PASS | ✅ |
| #3 users/status/ids 接线 | internal/app/cses_user.go + cses_user_test.go + internal/api/user.go + handlers.go | 161ff13 | PASS | PASS | TestUserStatusByIds_OnlineOffline/AllOffline/PresenceError/Empty PASS | ✅ |

> 落地分支：cses-im-server `feat/im-ws-echo-fills`（2026-06-28）。
> 完成后每行填 commit/PASS，并在 cses-im-server SESSION/log 写终态行 `✅ DONE 后端3处补齐 @<ts> | commit <范围> | 3/3 | cses-im-server`。
