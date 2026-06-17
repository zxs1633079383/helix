# Gap 分析 — user-misc 杂项域（csesapi → cses-im-server）

> 对照基准: `docs/csesapi-migration/full-map/partials/backend-http-user-misc.md`（csesapi 15 接口）
> 目标实现: `cses-im-server/internal/{api,app,ws}/`（活体 Read 源码 2026-06-16）
> 已有 gap 输入: `cses-im-server/docs/04-coverage-gap-map.md` + `2026-06-15-cutover-checklist-顶替mattermost.md` + `NEED_FIX_user_status_createuser_loadtarget.md`
> 域: 用户 / 团队 / 分组 / 模块 / 通知 / 搜索 / 健康检查 / 个人守卫
>
> **三态判据**: 已迁移=路由+app+DB 全通且签名/字段/WS 对齐源；部分迁移=已注册但 store/app 为空桩或字段/聚合形态有差异；缺失=路由未注册（无 live handler）。

---

## 0. 三态对照总表

| # | csesapi 接口 | im-server 实现（路由 / app / DB） | 状态 | 差异 / 缺口 |
|---|---|---|---|---|
| 1 | POST /users/list | `api/user.go listUsers` → `app.LoadUsers` → CsesChannel.LoadUserIdsByChannelId + CsesUser.LoadUsers | **已迁移** | 路由 `rt.Users /list` 注册（handlers.go:171）；信封 + data=`entity.Users` 对齐。app/store LoadUsers 覆盖率 0% 仅是缺测，路径活体可达。 |
| 2 | POST /users/status/ids | **未注册** | **缺失** | PlatformService statusCache 依赖 mattermost 深耦合基础设施，故意未迁（handlers.go:169 注释 + `NEED_FIX_user_status_createuser_loadtarget.md` + checklist P1-4）。前端在线状态展示阻塞。 |
| 3 | POST /users（createUser） | **未注册** | **缺失** | 原生 mattermost token/invite/admin/signup 四分支注册全链 + 审计 + guest license + `new_user`/`user_updated` 全局广播，未迁（NEED_FIX + checklist P1-4，待裁决迁移 or 由 ucenter/cses 侧供给）。用户开通流阻塞。 |
| 4 | POST /teams/upsert | `api/team.go upsertTeam` → `app.UpsertTeam` → CreateCsesChannel / ChangeCsesChannelInfo | **已迁移**（含修 bug） | 路由注册（handlers.go:164）；走 `rejectPersonalUser`；**修复了源 double-decode bug**（先 `io.ReadAll` 再二次 Unmarshal，team.go:20）。WS `channel_created`/`channel_member_update`/`update_channel` 均复用 CreateCsesChannel/ChangeCsesChannelInfo。 |
| 5 | POST /teams/member/add | `api/team.go changeTeamMember` → `app.ChannelMemberChange`（与 channel.go 共享） | **已迁移** | 路由注册（handlers.go:165）；走 `rejectPersonalUser`；复用 ChannelMemberChange。WS `channel_member_update`/`post_update`/`post`。 |
| 6 | DELETE /teams/member/quit | `api/team.go quitTeam` → `app.LeaveAllChannelsForTeam` | **已迁移**（含修 bug） | 路由注册（handlers.go:166，`MethodDelete`）；走 `rejectPersonalUser`；**修复了源成功无响应体 bug**（成功补写 success 信封，team.go:64）。WS `quit_company`（publishQuitCompany，cses_team.go:152）+ creator 变更系统 post（CreateChannelCreatorChangePost）。 |
| 7 | POST /groups（getGroups） | **未注册** | **缺失（uncertain）** | 源即空骨架（仅 TODO 注释，从不写响应）。im-server 故意不迁（checklist D 类 uncertain）。非阻塞——前端从未拿到有效响应。 |
| 8 | POST /modules/getAll | `api/modules.go getAllModules` → `app.GetAllModules` → Modules.GetAllModules | **已迁移**（含修笔误） | 路由注册（handlers.go:28，`PathPrefix("/getAll")`）；data=`[]*ent.Modules`；**修复了源 message 笔误**（源硬编码 "channelMemberChange success"，本仓不再误用，handler 直接 SetData 不 SetMessage）。 |
| 9 | POST /notification/loadSend | `api/notification.go loadSendNotifications` → `app.LoadNotificationsByUserId` → Notification.LoadByUserId | **已迁移** | 路由注册（handlers.go:31）；req=`query.NotificationReq`；data=`*entity.NotificationViews`；message "successfully load notifications" 对齐。**需核 offset bug**：源 store LoadByUserId 有 `page.Page*page.Page` offset bug；本仓 store LoadByUserId 实现是否已修待二次确认（见下方注 A）。 |
| 10 | POST /notification/loadTarget | **未注册** | **缺失（uncertain）** | 源 store LoadSendByUserId 全注释返回 nil；命名混淆（loadReceive vs LoadSend）。im-server 故意不迁（notification.go:12 注释 + checklist P1-5，待裁决迁移 or 废弃）。 |
| 11 | POST /search/post | `api/search.go postSearch` → `app.SearchPost` → **空桩** | **部分迁移** | 路由注册（handlers.go:88）；handler+信封完整，但 `app.SearchPost` 恒返回 `{posts:[]}` 空容器（ES/CsesPost store 未迁，search.go:22）。**返回 data 形态偏离源**：源 data=`*entity.Posts`（裸数组），本仓 data=`map[string]any{posts:[]}`（套 key，见下方注 B）。 |
| 12 | POST /search/user | `api/search.go userSearch` → `app.SearchUser` → **空桩** | **部分迁移** | 路由注册（handlers.go:89）；`app.SearchUser` 恒返回 `{users:[]}`。源 store 即 nil 桩，本仓沿用空桩语义但加 nil-guard。**data 形态偏离**：源 data=`*entity.UserEntities`，本仓 data=`map[string]any{users:[]}`。 |
| 13 | POST /search/channel | `api/search.go channelSearch` → `app.SearchChannel` → **空桩** | **部分迁移** | 路由注册（handlers.go:90）；保留 userId 非空 gate（"user no login"）+ 注入 userId。`app.SearchChannel` 恒返回 `{channels:[]}`（修源 nil 解引用 panic）。**data 形态偏离**：源 data=`*model.ChannelList`，本仓 data=`map[string]any{channels:[]}`。 |
| 14 | POST /search/do（doSearch） | `api/search.go doSearch` → `app.DoSearch` → 分派空桩 | **部分迁移** | 路由注册（handlers.go:91）；注入 userId；**修源 `*param.SearchType` nil 解引用 panic**（nil 回落 SearchTypeAll，search.go:44）。各分支均空桩。**聚合 key 偏离**：源 searchAll 返回 `posts/channels/users/channelDetails` 四 key，本仓 `searchAll` 只返 `posts/channels/users` 三 key（**缺 `channelDetails`**，search.go:61-67；且嵌套结构与源不同，见注 B）。 |
| 15 | GET /health | `api/router.go healthCheck` → 裸 `{"status":"OK"}` | **已迁移** | 路由注册（router.go:84，Health 子路由不挂 Auth/TrackId）；不走信封，对齐源。K8s liveness/readiness 对等。 |
| — | personal_guard（rejectPersonalUser） | `api/team.go rejectPersonalUser` → `middleware.RejectPersonalUser` | **已迁移** | 唯一写 403 的出口；被 team #4/#5/#6 调用。需 verbatim 核 magic string `PERSONAL_USER_FORBIDDEN`（见注 C）。 |

