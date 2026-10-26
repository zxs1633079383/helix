# csesapi 后端 HTTP 接口清点 — user-misc 杂项域

> 域: 用户 / 团队 / 分组 / 模块 / 通知 / 搜索 / 健康检查 / 个人守卫
> 涉及文件: `server/channels/csesapi/{user,team,cses_group,modules,notification,search,personal_guard,health}.go`
> 路由 base prefix: `/api/cses`（`model.APIURLSuffixCSES`）
> 数据来源: 活体 Read 源码（2026-06-16）。字段定义 verbatim 摘自 entity / ent 源码。
> 横切契约（响应信封 `dto.CommonRes{status,message,data}` / 鉴权入站 / Cses-Track-Id 透传）见 `docs/csesapi-migration/01-接口可达性清单.md` §2，本文不重复，仅标注**偏离信封**的端点。

---

## 0. 域接口清单表

| # | METHOD + PATH | handler | app 方法 | DB 表 / 数据源 | WS 事件 | k/d | risk |
|---|---|---|---|---|---|---|---|
| 1 | POST /api/cses/users/list | listUsers | LoadUsers | channel_member(LoadUserIdsByChannelId) · user(LoadUsers) | — | keep | low |
| 2 | POST /api/cses/users/status/ids | getUserStatusesByIds | GetUserStatusesByIds | 无 DB(PlatformService statusCache) | — | keep | medium |
| 3 | POST /api/cses/users | createUser | GetTokenById · CreateUserWithToken/InviteId/AsAdmin/FromSignup | Token · Users · Teams · Channels · ChannelMembers · Preferences(原生) | new_user · user_updated | keep | high |
| 4 | POST /api/cses/teams/upsert | upsertTeam | UpsertTeam → CreateCsesChannel / ChangeCsesChannelInfo | channel · channel_member · channel_member_history | channel_created · channel_member_update · update_channel | keep | high |
| 5 | POST /api/cses/teams/member/add | changeTeamMember | ChannelMemberChange | channel · channel_member · channel_member_history · channel_topic · post + gRPC | channel_member_update · post_update · post | keep | high |
| 6 | DELETE /api/cses/teams/member/quit | quitTeam | LeaveAllChannelsForTeam | channel_member · channel_member_snapshot · channel_member_history · post | quit_company · post(creator 变更系统消息) | keep | high |
| 7 | POST /api/cses/groups | getGroups | —（空骨架） | — | — | **uncertain** | low |
| 8 | POST /api/cses/modules/getAll | getAllModules | GetAllModules | modules | — | keep | low |
| 9 | POST /api/cses/notification/loadSend | loadSendNotifications | LoadNotificationsByUserId | notification(LoadByUserId) | — | keep | medium |
| 10 | POST /api/cses/notification/loadTarget | loadReceiveNotifications | LoadSendNotificationsByUserId | notification(LoadSendByUserId — 实现体注释/桩) | — | **uncertain** | high |
| 11 | POST /api/cses/search/post | postSearch | SearchPost | post(CsesPost.SearchPost) | — | keep | medium |
| 12 | POST /api/cses/search/user | userSearch | SearchUser | CsesUser.Search（**桩 nil,nil**） | — | keep | high |
| 13 | POST /api/cses/search/channel | channelSearch | SearchChannel | CsesChannel.Search（**桩**） | — | keep | high |
| 14 | POST /api/cses/search/do | doSearch | DoSearch → searchAll/post/user/channel/postInChannel | post(真) + user/channel 桩 + channel(SearchChannelsByIds) | — | keep | high |
| 15 | GET /api/cses/health | healthCheck | —（裸 ReturnStatusOK） | — | — | keep | low |

> personal_guard.go 不暴露独立路由：`rejectPersonalUser(c,w)` 是被 team.go 三条路由（#4/#5/#6）开头调用的守卫函数。

