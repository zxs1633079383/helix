# Backend WS 推送事件全图（csesapi → app 层 → Hub Publish）

> 用途：csesapi RESTful handler 不直接 `Publish`，而是经 **app 层方法**（`cses_post.go` / `cses_channel.go` / `cses_team.go` / `post_core.go` / `cses_agent.go` / `gateway_bot.go`）构造 `model.WebSocketEvent` 后 `a.Publish(...)` / `a.PublishSlice(...)`。本文追踪 csesapi 业务链路实际推送的全部 WS 事件类型，给 cses-im-server `internal/ws` 移植做 wire 契约 + 触发时机 + 广播范围参照。
>
> 活体源校正基线（mattermost feat/im-sync-v2-backend，2026-06-16）：
> - 事件常量定义：`server/public/model/websocket_message.go:21-144`（**不是** `cons/fields.go` —— `cons/fields.go` 只有 Pulsar/ES/Redis 等基础设施常量，**无任何 WS 事件常量**，原任务假设有误，此处校正）。
> - 推送站点：`server/channels/app/{cses_post.go,cses_channel.go,cses_team.go,post_core.go,cses_agent.go,gateway_bot.go}`。
> - 构造 API：`model.NewWebSocketEvent(action, teamId, channelId, userId, omitUsers, omitConnectionId)` + `model.NewWebSocketEventForUsers(action, userIds)`。
> - 发布 API：`a.Publish(ev)`（单条）/ `a.PublishSlice(msgs, userId, channelId)`（批量，末尾自动追发 `increment_channel_end`）。
>
> 对 `docs/03-ws-port-spec.md` 附录「17 个 cses 用到的 WebsocketEventType 常量」的**校正/补全**：实际 csesapi 业务链路推送 **19** 个事件类型。03-spec 漏列了 `posts_update`（`SendEntPostMessage` 撤回/批量路径）和把 `post`（首发推送，`SendPostMessage`）与 `post_update` 混淆。`add_manager`/`remove_manager` 确认源码已注释，不实现（与 03-spec 一致）。

---

## 1. WS 事件分类总表（csesapi 业务链路实际推送）

| # | 事件名 (action) | 常量 | 主要触发链路 | 广播范围 | 关联 BroadcastHook |
|---|---|---|---|---|---|
| 1 | `post` | `WebsocketEventPost` | 发消息首推（`SendPostMessage`）/ 系统提示（权限变更/bot/agent） | ChannelId（频道成员）或 UserId（单用户系统提示） | 无（cses 路径不挂 hook） |
| 2 | `posts_update` | `WebsocketEventPostsUpdate` | 撤回/批量 post 字段更新（`SendEntPostMessage`） | ChannelId | 无 |
| 3 | `post_update` | `WebsocketEventPostUpdate` | 编辑/加急/快捷回复/已读触发的单 post 字段更新 | ChannelId 或 UserIds | 无 |
| 4 | `post_read` | `WebsocketEventPostRead` | 已读单条 post（`publishRead`） | UserIds（发送者+已读者，≤2 人） | 无 |
| 5 | `post_pin` | `WebsocketEventPostPin` | 置顶/取消置顶消息 | ChannelId | 无 |
| 6 | `post_schedule_created` | `WebsocketEventPostScheduleCreated` | 创建定时消息 | UserId（发送者本人） | 无 |
| 7 | `post_schedule_canceled` | `WebsocketEventPostScheduleCanceled` | 取消定时消息 | UserId（发送者本人） | 无 |
| 8 | `increment_channel` | `WebsocketEventIncrementChannel` | 增量同步会话列表（首屏/重连，`PublishSlice` 批量） | UserId（请求者本人，逐 channel 一条） | 无 |
| 9 | `increment_channel_end` | `WebsocketEventIncrementChannelEnd` | 增量同步结束哨兵（`PublishSlice` 末尾自动追 / 空集时显式发） | UserId（请求者本人） | 无 |
| 10 | `channel_created` | `WebsocketEventChannelCreated` | 创建群聊 / 入群成功 | UserId（逐成员一条，含创建人回流） | 无 |
| 11 | `channel_member_update` | `WebsocketEventChannelMemberUpdate` | 加入已有群（增量 channel 推送给单成员） | UserId（逐成员一条） | 无 |
| 12 | `channel_member_role_updated` | `WebsocketEventChannelMemberRoleUpdated` | 更新群成员角色 | ChannelId | 无 |
| 13 | `channel_close` | `WebsocketEventChannelClose` | 解散/关闭群聊 | ChannelId | 无 |
| 14 | `update_channel` | `WebsocketEventUpdateChannel` | 群信息变更（置顶/名称/公告/通知设置/lastPost 刷新/未读 badge） | ChannelId 或 UserId/UserIds | 无 |
| 15 | `update_channel_notice` | `WebsocketEventUpdateChannelNotice` | 群公告变更 | ChannelId | 无 |
| 16 | `update_channel_member_nickName` | `WebsocketEventUpdateChannelMemberNickName` | 修改群内昵称 | ChannelId | 无 |
| 17 | `change_channel_approval` | `WebsocketEventChangeChannelApprovalStatus` | 开/关发言审批 | ChannelId | 无 |
| 18 | `quit_company` | `WebsocketEventQuitCompany` | 用户退出公司（离开该 team 所有群） | UserIds（同 team 受影响用户） | 无 |
| 19 | `post`（agent/bot） | `WebsocketEventPost` | bot/agent 回复消息推送（`cses_agent.go` / `gateway_bot.go`） | ChannelId | 无 |

