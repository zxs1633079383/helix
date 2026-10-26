# Gap 分析：posts 域 — cses-im-server 是否已复刻 csesapi

> 对照基准：`docs/csesapi-migration/full-map/partials/backend-http-posts.md`（csesapi 35 接口）
> 目标实现：`cses-im-server/internal/{api,app,ws}/`（活体源码，2026-06-16）
> 已有 gap 分析交叉：`cses-im-server/docs/04-coverage-gap-map.md` / `05-app-gap.md`
>
> **结论速览**：35 个 csesapi 接口里 **34 个已迁移**（路由 + app + DB + WS 全链路就位），**1 个有意不迁**（`createMock` 压测旁路），**0 个真正缺失**。
> 阻塞替代 mattermost 的核心问题 **不在「接口缺失」，而在「WS 事件命名重构」**：csesapi 源把 revoke/quickReply/urgent/confirm 统一标 `post_update`，im-server 改为细分事件（`posts_update` / `post_urgent` / `post_confirm_urgent`）。这是**有意的协议升级**（im-server WS event_type.go L25-26 注释明确），但**必须与客户端 wire 契约逐一核对**，否则前端按旧 `post_update` 监听会漏收。详见末节「WS 事件差异专章」。

---

## 三态对照表（35 接口）

> 状态图例：✅ 已迁移（全链路就位） / 🟡 部分迁移（签名/字段/WS 有差异） / ❌ 缺失