**WS 事件去重汇总（供 WS 追踪 agent 交叉核对）**：`new_user` · `user_updated` · `channel_created` · `channel_member_update` · `update_channel` · `post_update` · `post` · `quit_company`。
> 说明：本域**无自有 channel_event 驱动事件**；WS 全部来自下游复用方法（createUser 走原生 SendNotifications 链；upsertTeam/changeTeamMember 复用 channel.go 的 CreateCsesChannel/ChannelMemberChange；quitTeam 走 LeaveAllChannelsForTeam 显式 Publish）。事件常量定义均在 `public/model/websocket_message.go`，**非** `cons/fields.go`。

---

## 1. POST /api/cses/users/list — 列出某 channel 成员用户

- **method+path**: `POST /api/cses/users/list`
- **handler**: `listUsers`（user.go:20）

请求结构（handler 内联匿名 struct，camelCase）:
```go
var req = struct {
    ChannelId string `json:"channelId"`
}{}
```

响应结构（`dto.CommonRes` 信封，data = `entity.Users`）:
```go
type Users ent.Users   // entity/user.go:19；ent.Users = []*ent.User
```
裸 ent.User 列表（含 id / username 等原生用户字段）。

- **app 方法**: `LoadUsers(channelId string) (entity.Users, error)`（cses_user.go:7）
  - 链路: `CsesChannel().LoadUserIdsByChannelId(channelId)` → `CsesUser().LoadUsers(userIds)`
- **DB 表**: `channel_member`（查 userIds） + `user`（按 ids 查用户）
- **触发 WS**: 无
- **业务说明**: 给定 channelId 返回成员用户列表。无权限校验。

---

## 2. POST /api/cses/users/status/ids — 批量查用户在线状态

- **method+path**: `POST /api/cses/users/status/ids`
- **handler**: `getUserStatusesByIds`（user.go:41）

请求结构（内联匿名 struct）:
```go
var req = struct {
    UserIds []string `json:"userIds"`
}{}
```

响应结构: `dto.CommonRes` 信封，data = `[]*model.Status`。**特例**: `len(UserIds)==0` 时走 `WriteJSON(w, nil)` 直接写裸 `null`（不套信封）。
```go
// public/model/status.go（verbatim 字段 tag）
UserId         string `json:"user_id"`
Status         string `json:"status"`
Manual         bool   `json:"manual"`
LastActivityAt int64  `json:"last_activity_at"`
ActiveChannel  string `json:"active_channel,omitempty" db:"-"`
DNDEndTime     int64  `json:"dnd_end_time"`
PrevStatus     string `json:"-"`
```
> 注意: model.Status 用 **snake_case**（原生 mattermost 约定），与 cses 自定义 camelCase 不同。迁移须保留 snake_case。

- **app 方法**: `GetUserStatusesByIds(userIDs []string) ([]*model.Status, *model.AppError)`（status.go:19）→ `a.Srv().Platform().GetUserStatusesByIds(userIDs)`
- **DB 表**: 无（PlatformService 内存 statusCache，缓存 miss 回落原生 Status store）
- **触发 WS**: 无
- **业务说明**: 注释里原生的 `len(userId)!=26` 校验已被注释掉。无权限校验。

---

## 3. POST /api/cses/users — 创建用户（不走信封 / 深耦合原生）

- **method+path**: `POST /api/cses/users`
- **handler**: `createUser`（user.go:70）

请求结构: `model.User`（原生 mattermost 用户对象，整体 decode）+ URL query 参数:
```
t   = tokenId    （token 邀请）
iid = inviteId   （邀请链接 id）
r   = redirect
```
入站前 `user.SanitizeInput(c.IsSystemAdmin())`。

响应结构: **不走信封**。成功 → `w.WriteHeader(http.StatusCreated)`(201) + `json.NewEncoder(w).Encode(ruser)`（裸 `*model.User`）。失败走 `c.Err`（原生 AppError → 非 200 状态码）。

- **app 方法**（4 分支按 query 决定）:
  - `tokenId != ""`: `GetTokenById(tokenId)` → 校验 guest license/enabled → `CreateUserWithToken(c.AppContext, &user, token)`
  - `inviteId != ""`: `CreateUserWithInviteId(c.AppContext, &user, inviteId, redirect)`
  - `c.IsSystemAdmin()`: `CreateUserAsAdmin(c.AppContext, &user, redirect)`
  - else: `CreateUserFromSignup(c.AppContext, &user, redirect)`