> **BroadcastHook 说明**：mattermost 仅 2 个 hook（`add_mentions` / `add_followers`），且**只在原生 `posted` 事件路径**（`notification.go:692-698`）挂载。**所有 cses 业务路径（cses_*.go）从不调 `AddHook`**，因此本全图 19 个事件**全部 hook=无**。移植时 hook 仍需随 Hub 注册（原生路径备用），但 cses push 不依赖。证据见 `docs/03-ws-port-spec.md` §5.4 注。

> **envelope wire 约定**（所有事件统一，`websocket_message.go`）：顶层 `{v, action, data, broadcast, seq}`；`data` 内业务字段命名遵 cses 约定（`createAt` 非 createdAt / `eventType` 数字枚举 / `event_seq` 顶层注入）。`broadcast` 字段：`omitUsers`(map) / `userId` / `userIds`([]) / `channelId` / `teamId` / `connectionId` / `omit_connectionId`。

---

## 2. 事件详情节

### 2.1 `post`（WebsocketEventPost）— 发消息首推 + 系统提示

**构造**（多站点）：
- `post_core.go:417` `SendPostMessage`（主路径，发消息首推）：
  ```go
  msg := model.NewWebSocketEvent(model.WebsocketEventPost, "", post.ChannelId, "", nil, "")
  msg = msg.SetData(post.ToMap())
  if seq, ok := post.Props[PostEventSeqPropKey].(int64); ok && seq > 0 {
      msg.Add("event_seq", seq)   // 顶层注入 channel_event.event_seq，客户端 pts gate 消费
  }
  a.Publish(msg)
  ```
- `cses_channel.go:874` `model.NewWebSocketEvent(WebsocketEventPost, "", "", userId, nil, "").SetData(post.ToMap())`（单用户系统提示）
- `cses_channel.go:1089` `publishChangePermissionMessage`（权限变更系统消息，`SetData({"message": content})` + `Add("channelId", ...)`）
- `cses_agent.go:406` / `gateway_bot.go:175`（bot/agent 回复，见 §2.19）