---

## 1. WS 事件齐备性核对（cses-im-server/internal/ws/event_type.go）

本域 WS 全部来自下游复用方法（无自有 channel_event 驱动）。逐个核对：

| WS 事件 | 源触发端点 | im-server 定义 | 状态 |
|---|---|---|---|
| `new_user` | createUser(#3) | `WebsocketEventNewUser`（event_type.go:65） | 常量已定义，但**触发它的 createUser 路由未迁** → 实际不会广播 |
| `user_updated` | createUser(#3) | `WebsocketEventUserUpdated`（event_type.go:64） | 同上，常量在但无触发路径 |
| `channel_created` | upsertTeam(#4) | `WebsocketEventChannelCreated`（event_type.go:30） | **齐备**（CreateCsesChannel 触发） |
| `channel_member_update` | upsertTeam/changeTeamMember(#4/#5) | `WebsocketEventChannelMemberUpdate`（event_type.go:32） | **齐备** |
| `update_channel` | upsertTeam 改信息(#4) | `WebsocketEventUpdateChannel`（event_type.go:47） | **齐备** |
| `post_update` | changeTeamMember(#5) | `WebsocketEventPostUpdate`（event_type.go:44） | **齐备**（复用 channel member change 链） |
| `post` | changeTeamMember/quitTeam(#5/#6) | `WebsocketEventPost`（event_type.go:35） | **齐备** |
| `quit_company` | quitTeam(#6) | `WebsocketEventQuitCompany`（event_type.go:46） | **齐备**（publishQuitCompany，cses_team.go:152，NewWebSocketEventForUsers 多播） |

> 结论：本域 WS 常量 **8/8 全定义**。可触发性 6/8（`new_user`/`user_updated` 因 createUser 未迁而无触发路径，属 #3 缺失的连带影响，非 WS 层独立缺口）。

---

## 2. 缺失 / 偏差能力优先级清单（对替代 mattermost 的阻塞程度）

### P0 — 阻塞替代（前端核心流不可用）
（本域无 P0：用户列表/团队/通知/模块/健康检查均已迁移；缺失项均非首屏强依赖或有外部供给路径）

### P1 — 高阻塞（功能缺位，需裁决迁移 or 外部供给）
1. **POST /users（createUser，#3）** — 用户开通全流程。原生 mattermost 注册深耦合（token/invite/admin/signup 四分支 + 审计 + guest license + new_user/user_updated 广播）。`NEED_FIX_user_status_createuser_loadtarget.md` + checklist P1-4：先裁决迁移 or 由 ucenter/cses 侧供给。**未迁则无法在 im-server 单仓完成开户。**
2. **POST /users/status/ids（#2）** — 批量在线状态查询。依赖 PlatformService statusCache。前端在线状态/红点展示阻塞。同 NEED_FIX，checklist P1-4。

### P2 — 中阻塞（已注册但空桩，接 store/ES 才生效；且 data 形态偏离源需对齐）
3. **search 四件套（#11–#14）** — 路由+信封已通，但 `app.Search*` 全空桩（ES/CsesPost/CsesUser/CsesChannel store 未迁）。**两类缺口**：
   - (a) **功能空桩**：恒返回空结果。接 ES + store 后才有真实搜索。
   - (b) **data 形态偏离源（契约风险，须对齐）**：源 postSearch=`*entity.Posts`、userSearch=`*entity.UserEntities`、channelSearch=`*model.ChannelList`（裸数组/列表）；本仓全统一成 `map[string]any{key:[]}` 套 key 形态。**且 doSearch 的 searchAll 缺源的 `channelDetails` 第四 key。** 客户端若 key off 源形态会解析失败——迁移真实实现时须 verbatim 对齐源 data 结构（含 channelDetails）。

### P3 — 低阻塞 / 可不修（uncertain，待业务裁决）
4. **POST /notification/loadTarget（#10）** — 源 store 全注释返回 nil，命名混淆。checklist P1-5：裁决迁移 or 废弃。非阻塞（源本就返回空）。
5. **POST /groups（getGroups，#7）** — 源空骨架从未实现。drop 即可（checklist D 类 uncertain）。非阻塞。

---

## 3. 待二次确认注记（活体定真因，勿照搬源 bug）

- **注 A（offset bug）**: 源 #9 loadSend store 内 `page.Page*page.Page`（应 `*page.Size`）。本仓 `Notification().LoadByUserId(ctx, userId, page)` 是否已修需 Read `internal/store/sqlstore/notification_store.go LoadByUserId` 实体确认（本次未深入该 store 函数体）。若未修则属隐性行为缺陷。
- **注 B（search data 形态）**: 本仓 search 全返 `map[string]any`，与源裸 `*entity.Posts`/`*entity.UserEntities`/`*model.ChannelList` 不一致。属**已注册但契约偏离**——接真实实现时是对齐窗口，须以源 data 类型为准（前端 key off 源）。doSearch 的 `searchAll` 缺 `channelDetails` key 是明确偏差。
- **注 C（personal guard magic string）**: 前端 key off `PERSONAL_USER_FORBIDDEN`。本仓走 `middleware.RejectPersonalUser`，须确认该 magic string verbatim 保留（本次未 Read middleware 实体，建议核 `internal/api/middleware` RejectPersonalUser 写出的 message）。

---

## 4. 汇总计数

- **已迁移**（含修 bug）: 7 — #1 users/list · #4 teams/upsert · #5 member/add · #6 member/quit · #8 modules/getAll · #9 notification/loadSend · #15 health（+ personal_guard 守卫）
- **部分迁移**（已注册但空桩 / data 形态偏离）: 4 — #11 search/post · #12 search/user · #13 search/channel · #14 search/do
- **缺失**（未注册）: 4 — #2 users/status/ids · #3 POST /users · #7 groups · #10 notification/loadTarget

> 注：#7 groups / #10 loadTarget 是 uncertain（源本就空），缺失但非阻塞；真正阻塞替代的缺失是 #2 / #3（用户状态 + 开户）。