- **DB 表**: Token · Users · Teams · Channels · ChannelMembers · Preferences（原生 mattermost 全套用户创建落库 + 审计 audit.AddEventParameter）
- **触发 WS**: `new_user`（WebsocketEventNewUser = "new_user"）· `user_updated`（WebsocketEventUserUpdated = "user_updated"）—— 由原生 CreateUser* 链内部全局广播触发。
- **业务说明**: 这是 csesapi 里唯一深度复用原生 mattermost 注册全流程（审计 + license 校验 + Plugin Hook + 全局广播）的端点。最高迁移风险：需完整携带原生 token/invite/signup 四分支 + 审计 + guest license 逻辑。`audit.Fail` 默认 + `auditRec.Success()` 收尾。

---

## 4. POST /api/cses/teams/upsert — 维护公司大群（走 rejectPersonalUser）

- **method+path**: `POST /api/cses/teams/upsert`（team.go:13，用 `PathPrefix("/upsert")`）
- **handler**: `upsertTeam`（team.go:59）；**开头 `rejectPersonalUser(c, w)`**

请求结构: `entity.CreateChannelSpecifyOwner`（camelCase）:
```go
// entity/create_channel.go:20
type CreateChannelSpecifyOwner struct {
    *ent.Channel                                          // 嵌入原生 channel 全字段
    Users       []*model.ChannelMemberMinimal `json:"users"`
    ForceCreate *bool                         `json:"forceCreate"`
    Owner       *model.ChannelMemberMinimal   `json:"owner"`
}
```
Validate: channel 非空 + users 非空 + (type=="D" → 必须 2 人)。

响应结构: `dto.CommonRes` 信封，成功 `SetMessage("upsertTeam success")`，data = null。

- **app 方法**: `UpsertTeam(session *model.Session, body io.Reader) *model.AppError`（cses_team.go:17）
  - `createChannelParam.ID == ""` → `CreateCsesChannel(session, param, nil, true, "")`（建公司大群）
  - 否则 → 二次 `json.Decode(body)` 成 `command.ChangeChannelInfo` → `ChangeCsesChannelInfo(...)`
- **DB 表**: channel · channel_member · channel_member_history（建群路径）；channels（改信息路径）
- **触发 WS**: 建群路径触发 CreateCsesChannel 的 `channel_created`(WebsocketEventChannelCreated) + `channel_member_update`(WebsocketEventChannelMemberUpdate)；改信息路径触发 `update_channel`。
- **业务说明**: 注释「不维护 team 表，只维护公司大群」。⚠️ **已知 bug**: body(io.Reader) 只能读一次——`ID != ""` 分支第二次 `json.Decode(body)` 解到空（io.Reader 已耗尽）。迁移须修（缓存 body bytes 后两次解码）。

---

## 5. POST /api/cses/teams/member/add — 团队成员变更（走 rejectPersonalUser）

- **method+path**: `POST /api/cses/teams/member/add`（team.go:14，`PathPrefix("/member/add")`）
- **handler**: `changeTeamMember`（team.go:38）；**开头 `rejectPersonalUser(c, w)`**

请求结构: `entity.ChannelMemberEvent`（camelCase）:
```go
// entity/channel.go:632
type ChannelMemberEvent struct {
    ChannelId  string                       `json:"channelId"`
    JoinUsers  []model.ChannelMemberMinimal `json:"joinUsers"`
    LeaveUsers []model.ChannelMemberMinimal `json:"leaveUsers"`
}
```

响应结构: `dto.CommonRes` 信封，成功 `SetMessage("channelMemberChange success")`，data = null。

- **app 方法**: `ChannelMemberChange(r.Context(), param, c.AppContext.Session())`（与 channel.go 的 `POST /channel/member/change` 共享同一 app 方法）
- **DB 表**: channel · channel_member · channel_member_history · channel_topic · post + gRPC（回填用户名）
- **触发 WS**: `channel_member_update` · `post_update` · `post`（join/leave 系统消息走 Pulsar）
- **业务说明**: team 域只是 channel.go ChannelMemberChange 的另一入口（加 personal guard）。迁移时与 channel.go member/change 合并复用，不重复实现。