**payload (`data`)**：`post.ToMap()`（entity.Post 序列化，camelCase；含 `id`/`channelId`/`userId`/`message`/`createAt`/`props`/`quickReply`/`mentions`/`replyRootId` 等）+ 顶层 `event_seq`(int64)。
**触发时机**：`POST /api/cses/posts/create`（发消息）；权限变更/bot 回复等系统流程。
**广播范围**：ChannelId（频道全成员）；系统提示变体走 UserId（单用户）。
**集群**：经 `a.Publish` → `cluster.go` 本地 hub + 跨 pod 扇出（ChannelId 广播）。

---

### 2.2 `posts_update`（WebsocketEventPostsUpdate）— 批量 post 字段更新

**构造**（`post_core.go:545` `SendEntPostMessage`）：
```go
msg := model.NewWebSocketEvent(model.WebsocketEventPostsUpdate, "", posts[0].ChannelId, "", nil, "")
msg.Add("posts", posts)                 // data.posts = []*ent.Post 数组
if maxSeq > 0 { msg.Add("event_seq", maxSeq) }  // 取同批 max(event_seq) 注顶层
a.Publish(msg)
```
**payload (`data`)**：`posts`([]ent.Post) + 顶层 `event_seq`(int64, 同批 max)。
**触发时机**：撤回消息（`RevokePost` 末尾 `SendEntPostMessage(posts)`，含被撤 post + reply 上下游）；批量 post 属性更新。
**广播范围**：ChannelId（`posts[0].ChannelId`）。
**集群**：跨 pod ChannelId 扇出。

---

### 2.3 `post_update`（WebsocketEventPostUpdate）— 单 post 字段更新

**构造**（多站点 `cses_post.go`）：
- `cses_post.go:149` `createChannelPostUrgent`：`NewWebSocketEventForUsers(WebsocketEventPostUpdate, *userIds).SetData(post.ToMap()).PrecomputeJSON()`（加急，UserIds 定向）
- `cses_post.go:1574/1747/1794/1820`：`NewWebSocketEvent(WebsocketEventPostUpdate, "", post.ChannelId, "", nil, "").SetData(post.ToMap())`（编辑/快捷回复等，ChannelId 广播）
- `cses_channel.go:1018`：`NewWebSocketEvent(WebsocketEventPostUpdate, "", "", value.Id, nil, "").SetData(post.ToMap())`（单用户）

**payload (`data`)**：`post.ToMap()`；编辑路径顶层带 `event_seq`（来自 `post.Props[PostEventSeqPropKey]`，A3/A4 pts gate 消费）。
**触发时机**：编辑消息（`UpdatePost`）/ 加急消息 / 快捷回复 reaction。
**广播范围**：ChannelId（广播）或 UserIds（加急定向受众）。
**集群**：随分支 ChannelId / UserIds 扇出。

---

### 2.4 `post_read`（WebsocketEventPostRead）— 已读单条

**构造**（`cses_post.go:716` `publishRead`）：
```go
var userIDs []string
if post.UserId == userId { userIDs = []string{userId} }
else { userIDs = []string{post.UserId, userId} }      // 发送者 + 已读者
msg := model.NewWebSocketEventForUsers(model.WebsocketEventPostRead, userIDs)
msg = msg.SetData(post.ToReadPushPost().ToMap())
a.Publish(msg)
```
**payload (`data`)**：`post.ToReadPushPost().ToMap()`（精简已读视图：postId/channelId/已读位等）。
**触发时机**：`POST /api/cses/posts/read`（已读回执，`ReadPost`）。
**广播范围**：UserIds（发送者 + 已读者，最多 2 人；自读时仅 1 人）。
**集群**：跨 pod UserIds 扇出。
**配套**：同 `ReadPost` 还可能发 `update_channel`（`cses_post.go:700`，刷会话未读，UserIds=[已读者]）。

---

### 2.5 `post_pin`（WebsocketEventPostPin）— 置顶/取消置顶

**构造**（`cses_channel.go:1840`）：`NewWebSocketEvent(WebsocketEventPostPin, "", (*posts)[0].ChannelId, "", nil, "")`。
**payload**：pin/unpin 的 post 信息（`Add`/`SetData`）。
**触发时机**：`POST /api/cses` 置顶/取消置顶路由。
**广播范围**：ChannelId。