| # | csesapi 接口 | im-server 实现（路由 + app 方法） | 状态 | 差异 / 缺口 |
|---|---|---|---|---|
| 1 | POST `/posts/create` | `posts.go:createPost` → `App.CreateCsesPost` (cses_post_create.go) | ✅ | 走 Pulsar SenderTopic 异步落库，consumer 侧落库 + WS。请求体裸 `*entity.Post` 一致 |
| 2 | POST `/posts/createPosts` | `post_write.go:createPosts` → `App.CreateCsesPosts` | 🟡 | **wire key 差异**：csesapi 源请求体**无 json tag**（PascalCase `Posts`/`ChannelIds`）；im-server 显式 `json:"posts"` / `json:"channelIds"`（camelCase）。若前端按源发 PascalCase，im-server decode 不到 → 空。**需确认前端实发 key** |
| 3 | POST `/posts/createSchedule` | `post_write.go:createScheduledPost` → `App.CreateCsesScheduledMessage` (cses_post_schedule.go) | ✅ | Redis 缓存 + Pulsar Delayed Delivery；WS `post_schedule_created` 单播 userId。一致 |
| 4 | POST `/posts/cancelSchedule` | `post_write.go:cancelScheduledPost` → `App.CancelCsesScheduledMessage` | ✅ | WS `post_schedule_canceled` 单播。一致 |
| 5 | POST `/posts/getSchedule` | `post_write.go:getScheduledPost` → `App.GetCsesScheduledMessage` | ✅ | 读 Redis，未命中返回 nil。一致 |
| 6 | POST `/posts/createMock` | **无**（未注册路由 / 无 handler / 无 app 方法） | ❌（有意不迁） | 纯压测旁路（源直发 Pulsar MockTopic）。迁移注意点 §6「createMock 可废弃」已拍板。**非阻塞** |
| 7 | POST `/posts/urgentPost` | `post_write.go:urgentPost` → `App.SetUrgentPost` (cses_post_urgent.go) | 🟡 | 链路全在（co-tx post+成员+notification+notification_recipient+AppendEvent）。**WS 差异**：源标 `update_channel ×N + post_update`；im-server 发 `update_channel ×N`（badge 增量）**+ `post_urgent` 多播**（payload `{post, userIds}`）而非 `post_update`。见 WS 专章 |
| 8 | POST `/posts/postContext` | `posts.go:getPostContext` → `App.LoadPostContextById` (cses_post_read.go) | ✅ | viewers 鉴权一致。纯读 |
| 9 | POST `/posts/top20` | `posts.go:getTop20` → `App.GetTop20Posts` | ✅ | **snake_case `channel_id`** 已照搬（posts.go L125 verbatim）。一致 |
| 10 | POST `/posts/urgentConfirm` | `post_write.go:urgentConfirm` → `App.ConfirmPostUrgent` | 🟡 | co-tx 链一致。**WS 差异**：源标 `update_channel ×N + post_update`；im-server 发 **`post_confirm_urgent`** 单播 post 发送者（publishPostConfirmUrgent L247）。无 update_channel ×N（仅 confirm 事件）。见 WS 专章 |
| 11 | POST `/posts/urgentCancel` | `post_write.go:urgentCancel` → `App.CancelPostUrgent` | 🟡 | co-tx 删通知+移除加急列表+清 expedite_map 一致。**WS 差异**：源标 `update_channel ×N + post_update`；im-server 仅 `sendUrgentPostAndIncrement` 发 `update_channel ×N`（无 post_update / 无 post_urgent）。见 WS 专章 |
| 12 | POST `/posts/get` | `posts.go:getPost` → `App.LoadPostsByIds` | ✅ | `postIds *[]string` 一致 |
| 13 | POST `/posts/getPostsAfterIndex` | `posts.go:getPostsAfterIndex` → `App.GetPostsAfterIndex` | ✅ | **命名陷阱已照搬**：字段 `PostId` json tag `postIds`（单数 id）。返回 `{posts, count}` map 一致 |
| 14 | POST `/posts/revoke` | `post_write.go:revoke` → `App.RevokePost` (cses_post_revoke.go) | 🟡 | co-tx delete + reply 链 edit + AppendEvent 一致。**WS 差异**：源标 `post_update`；im-server 发 **`posts_update`（复数批帧）** + 撤回最后一条时 `update_channel`（lastPost JSON 字符串）。见 WS 专章 |
| 15 | POST `/posts/getReplies` | `posts.go:getReplies` → `App.GetReplies` | ✅ | `entity.GetPostOpts` 内嵌 + `ReplyId` 一致；响应 `GetRepliesResp{RootPost, Replies}` verbatim 对齐 |
| 16 | POST `/posts/getReplyBranch` | `posts.go:getReplyBranch` → `App.GetReplyBranch` | ✅ | `ReplyFirstLevelId` + GetPostOpts 一致 |
| 17 | POST `/posts/makeTopic` | `post_write.go:makeTopic` → `App.MakeTopic` (cses_post_maketopic.go) | ✅ | 内嵌 `CreateChannelSpecifyOwner` + `Cses-Track-Id` header 透传一致；WS `channel_created` + `post_update`（envelope.tracing.csesTrackId）。SetNX 幂等锁就位 |
| 18 | POST `/posts/queryTodoList` | `posts.go:queryTodoList` → `App.QueryTodoList` | ✅ | `postIds []string` + session 一致 |
| 19 | POST `/posts/getUpdatedPosts` | `posts.go:queryUpdatedPosts` → `App.QueryUpdatedPosts` | ✅ | **`timeStamp` (大写 S)** json tag 已照搬 + `limit`。Redis ZSET+HMGET 增量 |
| 20 | POST `/posts/getLatestPost` | `posts.go:getLatestPost` → `App.GetLatestPost` | ✅ | `channelId` + `timestamp`（未用）一致 |
| 21 | POST `/posts/quickReply` | `post_write.go:quickReply` → `App.QuickReply` | ✅ | co-tx emoji toggle (jsonb_set) + AppendEvent(edit) + WS `post_update`（publishPostUpdate）。**与源 `post_update` 一致**（注意：与 revoke 用的 posts_update 不同）。userId 取请求体（与源一致） |
| 22 | POST `/posts/updatePostProps` | `post_write.go:updatePostProps` → `App.UpdateVotePostProps` | ✅ | `postId` + `props` 一致；co-tx + WS `post_update`（publishPostUpdate）。一致 |
| 23 | POST `/post/read` | `post_write.go:postRead` → `App.ReadPost` (cses_post_mark_read.go) | 🟡 | `entity.PostRead`（ChannelID/EndTime/Posts/StartTime/UserId 五字段）一致；UserId nil 取 session 一致。**WS 差异**：源标 `update_channel + post_read`；im-server `publishRead` 发 `post_read`（须核对是否也发 update_channel 清 badge，见 WS 专章 ⚠️） |
| 24 | POST `/post/read/list` | `posts.go:postReadList` → `App.ReadPostList` | ✅ | `PostReadListParam{PostIds}` 一致；响应 `[]*PostReadDto`（PascalCase `PostId`/`ReadBits` 无 tag）须确认 im-server entity 同形 |
| 25 | POST `/post/templateReceived` | `post_write.go:templateReceived` → `App.TemplateReceived` | ✅ | `postId` 一致；co-tx + WS `post_update`。一致 |
| 26 | POST `/post/announcement/save` | `post_write.go:announcementSave` → `App.AnnouncementSave` | ✅ | 裸 `*entity.Post` 入参一致；返回 `{announcementId}`；WS `post_update`（过期历史广播）。一致 |
| 27 | POST `/post/announcement/read` | `post_write.go:announcementRead` → `App.AnnouncementRead` | ✅ | `postId`+`channelId` 一致；co-tx + WS post_update。一致 |
| 28 | POST `/post/announcement/acceptList` | `posts.go:announcementAcceptList` → `App.AnnouncementAcceptList` | ✅ | `postId` 一致；纯读 |
| 29 | POST `/post/announcement/delete` | `post_write.go:announcementDelete` → `App.AnnouncementDelete` | ✅ | `postIds []string` 一致 |
| 30 | POST `/post/announcement/list` | `posts.go:announcementList` → `App.AnnouncementList` | ✅ | channelId 空回落 postId 逻辑已照搬（posts.go L254-257） |
| 31 | POST `/post/announcement/detail` | `posts.go:announcementDetail` → `App.AnnouncementDetail` | ✅ | `postIds []string` 一致；纯读公告表 |
| 32 | POST `/post/approval/approval` | `post_approval.go:approvalPost` → `App.ApprovePost` | ✅ | `postId` 一致；co-tx 落 post_approval 表。WS 源标「视实现」，im-server approveInTx 未见显式 post_update（与源同为弱约定，非阻塞） |
| 33 | POST `/post/bookmark/create` | `post_bookmark.go:createPostBookmark` → `App.SavePostBookmark` | ✅ | `channelId`/`userId`/`postIds` 一致 |
| 34 | POST `/post/bookmark/delete` | `post_bookmark.go:deletePostBookmark` → `App.DeletePostBookmark` | ✅ | `userId`/`postId` 一致 |
| 35 | POST `/post/bookmark/load` | `post_bookmark.go:loadPostBookmark` → `App.LoadPostBookmark` | ✅ | `channelId`/`userId`/内嵌 `PageOpts` 一致；分页返回 |