---

## 6. DELETE /api/cses/teams/member/quit — 退出团队所有群（走 rejectPersonalUser）

- **method+path**: `DELETE /api/cses/teams/member/quit`（team.go:15，`PathPrefix("/member/quit")`）
- **handler**: `quitTeam`（team.go:18）；**开头 `rejectPersonalUser(c, w)`**

请求结构: `entity.QuitTeamReq`（camelCase）:
```go
// entity/quit_team.go:3
type QuitTeamReq struct {
    UserId string `json:"userId"`
    TeamId string `json:"teamId"`
}
```

响应结构: ⚠️ **成功路径漏写响应体**（handler 成功分支无 `WriteJSON`，客户端收到空 body）。失败 → `dto.CommonRes` 信封 `SetMessage("quitTeam error")`。

- **app 方法**: `LeaveAllChannelsForTeam(session *model.Session, userID, teamID string) error`（cses_team.go:58）
  - 单 ent.Tx 内: GetUserTeamChannelMembers → BatchLeaveChannelMembers → SaveMultiple 快照 → GetNewCreators → BatchUpdateCreatorById → 发 WS → 发 creator 变更 post → SaveChannelMemberHistoryLeaveByUser
- **DB 表**: channel_member · channel_member_snapshot · channel_member_history · post（creator 变更系统消息）
- **触发 WS**:
  - `quit_company`（`model.WebsocketEventQuitCompany = "quit_company"`）：`NewWebSocketEventForUsers` 多播给 team 内受影响 userIds，payload `{"channels": []ChannelMemberMinimal}`
  - 若有新 creator → `CreateChannelCreatorChangePost` 触发 creator 变更系统消息（post 类事件）
- **业务说明**: 退出某用户在某 team 下所有 channel，并对原 creator 群补选新 creator。高风险（多步 tx + 快照 + creator 转移 + 系统消息）。迁移须修成功无响应体 bug。

---

## 7. POST /api/cses/groups — 分组（空骨架，uncertain）

- **method+path**: `POST /api/cses/groups`（cses_group.go:9，注释写 `// GET /api/v4/groups` 但实际 Methods("POST")）
- **handler**: `getGroups`（cses_group.go:12）

```go
func getGroups(c *Context, w http.ResponseWriter, r *http.Request) {
    // TODO: implement group search and listing
}
```

- **请求/响应**: 无（函数体只有 TODO 注释，**不写任何响应**）
- **app 方法 / DB 表 / WS**: 无
- **业务说明**: 纯占位骨架，从未实现。迁移决策: **drop 或业务确认后补实现**（uncertain）。

---

## 8. POST /api/cses/modules/getAll — 获取全部功能模块

- **method+path**: `POST /api/cses/modules/getAll`（modules.go:11，`PathPrefix("/getAll")`）
- **handler**: `getAllModules`（modules.go:14）

请求结构: 无 body（handler 不解析 body）。

响应结构: `dto.CommonRes` 信封，data = `[]*ent.Modules`:
```go
// channels/store/sqlstore/ent/modules.go:15（裸 ent，camelCase）
type Modules struct {
    config `json:"-"`
    ID    string `json:"id,omitempty"`     // 模块主键 ID
    Name  string `json:"name,omitempty"`   // 模块唯一名称
    Label string `json:"label,omitempty"`  // 模块标签
    URL   string `json:"url,omitempty"`    // 模块头像
    // selectValues sql.SelectValues（非序列化）
}
```

- **app 方法**: `GetAllModules(session *model.Session) ([]*ent.Modules, error)`（cses_modules.go:11）→ 单 ent.Tx 内 `Modules().GetAllModules(tx, ctx)`
- **DB 表**: `modules`
- **触发 WS**: 无
- **业务说明**: ⚠️ **已知笔误** — 成功响应 message 硬编码 `"channelMemberChange success"`（应为 `"getAllModules success"`）。无权限过滤（任何会话拿全部模块）。迁移须修 message。