---

### 2.6 `post_schedule_created`（WebsocketEventPostScheduleCreated）— 创建定时消息

**构造**（`cses_post.go:571` `CreateCsesScheduledMessage`）：
```go
event := model.NewWebSocketEvent(model.WebsocketEventPostScheduleCreated, "", "", post.UserId, nil, "")
event.Add("channelId", post.ChannelId)
a.Publish(event)
```
**payload (`data`)**：`channelId`(string)。
**触发时机**：创建定时消息（Pulsar DeliverAt 延时投递）。
**广播范围**：UserId（发送者本人）。

---

### 2.7 `post_schedule_canceled`（WebsocketEventPostScheduleCanceled）— 取消定时消息

**构造**（`cses_post.go:593` `CancelCsesScheduledMessage`）：`NewWebSocketEvent(WebsocketEventPostScheduleCanceled, "", "", userId, nil, "")` + `Add("channelId", ...)`。
**payload (`data`)**：`channelId`(string)。
**触发时机**：取消定时消息（删 Redis key + 改 schedule 状态）。
**广播范围**：UserId（发送者本人）。

---

### 2.8 `increment_channel`（WebsocketEventIncrementChannel）— 增量会话列表

**构造**（`cses_channel.go:224` `RunIncrementWorker`）：
```go
msgs := make([]*model.WebSocketEvent, 0, len(incrementChannels))
for _, inc := range incrementChannels {
    e := model.NewWebSocketEventForUsers(model.WebsocketEventIncrementChannel, []string{session.UserId})
    msgs = append(msgs, e.SetData(inc.ToMap()))
}
go a.PublishSlice(msgs, session.UserId, channelId)
```
**payload (`data`)**：`inc.ToMap()`（entity.IncrementChannel，含 `id`/`name`/`lastPost`(JSON string)/`unReadCount`/`lastEventSeq`/`needSync`/成员/admin/team 等）。
**触发时机**：增量同步（首屏 / WS 重连 / online lag），请求 `POST /api/cses` 增量接口后**异步**经 `PublishSlice` 推送。`fillLastEventSeq` 填 `LastEventSeq`（客户端按此调 `POST /api/v4/channels/sync` 拉 channel_event 增量）。
**广播范围**：UserId（请求者本人，逐 channel 一条）。
**集群**：`PublishSlice` 内部逐条 `PublishWithWaitGroup`，全部完成后追发 `increment_channel_end`。

---

### 2.9 `increment_channel_end`（WebsocketEventIncrementChannelEnd）— 增量结束哨兵

**构造**：
- `web_hub.go:55/57` `PublishSlice` 内核自动追发（`NewWebSocketEvent(WebsocketEventIncrementChannelEnd, "", "", userId, nil, "")`，channelId 非空时 `SetData({channelId})`）。
- `cses_channel.go:100/123` 空集时显式发（无 channelIds / 增量为空也必须发，否则前端永远 loading）。

**payload (`data`)**：可空；带 `channelId`(string，当 `PublishSlice` 传了 channelId 时)。
**触发时机**：增量同步批量推送结束。
**广播范围**：UserId（请求者本人）。
**移植要点**：`PublishSlice(messages, userId, channelId)` 必须在 wg 全部完成后追发本事件（`docs/03-ws-port-spec.md` §9.6）。

---

### 2.10 `channel_created`（WebsocketEventChannelCreated）— 创建群/入群

**构造**（`cses_channel.go:715`）：
```go
for key, value := range incrementChannels {     // key = 成员 userId
    a.fillSeqAndUnread(rootCtx, &value)
    message_ := model.NewWebSocketEvent(model.WebsocketEventChannelCreated, "", channel.ID, key, nil, "")
    a.Publish(message_.SetData(value.ToMap()).SetCsesTrackId(csesTrackId).PrecomputeJSON())
}
```
**payload (`data`)**：`value.ToMap()`（entity.IncrementChannel 单 channel 视图）+ envelope `tracing` 带 `csesTrackId`（`SetCsesTrackId`）。
**触发时机**：`POST /api/cses` 创建群聊（`CreateCsesChannel`）。
**广播范围**：UserId（逐成员一条，含创建人回流 + 被邀请者）。
**集群**：逐成员 UserId 扇出；`PrecomputeJSON` encode-once。