**统计**：✅ 已迁移 27 | 🟡 部分迁移（WS/字段差异）7 | ❌ 缺失 1（createMock，有意不迁）

---

## WS 事件差异专章（替代 mattermost 的真正风险点）

csesapi 源 map 把多个 post 写路由的 WS 推送笼统标为 `post_update`，但 cses-im-server 按 `internal/ws/event_type.go` 做了**事件细分重构**（L25-26 注释自陈：「post_urgent / post_confirm_urgent / post_recall / post_reply 为 post 写路由的推送事件常量」）。逐条差异：

| 接口 | csesapi 源 map 标注 | im-server 实际发出 | 风险评级 |
|---|---|---|---|
| revoke | `post_update` | **`posts_update`**（复数批帧，含 reply 链）+ 撤回末条时 `update_channel` | 🔴 高 — 事件名不同，前端漏收即撤回不生效 |
| urgentPost | `update_channel ×N + post_update` | `update_channel ×N` + **`post_urgent`**（payload `{post, userIds}`） | 🔴 高 — post_update→post_urgent |
| urgentConfirm | `update_channel ×N + post_update` | **`post_confirm_urgent`** 单播发送者（无 update_channel ×N） | 🔴 高 |
| urgentCancel | `update_channel ×N + post_update` | `update_channel ×N`（无 post_update / post_urgent） | 🟡 中 |
| quickReply | `post_update` | `post_update` | ✅ 一致 |
| updatePostProps / templateReceived / announcement* / makeTopic | `post_update` | `post_update` | ✅ 一致 |
| post/read | `update_channel + post_read` | `post_read`（**须核对 ReadPost 是否同发 update_channel 清 badge**） | 🟡 中 — 见 ⚠️ |