---

## 9. POST /api/cses/notification/loadSend — 加载用户通知（发送侧）

- **method+path**: `POST /api/cses/notification/loadSend`（notification.go:13）
- **handler**: `loadSendNotifications`（notification.go:17）

请求结构: `query.NotificationReq`（camelCase）:
```go
// entity/query/NotificationReq.go:3
type NotificationReq struct {
    Page                       // 嵌入
    State string `json:"state"`
}
// entity/query/page.go:3
type Page struct {
    Page int `json:"page" query:"page" validate:"required,min=1"`
    Size int `json:"size" query:"size" validate:"required,min=1,max=100"`
}
```

响应结构: `dto.CommonRes` 信封，成功 `SetMessage("successfully load notifications")`，data = `*entity.NotificationViews`:
```go
// entity/notification.go:81（camelCase）
type NotificationView struct {
    ChannelID   string                  `json:"channelId,omitempty"`
    ChannelName *string                 `json:"channelName,omitempty"`
    Message     map[string]interface{}  `json:"message,omitempty"`
    ExtraProps  *ExtraProps             `json:"extraProps,omitempty"`
    FromID      *string                 `json:"fromId,omitempty"`
    FromName    *string                 `json:"fromName,omitempty"`
    Id          string                  `json:"id,omitempty"`
    IsBroadcast *bool                   `json:"isBroadcast,omitempty"`
    RelationId  *string                 `json:"relationId,omitempty"`
    TargetUser  []TargetUser            `json:"targetUser,omitempty"`
    Type        *types.NotificationType `json:"type,omitempty"`
}
type NotificationViews []*NotificationView
```

- **app 方法**: `LoadNotificationsByUserId(page *query.NotificationReq, userId string) (*entity.NotificationViews, error)`（cses_notifications.go:8）→ `Notification().LoadByUserId(&userId, page)` → `ConvertEntityNotificationsToNotificationViews`
- **DB 表**: `notification`（含 recipient 关联）
- **触发 WS**: 无（纯读）
- **业务说明**: ⚠️ **已知 offset bug**（01-清单 §3.9 记录）：store LoadByUserId 内 offset 计算 `page.Page*page.Page`（应为 `page.Page*page.Size`）。迁移须修。userId 取自 session。

---

## 10. POST /api/cses/notification/loadTarget — 加载用户通知（接收侧，uncertain）

- **method+path**: `POST /api/cses/notification/loadTarget`（notification.go:14）
- **handler**: `loadReceiveNotifications`（notification.go:40）

请求结构: `query.Page`（**注意 handler 默认初始化 `{Page:0, Size:10}`**，与 #9 用 NotificationReq 不同）:
```go
var req = query.Page{ Page: 0, Size: 10 }
```

响应结构: `dto.CommonRes` 信封，data = `*entity.NotificationViews`（同 #9，但**实际返回 nil/空**）。

- **app 方法**: `LoadSendNotificationsByUserId(page *query.Page, userId string) (*entity.NotificationViews, error)`（cses_notifications.go:17）→ `Notification().LoadSendByUserId(&userId, page)`
- **DB 表**: `notification`（但 **store 实现体全注释，返回 nil** — 01-清单 §3.9 记录）
- **触发 WS**: 无
- **业务说明**: ⚠️ 命名混淆（handler 叫 loadReceive，app 叫 LoadSend）+ store 未实现。**uncertain**：迁移前业务确认补实现或废弃。

---

## 11. POST /api/cses/search/post — 搜索消息

- **method+path**: `POST /api/cses/search/post`（search.go:13）
- **handler**: `postSearch`（search.go:38）