---

### 2.11 `channel_member_update`（WebsocketEventChannelMemberUpdate）— 加入已有群

**构造**（`cses_channel.go:655` / `974`）：
```go
for key, value := range incrementChannels {     // key = 成员 userId
    a.fillSeqAndUnread(rootCtx, &value)
    message_ := model.NewWebSocketEvent(model.WebsocketEventChannelMemberUpdate, "", "", key, nil, "")
    a.Publish(message_.SetData(value.ToMap()).PrecomputeJSON())
}
```
**payload (`data`)**：`value.ToMap()`（entity.IncrementChannel 单 channel 视图，含 phantom UnReadCount + LastEventSeq）。
**触发时机**：加入已存在的群聊（增量 channel 推给新成员）。
**广播范围**：UserId（逐成员一条）。

---

### 2.12 `channel_member_role_updated`（WebsocketEventChannelMemberRoleUpdated）— 更新成员角色

**构造**（`cses_channel.go:1448`）：
```go
event := model.NewWebSocketEvent(model.WebsocketEventChannelMemberRoleUpdated, "", cmd.ChannelId, "", nil, "")
event.Add("channelId", cmd.ChannelId)
event.Add("userIds", cmd.UserIds)
event.Add("role", cmd.Role)
a.Publish(event)
```
**payload (`data`)**：`channelId`(string) / `userIds`([]string) / `role`(角色枚举)。
**触发时机**：更新群成员角色（`UpdateCsesChannelMemberRoles`）。
**广播范围**：ChannelId（频道全成员）。

---

### 2.13 `channel_close`（WebsocketEventChannelClose）— 解散群

**构造**（`cses_channel.go:2211`）：
```go
closeMsg := entity.CloseMsg{ChannelID: channelId, DeleteAt: milli}
msg := model.NewWebSocketEvent(model.WebsocketEventChannelClose, "", channelId, "", nil, "").SetData(closeMsg.ToMap())
a.Publish(msg)
```
**payload (`data`)**：`closeMsg.ToMap()` = `channelId`(string) + `deleteAt`(int64)。
**触发时机**：群主解散/关闭群（`ChannelClose`，需 isChannelMaster）。
**广播范围**：ChannelId。

---

### 2.14 `update_channel`（WebsocketEventUpdateChannel）— 群信息变更（高频复用）

**构造**（多站点，最高频复用事件）：
- `cses_channel.go:1220/1245/1270/2024`：ChannelId 广播（名称/orient/置顶等通用群属性变更）。
- `cses_channel.go:1356` `UpdateCsesChannelMemberNotify`：`NewWebSocketEvent(..., "", "", userId, ...).SetData(channelNotifyVo.ToMap())`（通知设置，UserId 单用户）。
- `cses_channel.go:1685`：`NewWebSocketEvent(..., "", "", result.UserId, ...).SetData(topMap)`（置顶，UserId）。
- `cses_post.go:113/141` `pushUpdateChannelWithUrgent`/`SendPostMention`：`NewWebSocketEvent(..., "", "", memberId, ...).SetData(incrementData.ToMap()).PrecomputeJSON()`（未读 badge/提及，逐 member UserId）。
- `cses_post.go:458` 撤回刷 lastPost：`SetData({"id": channelId, "lastPost": lastPostStr})`（**lastPost 必须 JSON string** 非 map，否则客户端反序列化报 `field lastPost: invalid type: map, expected a string`，2026-05-29 实测踩坑）。
- `cses_post.go:700` 已读刷会话：`NewWebSocketEventForUsers(..., [readerUserId]).SetData(updateResultPost)`。