> ⚠️ **必须活体核对 2 项**（静态读不足以定论，遵循 decisions/2026-05-29-static-review-vs-live-root-cause）：
> 1. `App.ReadPost`（cses_post_mark_read.go `publishRead`）是否在 `post_read` 之外**也发 `update_channel`** 清 badge——源标双事件，im-server 命名为 publishRead 单一函数，需 grep 确认。
> 2. im-server 的细分事件（posts_update / post_urgent / post_confirm_urgent）**是否就是客户端当前监听的事件名**。若客户端 cses-client 的 Rust handler 已按这些细分名重写（im-sync v2 重构后），则无差异、属"协议已双端对齐"；若客户端仍按旧 `post_update` 监听，则替代 mattermost 时撤回/加急会静默失效。
>
> **判据**：这些差异是「有意的协议升级」还是「迁移漏对齐」，取决于客户端 wire 契约现状 —— 不在本域可单方面定论，须与 client 侧 gap 文档交叉（`gap-client-*` / message-v3 Rust handler 监听表）。

---

## 缺失 / 待办能力优先级清单（按替代 mattermost 阻塞程度排序）

> 注：纯接口缺失只有 1 个且有意不迁；真正待办是「WS 协议对齐核对」。

### P0 — 阻塞替代（必须 cutover 前关闭）
1. **WS 事件命名双端核对（revoke/urgent/confirm 4 条）**：确认 im-server 的 `posts_update` / `post_urgent` / `post_confirm_urgent` 与客户端监听事件名一致。这是替代 mattermost 后「撤回不生效 / 加急不弹 / 确认回执丢」的直接根因点。**取证手段**：双端日志联调（im-dual-endpoint-log-debug skill）实发一条 revoke/urgent，看客户端是否收到并处理。
2. **`post/read` 是否同发 `update_channel` 清 badge**：源是双事件（post_read + update_channel）；im-server `publishRead` 须确认未漏 update_channel，否则已读后未读 badge 不清。

### P1 — 字段级 wire 对齐（前端发不出/收不到的静默坑）
3. **`createPosts` 请求体 key 大小写**：源无 tag = PascalCase（`Posts`/`ChannelIds`）；im-server camelCase（`posts`/`channelIds`）。须确认前端实发 key，否则转发/批量发 decode 空。
4. **`post/read/list` 响应 `PostReadDto` 无 tag = PascalCase**（`PostId`/`ReadBits`）：须确认 im-server 对应 entity 同形（PascalCase wire），否则前端读不到已读 bitmap。

### P2 — 非阻塞（已拍板不迁）
5. **`createMock`**：纯压测旁路，迁移注意点 §6 明确可废弃。无需补。

### 已确认无缺口（不必担心）
- 全部只读接口（get / postContext / getReplies / getReplyBranch / getLatestPost / top20 / queryTodoList / getUpdatedPosts / getPostsAfterIndex / read/list / announcement 只读三件套）— 路由 + app 全就位，签名 verbatim 对齐（含 snake_case `channel_id`、`timeStamp` 大写 S、`postIds` 单数命名陷阱三处都已照搬）。
- 全部写接口（urgent 三件套 / revoke / quickReply / updatePostProps / makeTopic / templateReceived / announcement 写三件套 / approval / schedule 三件套 / bookmark 三件套）— co-tx + channel_event AppendEvent 不变量（C002/C003）已落地，DB 表齐备。
- WS 事件常量定义齐备：`ws/event_type.go` 含 post_update / posts_update / post_read / post_urgent / post_confirm_urgent / post_recall / post_reply / post_schedule_created / post_schedule_canceled / update_channel / channel_created — **覆盖源域全部 6 个事件 + 4 个细分新增**。

---

## 备注

- 覆盖率层面（04/05 gap-map）：post 域 app 方法**实现就位但单测覆盖偏低**（cses_post_urgent/maketopic/revoke/mark_read/schedule 多为 0% 单测命中）—— 这是**测试覆盖缺口**，不是**功能缺口**。功能链路（路由→app→store→WS）经源码核对均已接通。
- 本域 gap 的本质：**不是接口没搬，而是 WS 协议在搬运过程中做了"事件细分"重构**。重构本身合理（细分事件比笼统 post_update 更利于客户端精确处理），但**双端契约一致性必须验证**才能判定是"升级"还是"drift"。