请求结构: `search.DoSearchRequest`（camelCase，verbatim）:
```go
// entity/search/post_search_param.go:25
type DoSearchRequest struct {
    Keyword     *string            `json:"keyword"`     // message 关键词
    SearchType  *Type              `json:"searchType"`  // 0:所有 1:消息 2:用户 3:频道 4:含消息的频道
    PostType    *types.PostType    `json:"postType"`
    ChannelType *types.ChannelType `json:"channelType"`
    ChannelId   string             `json:"channelId"`
    TeamId      *string            `json:"teamId"`
    IsAll       bool               `json:"isAll"`       // 默认 false，否则每个 10 条
    UserId      *string            `json:"-"`           // 服务端注入，不接收客户端
}
// type Type int; iota: All=0 Post=1 User=2 Channel=3 PostInChannel=4
```

响应结构: `dto.CommonRes` 信封，data = `*entity.Posts`（nil 时回填 `&entity.Posts{}`）。

- **app 方法**: `SearchPost(param search.DoSearchRequest) (*entity.Posts, error)`（cses_search.go:80）→ `searchPost` → `CsesPost().SearchPost(postSearchRequest)`（masterEnt.Post.Query）
- **DB 表**: `post`
- **触发 WS**: 无
- **业务说明**: 唯一**真实实现**的搜索分支。postSearch 不注入 UserId（与 channelSearch/doSearch 不同）。

---

## 12. POST /api/cses/search/user — 搜索用户（store 桩）

- **method+path**: `POST /api/cses/search/user`（search.go:14）
- **handler**: `userSearch`（search.go:60）

请求结构: `search.DoSearchRequest`（同 #11）。

响应结构: `dto.CommonRes` 信封，data = `*entity.UserEntities`:
```go
// entity/user.go:5
type UserEntity struct {
    Id       string       `json:"id,omitempty"`       // User Id
    UserName string       `json:"username,omitempty"`
    Teams    []string     `json:"teams,omitempty"`
    Friends  UserEntities `json:"friends,omitempty"`
}
type UserEntities []*UserEntity
```

- **app 方法**: `SearchUser(param) (*entity.UserEntities, error)`（cses_search.go:105）→ `searchUser` → `CsesUser().Search(userSearchRequest)`
- **DB 表**: `user`（理论上）— ⚠️ **store CsesUser.Search 返回 `nil, nil` 桩**（01-清单 §3.8）
- **触发 WS**: 无
- **业务说明**: store 未实现。迁移前必须补 store 实现 + nil guard。

---

## 13. POST /api/cses/search/channel — 搜索频道（store 桩）

- **method+path**: `POST /api/cses/search/channel`（search.go:15）
- **handler**: `channelSearch`（search.go:78）

请求结构: `search.DoSearchRequest`（同 #11）。handler 在解码后**校验 session.UserId 非空**（空 → `SetMessage("user no login")`），然后 `request.UserId = &session.UserId` 注入。

响应结构: `dto.CommonRes` 信封，data = `*model.ChannelList`（原生 mattermost 类型，`[]*model.Channel`）。

- **app 方法**: `SearchChannel(param) (*model.ChannelList, error)`（cses_search.go:128）→ `searchChannel`（注意内部 `*request.UserId` 解引用）→ `CsesChannel().Search(channelSearchRequest)`
- **DB 表**: `channels`（理论上）— ⚠️ **store CsesChannel.Search 桩**（01-清单 §3.8）；nil channels → `return nil, nil`
- **触发 WS**: 无
- **业务说明**: ⚠️ searchChannel 内 `*request.UserId` 在 UserId 为 nil 时 panic。handler 层有 UserId 非空 gate 缓解，但 doSearch 路径（#14）经 searchAll 调用 SearchChannel 时也注入了 UserId。迁移须补 store + nil guard。

---

## 14. POST /api/cses/search/do — 聚合搜索（按 SearchType 分发）

- **method+path**: `POST /api/cses/search/do`（search.go:16）
- **handler**: `doSearch`（search.go:19）

请求结构: `search.DoSearchRequest`（同 #11）。handler 解码后 `param.UserId = &session.UserId` 注入。

响应结构: `dto.CommonRes` 信封，data = `*map[string]interface{}`（结构随 SearchType 变）:
- key 枚举（cses_search.go:11）: `"posts"` / `"channels"` / `"users"` / `"channelDetails"`