**payload (`data`)**：随站点不同——通用群属性 / `channelNotifyVo.ToMap()` / `incrementData.ToMap()`（含 `lastEventSeq` + phantom `unReadCount`）/ `{id, lastPost(JSON string)}`。
**触发时机**：群名称/公告/置顶/通知设置变更、未读 badge 刷新、提及/加急 increment、撤回刷 lastPost、已读刷会话未读。
**广播范围**：ChannelId（群属性）或 UserId/UserIds（按用户定制的 increment/badge/通知）。
**移植要点**：`lastPost` 字段在 `update_channel` 与 `increment_channel` 路径必须一致序列化为 **JSON 字符串**（客户端 channel VO `last_post: String`）。

---

### 2.15 `update_channel_notice`（WebsocketEventUpdateChannelNotice）— 群公告

**构造**（`cses_channel.go:1190` `publishChangeChannelNotice`）：`NewWebSocketEvent(WebsocketEventUpdateChannelNotice, "", channelNotice.ChannelId, "", nil, "")`。
**payload (`data`)**：channelNotice 内容（`Add`/`SetData`，含 `channelId` + `content`）。
**触发时机**：修改群公告（`ChangeChannelNotice`）。
**广播范围**：ChannelId。

---

### 2.16 `update_channel_member_nickName`（WebsocketEventUpdateChannelMemberNickName）— 群内昵称

**构造**（`cses_channel.go:1330`）：
```go
event := model.NewWebSocketEvent(model.WebsocketEventUpdateChannelMemberNickName, "", channelId, "", nil, "")
event.Add("channelId", channelId)
event.Add("userId", userId)
event.Add("nickName", nickValue)   // nil→空串（清空）
a.Publish(event)
```
**payload (`data`)**：`channelId`(string) / `userId`(string) / `nickName`(string，空串=清空)。
**触发时机**：修改群内昵称。
**广播范围**：ChannelId。

---

### 2.17 `change_channel_approval`（WebsocketEventChangeChannelApprovalStatus）— 发言审批

**构造**（`cses_channel.go:1073` `publishChangeApprovalStatusMessage`）：
```go
message := model.NewWebSocketEvent(model.WebsocketEventChangeChannelApprovalStatus, "", channelId, "", nil, "")
message.Add("channelId", channelId)
message.Add("approvalStatus", true/false)
message.Add("message", "当前群聊已开启/关闭发言审批")
a.Publish(message)
```
**payload (`data`)**：`channelId`(string) / `approvalStatus`(bool) / `message`(string)。
**触发时机**：开/关群发言审批。
**广播范围**：ChannelId。
**注**：action 字符串值是 `change_channel_approval`（常量名 `WebsocketEventChangeChannelApprovalStatus` 末尾带 Status，但 wire 值无 status 后缀）。

---

### 2.18 `quit_company`（WebsocketEventQuitCompany）— 退出公司

**构造**（`cses_team.go:120` `leaveAllChannelsForTeam`）：
```go
message := model.NewWebSocketEventForUsers(model.WebsocketEventQuitCompany, idsByTeamId)
message.SetData(map[string]interface{}{"channels": channelMembers})
a.Publish(message)
```
**payload (`data`)**：`channels`(受影响 channelMember 列表)。
**触发时机**：用户退出公司 → 离开该 team 所有群（同事务内推送，配套发 CREATOR 变更 post）。
**广播范围**：UserIds（`idsByTeamId` = 同 team 受影响用户）。

---

### 2.19 `post`（agent/bot 回复变体）

**构造**：
- `cses_agent.go:406`：`NewWebSocketEvent(WebsocketEventPost, "", post.ChannelId, "", nil, "")`（agent 回复）。
- `gateway_bot.go:175`：同上（gateway bot 回复）。

**payload (`data`)**：bot/agent 生成的 post（`SetData(post.ToMap())`）。
**触发时机**：bot/agent webhook 回调生成消息后推送。
**广播范围**：ChannelId。
**关系**：与 §2.1 同 action（`post`），仅触发源是 bot/agent 链路，wire 一致。