- **app 方法**: `DoSearch(param) (*map[string]interface{}, error)`（cses_search.go:27）→ 按 `*param.SearchType` 查 `searchMap` 分发:
  - `SearchTypeAll(0)` → `searchAll`（channelDetails + channels + users 三合一）
  - `SearchTypePost(1)` → `searchPost`（真）
  - `SearchTypeUser(2)` → `searchUser`（桩）
  - `SearchTypeChannel(3)` → `searchChannel`（桩）
  - `SearchTypePostInChannel(4)` → `searchPostInChannel`（post 真 + SearchChannelsByIds 真）
- **DB 表**: `post`（真）· `user`（桩）· `channels`（桩 + SearchChannelsByIds 真）
- **触发 WS**: 无
- **业务说明**: 聚合入口。searchAll 串行调 SearchChannel/SearchUser（含桩 nil 风险）。⚠️ `*param.SearchType` 为 nil 时 `searchMap[*param.SearchType]` 解引用 panic（无 SearchType 校验）。迁移须补 nil guard + 补 user/channel store 实现。

---

## 15. GET /api/cses/health — 健康检查（不走信封）

- **method+path**: `GET /api/cses/health`（health.go:6）
- **handler**: `healthCheck`（health.go:9）

请求结构: 无。

响应结构: **不走信封**。`ReturnStatusOK(w)` → 裸 `{"status":"OK"}`（原生 mattermost helper）。

- **app 方法**: 无
- **DB 表**: 无
- **触发 WS**: 无
- **业务说明**: 框架层 `/api/cses/health` 还跳过 duration 日志（01-清单 §2.3）。健康探针端点。迁移直接对等 K8s liveness/readiness。

---

## 附: personal_guard.go（无独立路由，守卫函数）

- **常量**: `const PersonalUserForbiddenMessage = "PERSONAL_USER_FORBIDDEN"`（personal_guard.go:13）
- **函数**: `rejectPersonalUser(c *Context, w http.ResponseWriter) bool`（personal_guard.go:18）
  - `app.IsPersonalUser(c.AppContext.Session())`（= `session != nil && session.CompanyId == ""`）命中 → `w.WriteHeader(http.StatusForbidden)`(403) + `dto.CommonRes{status:"failed", message:"PERSONAL_USER_FORBIDDEN"}` → 返回 true（调用方 MUST `return`）
- **调用方**: team.go 三条路由全部（#4 upsertTeam / #5 changeTeamMember / #6 quitTeam）开头调用。
- **迁移注意**: 这是 csesapi 里**唯一**写非 200（403）HTTP 状态码的路径（其余信封端点恒 200）。前端 key off 该 magic string，迁移须 verbatim 保留 `"PERSONAL_USER_FORBIDDEN"`。

---

## 迁移高风险 / 待修 bug 速查（本域）

1. **createUser（#3）**: 深耦合原生 mattermost 注册全链（token/invite/admin/signup 四分支 + 审计 + guest license + Plugin Hook + new_user/user_updated 全局广播 + 201 裸响应）。最高迁移成本。
2. **search 桩（#12/#13/#14）**: CsesUser.Search / CsesChannel.Search 返回 nil 桩；channelSearch / doSearch 在 nil UserId 上有 panic 路径。迁移前必须补 store 实现 + nil guard。
3. **upsertTeam（#4）双 decode bug**: io.Reader 只读一次，ID!="" 分支二次 decode 解到空。
4. **quitTeam（#6）成功无响应体**: 成功路径漏 WriteJSON。
5. **loadSend（#9）offset bug**: store 内 `page.Page*page.Page` 应为 `page.Page*page.Size`。
6. **loadTarget（#10）store 全注释**: 命名混淆 + 返回 nil。uncertain，业务确认补/废。
7. **modules getAll（#8）message 笔误**: 硬编码 "channelMemberChange success"。
8. **getGroups（#7）空骨架**: 从未实现，uncertain。
9. **大小写约定**: 本域绝大多数 cses 自定义结构 camelCase；唯 `model.Status`（#2）、`model.User`（#3）、`model.ChannelList`（#13）等原生类型用 **snake_case**，迁移须按类型来源区分，不可一刀切。