---

## 3. 移植校正清单（对 03-ws-port-spec.md）

1. **事件总数**：03-spec 附录列 17 个 → 实际 csesapi 业务链路 **19** 个 action（去重后 16 个不同 action 字符串，因 `post` 有首发/系统/bot 三个触发变体共用，`update_channel` 多站点复用）。**新增确认**：`posts_update`（撤回/批量）必须移植，03-spec 漏列。
2. **`post` vs `post_update` vs `posts_update`**：三者不同 action，移植常量裁剪清单需全带：
   - `post`（首发推送 `SendPostMessage`）
   - `post_update`（单 post 字段更新，编辑/加急）
   - `posts_update`（批量 post 更新，撤回路径）
3. **`event_seq` 顶层注入**：`post`/`posts_update`/`post_update`（编辑路径）三事件在 `data` **顶层**注入 `event_seq`(int64)，源自 `post.Props[PostEventSeqPropKey]`，客户端 pts gate 直接读 `data.event_seq`（不解 `data.props.channel_event_seq`）。移植 `internal/ws` + cses 推送 app 层必须保留此顶层字段。
4. **`lastPost` 必须 JSON string**：`update_channel` / `increment_channel` 的 `lastPost` 字段是序列化后的 JSON 字符串（非嵌套对象），客户端 `channel VO last_post: String`。撤回路径 2026-05-29 踩坑（map 反序列化失败 → 会话 lastPost 不刷）。
5. **BroadcastHook 全无**：19 个 cses 事件**无一**挂 `add_mentions`/`add_followers`（hook 仅原生 `posted` 路径）。移植 hook 随 Hub 注册即可，cses push 路径不调 `AddHook`。
6. **`SetCsesTrackId`**：`channel_created` 经 `SetCsesTrackId` 在 envelope `tracing` 注入 trackId（in-channel 成员共享同一份）。移植 event builder 需保留 `SetCsesTrackId` + envelope tracing 字段。
7. **`PrecomputeJSON` 用法**：逐用户扇出（`channel_created`/`channel_member_update`/加急/提及）调 `PrecomputeJSON()` 做 encode-once。移植保留 encode-once 不变量（03-spec §7 H2）。
8. **`AddManager`/`RemoveManager`**：源码已注释（`add_manager`/`remove_manager`），不实现（与 03-spec 一致）。

---

## 4. 广播范围速查（移植 `cluster.go` PublishSkipClusterSend 4 分支映射）

| 广播形态 | 用到的事件 | Broadcast 字段 |
|---|---|---|
| ChannelId 频道全成员 | post / posts_update / post_update(编辑) / post_pin / channel_member_role_updated / channel_close / update_channel(群属性) / update_channel_notice / update_channel_member_nickName / change_channel_approval / post(bot) | `ChannelId` 非空 → `ShouldSendEvent` 查成员表 |
| UserId 单用户 | post(系统提示) / post_schedule_* / increment_channel / increment_channel_end / channel_created / channel_member_update / update_channel(通知/badge/置顶) | `UserId` 非空 → 该用户全连接 |
| UserIds 多用户定向 | post_read(≤2) / post_update(加急) / update_channel(已读刷会话) / quit_company(同 team) | `UserIds` 非空 → 逐用户 hub |
| viewers 受限（密聊） | （cses 密聊消息经 `data["viewers"]` + `ResolveBroadcastViewerUserIds` 预过滤，叠加在 ChannelId 广播上） | `data.viewers` → restricted 仅 viewers 各自 hub |

> 移植 `internal/ws/cluster.go` 的 `PublishSkipClusterSend` 4 分支（ConnectionId / UserId / UserIds+viewers / 兜底全 hub）需覆盖以上全部形态，viewers 双路径走 `ResolveBroadcastViewerUserIds` 单一真源（03-spec §3.3 / §9.5）。
