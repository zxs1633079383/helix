# Helix 接管联调测试矩阵（CI-4 可执行测试清单）

> **用途**：helix-im 接管 cses-client IM 后的端到端联调（CI-4）逐条测试脚本蓝本。
> 覆盖 **89 outbound 命令**（88 registry HTTP-fire + `im_send_message` send arm）逐条 ：命令名 /
> HTTP 端点+方法 / 请求体关键字段 / **触发的 WS 推送事件**（写命令）或「无 WS·验 HTTP 响应」（读命令）+
> 每条期望帧断言点 + 落库 / `im:__bus__` 投影验证点，按 **8 阶段状态机顺序**编排成可勾选 checklist。
>
> **真源（不可偏离）**：
> - WS 事件映射 → `helix/crates/helix-im/docs/csesapi-migration/full-map/partials/5--backend-ws-events.md`（19 事件 wire 契约 + 触发链路）
> - 命令清单 → `helix/crates/helix-im/src/outbound/`（distributed_slice 注册）+ `module.rs`（send arm）
> - 投影契约 → `helix/crates/helix-im/docs/projection-schema.md`（21 工厂 → 20 channel `data` 键集冻结）
>
> **方法论** → 同目录 `2026-06-23-HTTP-WS接管联调方法论.md`（HTTP↔WS 因果 + 测试编排范式）。

---

## 0. 三铁律断言点（每条写命令的期望帧都要过这三关）

接管联调最易翻车的三个 wire 契约点（来自 `5--backend-ws-events.md` §3 + `projection-schema.md` §6）：

| 铁律 | 断言点 | 出处 |
|---|---|---|
| **① `event_seq` 顶层注入** | `post` / `posts_update` / `post_update`(编辑路径) 三事件，`data` **顶层**有 `event_seq`(int64)，源自 `post.Props[PostEventSeqPropKey]`；客户端 pts gate 直接读 `data.event_seq`（**不**解 `data.props.channel_event_seq`）。缺顶层 = cursor gate 收不到 = 离线对账断 | WS doc §2.1/§2.2/§2.3 + 校正清单 #3 |
| **② `lastPost` 必须 JSON string** | `update_channel` / `increment_channel` 的 `lastPost` 字段是**序列化后的 JSON 字符串**（非嵌套 object）。客户端 channel VO `last_post: String`。若推 map → 反序列化报 `field lastPost: invalid type: map, expected a string`（2026-05-29 实测踩坑），会话 lastPost 不刷 | WS doc §2.14 + 校正清单 #4 |
| **③ casing 三陷阱** | (a) `change_channel_approval` wire action **无 Status 后缀**（常量名 `WebsocketEventChangeChannelApprovalStatus` 带 Status，但 wire 值是 `change_channel_approval`）；(b) 时间字段 `createAt` **非** `createdAt`/`created_at`（wire/前端 camelCase 漏 d）；(c) `top20` body 用 **snake_case** `channel_id`、`getUpdatedPosts` 用 **`timeStamp` 大写 S**、`getPostsAfterIndex` 的 `postIds` 是**单 id string 非数组** | WS doc §2.17 注 + projection §6 + posts_read.rs |

> **投影 channel_id vs channelId 陷阱**：helix emit 信号类用 `channel_id`(snake)，透传 wire 类用 `channelId`(camel)——逐 channel 不同，见 projection §1 表，不可假设统一。

---

## 1. 89 命令 ↔ HTTP 端点 ↔ WS 推送 全映射（计数核对真源）

> 端点前缀统一 `/api/cses`（下表 path 省略前缀）。method 列：HTTP-fire 命令现 registry 恒发 **POST**
> （`http_post` 单点），GET/DELETE 语义差异属 driver 接缝 P6（注释标 †=现网真实 method 非 POST，driver 映射时校正）。
> **WS 推送**列：写命令填触发的 WS event（锚 `5--backend-ws-events.md`）；读命令填「无 WS·验 HTTP 响应」。

### 1.1 send arm（1 条，不在 registry，走 module.rs send arm）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS 推送 | 投影 channel（`im:__bus__`） |
|---|---|---|---|---|---|
| S0 | `im_send_message` | `posts/create` POST | post 对象（channelId/message/temporaryId/...，body 真源见 `真机curl真源.md`）| **`post`**（首发 `SendPostMessage`，顶层 `event_seq`）| 乐观：`im:post:sending`{channel_id,temporary_id}；echo：`im:post:received`{channel_id,event_seq,msg_id} |

### 1.2 posts 写族（registry，10 条）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS 推送 | 投影 channel |
|---|---|---|---|---|---|
| 1 | `im_create_posts` | `posts/createPosts` POST | `{posts:[Post...], channelIds:[...]}`（camelCase 透传）| **`post`** × 逐目标 channel | `im:post:received` 逐 channel |
| 2 | `im_post_read` | `post/read` POST | `{channelId, endTime?}` | **`post_read`**(≤2人) + **`update_channel`**(刷未读) | `im:post:read`{channel_id,event_seq,msg_id} + `im:channel:update` |
| 3 | `im_revoke` | `posts/revoke` POST | `{postId}` | **`posts_update`**（撤回批量，顶层 max event_seq）+ `update_channel`(刷 lastPost) | `im:post:deleted` / `im:post:batch-updated` + `im:channel:update` |
| 4 | `im_create_schedule` | `posts/createSchedule` POST | `{post:{channelId,message,temporaryId?}, schedulePostAt}` | **`post_schedule_created`**(UserId 本人) | `im:channel:schedule-created`{channelId,hasSchedulePost} |
| 5 | `im_cancel_schedule` | `posts/cancelSchedule` POST | `{channelId}` | **`post_schedule_canceled`**(UserId 本人) | `im:channel:schedule-canceled`{channelId,hasSchedulePost} |
| 6 | `im_urgent_post` | `posts/urgentPost` POST | `{channelId, postId, targetIds:[], message?}` | **`post_update`**(加急，UserIds 定向，顶层 event_seq) | `im:post:updated`{channel_id,event_seq,msg_id} |
| 7 | `im_urgent_confirm` | `posts/urgentConfirm` POST | `{postId, channelId}` | **`post_update`** | `im:post:updated` |
| 8 | `im_urgent_cancel` | `posts/urgentCancel` POST | `{postId, channelId}` | **`post_update`** | `im:post:updated` |
| 9 | `im_quick_reply` | `posts/quickReply` POST | `{postId, emoji, userId?}` | **`post_update`**(reaction) | `im:post:updated` |
| 10 | `im_update_post_props` | `posts/updatePostProps` POST | `{postId, props:{}}` | **`post_update`** | `im:post:updated` |
| 11 | `im_template_received` | `post/templateReceived` POST（**前缀 `post` 单数**）| `{postId}` | **`post_update`** | `im:post:updated` |

> ⚠️ 行 #11 端点前缀 `post`（单数），勿写 `posts`（命名陷阱，`api.go:171` Post subrouter）。

### 1.3 channel 写族（registry，14 条）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS 推送 | 投影 channel |
|---|---|---|---|---|---|
| 12 | `im_create_channel` | `channel/create` POST | raw body（ent.Channel + users[]，直传透传）| **`channel_created`** + **`channel_member_update`** + **`post`**(系统加群) | `im:channel:created`{channel_id,channel} + `im:channel:member-updated` + `im:post:received` |
| 13 | `im_create_specify_owner` | `channel/createSpecifyOwner` POST | raw body（须含 `displayName`；snake_case `direct_key` 保真）| 同 #12 | 同 #12 |
| 14 | `im_make_topic` | `posts/makeTopic` POST | raw body（须含 `postId`）| **`channel_created`** + **`post_update`** | `im:channel:created` + `im:post:updated` |
| 15 | `im_channel_member_change` | `channel/member/change` POST | `{channelId, joinUsers?:[], leaveUsers?:[]}`（admin 增减）| **`channel_member_update`** | `im:channel:member-updated`{channel_id,channel} |
| 16 | `im_channel_leave` | `channel/member/leave` POST | `{channelId}`（仅自己退）| **`channel_member_update`** + **`post`** | `im:channel:member-updated` + `im:post:received` |
| 17 | `im_channel_close` | `channel/close` POST | `{channelId}` | **`channel_close`**(broadcast channelId，自己也收) | `im:channel:closed`{channelId,deleteAt} |
| 18 | `im_update_member_nickname` | `channel/member/change/nickname` POST | `{channelId, nickname, userId?}`（nickname 空=清空）| **`update_channel_member_nickName`**(camelCase) | `im:channel:memberNickname`{channelId,userId,nickName} |
| 19 | `im_channel_member_role` | `channel/member/change/role` POST | `{channelId, userIds:[], role}` | **`channel_member_role_updated`**(broadcast channelId) | （无专投影信号；前端回读成员表）|
| 20 | `im_channel_member_notify` | `channel/member/change/notify` POST | `{channelId, notify}` | **`update_channel`**(定向本人) | `im:channel:update`{channel_id} |
| 21 | `im_channel_change_info` | `channel/change/info` POST | `{channelId, name?/displayName?/header?/purpose?/orient?/module?/pictureType?/picture?/source?}` | **`update_channel`** | `im:channel:update` |
| 22 | `im_channel_change_source` | `channel/change/source` POST | `{id, title}`（**`id`**=channelId 键陷阱）| **`update_channel`** | `im:channel:update` |
| 23 | `im_channel_change_picture` | `channel/change/picture` POST | `{id, pictureType, picture}`（**`id`** 键）| **`update_channel`** | `im:channel:update` |
| 24 | `im_channel_change_notice` | `channel/change/notice` POST | `{id, notice}`（**`id`** 键，notice=map）| **`update_channel_notice`**(可伴 post 系统消息) | `im:channel:update`（+ 公告内容）|
| 25 | `im_channel_change_top` | `channel/change/top` POST | `{channelId, top:bool}`（per-member）| **`update_channel`**(定向本人) | `im:channel:update` |

> ⚠️ 行 #22/#23/#24 用 **`id`** 作 channelId 键（Go command json tag `json:"id"`），#21/#25 用 **`channelId`**——同族不同键（decode 静默失败防线，写错 Go 收空 id 拒绝）。
> ⚠️ **`change/props`（#15 change/props）故意不在 89**：现网 `cses_channel.go:1226-1250` 落库 Props 后**无 a.Publish**（与其他 change/* 不一致）。helix 不补 `im_update_channel_props` outbound，也**勿**为它补 `update_channel` WS handler（会凭空广播服务端不推的事件，破坏对账）。

### 1.4 posts 读族（registry，17 条 · 无 WS·验 HTTP 响应）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS | 验证点 |
|---|---|---|---|---|---|
| 26 | `im_get_schedule` | `posts/getSchedule` POST | `{channelId, userId}` | 无 WS | HTTP 响应 = 定时消息列表 |
| 27 | `im_post_context` | `posts/postContext` POST | `{postId, before:int}` | 无 WS | 响应 = post 上下文 |
| 28 | `im_top20` | `posts/top20` POST | `{channel_id}`（**snake_case 唯一读端陷阱**）| 无 WS | 响应 = 首屏 20 条 |
| 29 | `im_get_posts` | `posts/get` POST | `{postIds:[]string}`（复数数组）| 无 WS | 响应 = 批量 post |
| 30 | `im_get_posts_after_index` | `posts/getPostsAfterIndex` POST | `{postIds: <单 id string>}`（**单 string 非数组陷阱**）| 无 WS | 响应 = 往后翻页 |
| 31 | `im_get_replies` | `posts/getReplies` POST | `{replyId, pageNumber, pageSize, revoke?}` | 无 WS | 响应 = 回复列表 |
| 32 | `im_get_reply_branch` | `posts/getReplyBranch` POST | `{replyFirstLevelId, pageNumber, pageSize}` | 无 WS | 响应 = 分支回复 |
| 33 | `im_query_todo_list` | `posts/queryTodoList` POST | `{postIds:[]string}` | 无 WS | 响应 = 待办状态 |
| 34 | `im_get_updated_posts` | `posts/getUpdatedPosts` POST | `{timeStamp, limit}`（**大写 S 陷阱**）| 无 WS | 响应 = 游标拉更新 post |
| 35 | `im_get_latest_post` | `posts/getLatestPost` POST | `{channelId, timestamp}` | 无 WS | 响应 = 最新首屏（too_long 重拉链）|
| 36 | `im_bookmark_create` | `post/bookmark/create` POST | `{channelId, userId, postIds:[]}` | 无 WS | 响应 = 创建结果 |
| 37 | `im_bookmark_delete` | `post/bookmark/delete` POST | `{userId, postId}` | 无 WS | 响应 = 删除结果 |
| 38 | `im_bookmark_load` | `post/bookmark/load` POST | `{channelId, userId}` | 无 WS | 响应 = 书签列表 |
| 39 | `im_announcement_accept_list` | `post/announcement/acceptList` POST | `{postId}` | 无 WS | 响应 = 公告接收列表 |
| 40 | `im_announcement_list` | `post/announcement/list` POST | `{channelId, postId?}` | 无 WS | 响应 = 公告列表 |
| 41 | `im_announcement_detail` | `post/announcement/detail` POST | `{postIds:[]string}` | 无 WS | 响应 = 公告详情 |
| 42 | `im_post_read_list` | `post/read/list` POST | `{postIds:[]string}` | 无 WS | 响应 = 已读位历史（B1 非 bitmap）|

> ⚠️ #36-42 端点前缀 **`post`（单数）** + `/bookmark/` `/announcement/` `/read/` 子路由。

### 1.5 user-misc 族（registry，11 条 · 无 WS·验 HTTP 响应）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS | 验证点 |
|---|---|---|---|---|---|
| 43 | `im_list_users` | `users/list` POST | `{channelId}` | 无 WS | 响应 = 成员用户 |
| 44 | `im_get_user_statuses` | `users/status/ids` POST | `{userIds:[]string}` | 无 WS | 响应 = 在线状态（presence 接管 D2）|
| 45 | `im_team_upsert` | `teams/upsert` POST | `team`（整对象透传）| 无 WS | 响应 = upsert 结果 |
| 46 | `im_team_member_add` | `teams/member/add` POST | `event`（ChannelMemberEvent 透传）| 无 WS | 响应 = 变更结果 |
| 47 | `im_team_quit` | `teams/member/quit` POST† | `{userId, teamId}` | 无 WS（**注**：服务端退公司事务可推 `quit_company`，属服务端态非本命令直推）| HTTP DELETE† 无响应体（源 bug 透传）|
| 48 | `im_get_all_modules` | `modules/getAll` POST | **空 body** `{}` | 无 WS | 响应 = 会话分组模块 |
| 49 | `im_load_send_notifications` | `notification/loadSend` POST | `{page:int≥1, size:int≥1, state?}` | 无 WS | 响应 = 通知列表 |
| 50 | `im_search_post` | `search/post` POST | DoSearchRequest（`{channelId, keyword?, searchType?, teamId?, isAll?}`；userId 服务端注入禁前端传）| 无 WS | 响应 = 消息搜索结果 |
| 51 | `im_search_user` | `search/user` POST | 同 DoSearchRequest | 无 WS | 响应 = 用户搜索 |
| 52 | `im_search_channel` | `search/channel` POST | 同 DoSearchRequest | 无 WS | 响应 = 频道搜索 |
| 53 | `im_search_do` | `search/do` POST | 同 DoSearchRequest | 无 WS | 响应 = 聚合（channels/users/channelDetails 四 key）|

### 1.6 bot-agent 域（registry，35 条）

> bot-agent 端点子路由：`posts/*`(gateway) · `bot-manage/*` · `bot-agent/*` · `agents/*` · `webhook/*`。
> **calls 族（#23-27，含 #info）`auth_kind=Bot`**（bot-token 调用，driver 注 bot 凭据）。其余 `Session`。
> †=现网真实 method GET/DELETE（driver P6 校正）；path 含 `{param}`=`path_override` 动态拼。

#### gateway（2 条 · 落 post 后推 WS）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS 推送 | 投影 channel |
|---|---|---|---|---|---|
| 54 | `im_create_bot` | `posts/createBot` POST | `{sessionId, userId?, channelId?, conversationType?, text}` | **`post`**(bot 变体，§2.19) | `im:post:received` |
| 55 | `im_bot_callback` | `posts/botCallback` POST | `{messageId, timestamp, to, text, mediaUrl?, ...}` | **`post`**(bot 回复，§2.19；helix 注释标 `posted` 为原生路径名，**矩阵以 WS doc §2.19 `post` 为准**) | `im:post:received` |

#### bot_manage（9 条 · 无 WS·验 HTTP 响应）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS | 验证点 |
|---|---|---|---|---|---|
| 56 | `im_bot_list_visible` | `bot-manage/list` POST† | 空 body（session 取 userId/companyId）| 无 WS | 响应 = 可见 bot 列表 |
| 57 | `im_bot_create` | `bot-manage/create` POST | `{name, description, teamId?}` | 无 WS | 响应 = 创建 bot |
| 58 | `im_bot_token_generate` | `bot-manage/token/generate` POST | `{botUserId, description, expireAt?}` | 无 WS | 响应 = token |
| 59 | `im_bot_token_list` | `bot-manage/token/list?botUserId=` POST† | query param `botUserId` | 无 WS | 响应 = token 列表 |
| 60 | `im_bot_token_revoke` | `bot-manage/token/revoke` POST | `{tokenId}` | 无 WS | 响应 = 撤销结果 |
| 61 | `im_bot_scope_set` | `bot-manage/scope/set` POST | `{botUserId, scopeType, teamId?}` | 无 WS | 响应 = scope 结果 |
| 62 | `im_bot_visible_user_add` | `bot-manage/scope/visible-user/add` POST | `{botUserId, userId}` | 无 WS | 响应 = 添加结果 |
| 63 | `im_bot_visible_user_remove` | `bot-manage/scope/visible-user/remove` POST | `{botUserId, userId}` | 无 WS | 响应 = 移除结果 |
| 64 | `im_bot_visible_user_list` | `bot-manage/scope/visible-user/list?botUserId=` POST† | query param `botUserId` | 无 WS | 响应 = 可见用户列表 |

#### config（10 条 · 无 WS·验 HTTP 响应 · path param 多）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS | 验证点 |
|---|---|---|---|---|---|
| 65 | `im_bot_agent_config_create` | `bot-agent/config` POST | config 整对象（校验 botUserId）| 无 WS | 响应 = 创建 config |
| 66 | `im_bot_agent_config_get` | `bot-agent/config/{botUserId}` POST† | path param | 无 WS | 响应 = config |
| 67 | `im_bot_agent_configs_enabled` | `bot-agent/configs/enabled` POST† | 空 body | 无 WS | 响应 = 启用列表 |
| 68 | `im_bot_agent_config_delete` | `bot-agent/config/{botUserId}` POST† | path param（DELETE）| 无 WS | 响应 = 删除结果 |
| 69 | `im_bot_agent_regenerate_token` | `bot-agent/config/{botUserId}/regenerate-token` POST | path param | 无 WS | 响应 = 新 token |
| 70 | `im_bot_agent_test_webhook` | `bot-agent/config/{botUserId}/test-webhook` POST | path param + `{message}` | 无 WS | 响应 = 测试结果 |
| 71 | `im_bot_agent_channel_bots` | `bot-agent/channel/{channelId}/bots` POST† | path param | 无 WS | 响应 = channel bot 列表 |
| 72 | `im_bot_agent_channel_available_bots` | `bot-agent/channel/{channelId}/available-bots` POST† | path param | 无 WS | 响应 = 可用 bot |
| 73 | `im_bot_agent_channel_add` | `bot-agent/channel/{channelId}/bot/{botUserId}` POST | 双 path param（addedBy=session）| 无 WS | 响应 = 添加结果 |
| 74 | `im_bot_agent_channel_remove` | `bot-agent/channel/{channelId}/bot/{botUserId}` POST† | 双 path param（DELETE）| 无 WS | 响应 = 移除结果 |

#### calls（6 条 · `auth_kind=Bot` · 无 WS·验 HTTP 响应）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS | 验证点 |
|---|---|---|---|---|---|
| 75 | `im_bot_agent_info` | `bot-agent/info` POST†（Bot）| 空 body（ctx config 取）| 无 WS | 响应 = `{id,username,displayName}` |
| 76 | `im_bot_agent_send_direct` | `bot-agent/messages/direct` POST（Bot）| `{teamId, userId, channelId?, message, ...}` | 无 WS（bot 落 post → 服务端态推 `post`，非本命令直产）| HTTP 响应 = 发送结果 |
| 77 | `im_bot_agent_send_channel` | `bot-agent/messages/channel` POST（Bot）| `{teamId, channelId, message}` | 无 WS（同上）| HTTP 响应 |
| 78 | `im_bot_agent_team_members` | `bot-agent/teams/{team_id}/members?page=&per_page=` POST†（Bot）| path + query | 无 WS | 响应 = 成员 |
| 79 | `im_bot_agent_get_user` | `bot-agent/teams/{team_id}/users/{user_id}` POST†（Bot）| 双 path | 无 WS | 响应 = 用户 |
| 80 | `im_bot_agent_team_channel` | `bot-agent/teams/{team_id}/channel` POST†（Bot）| path | 无 WS | 响应 = 默认 channel |

#### agents（5 条）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS 推送 | 投影 channel |
|---|---|---|---|---|---|
| 81 | `im_agent_create` | `agents/create` POST | `{name, type, description?, teamId, ownerUserId?}` | 无 WS | HTTP 响应 = 创建 agent |
| 82 | `im_agent_join_channel` | `agents/join-channel` POST | `{type, channelId, teamId}` | 无 WS | HTTP 响应 |
| 83 | `im_agent_start_session` | `agents/start-session` POST | `{sessionId?, taskName?, teamId, starterUserId?, ...}` | 无 WS | HTTP 响应 = session |
| 84 | `im_agent_send` | `agents/send` POST | `{sessionId, agentType, text, conversationType?, channelId?, userId?}` | 无 WS（落 post→服务端态）| HTTP 响应 |
| 85 | `im_agent_callback` | `agents/callback` POST | `{messageId, ..., text, usage{}, routing{}}` | **`post`**(agent 回复，§2.19；**内存构造 post 不推 cursor**，与 mattermost 一致不写 channel_event) | `im:post:received`（注意：无 event_seq 推进）|

#### webhook（3 条 · 无 WS·验 HTTP 响应）

| # | 命令名 | 端点 / method | 请求体关键字段 | WS | 验证点 |
|---|---|---|---|---|---|
| 86 | `im_webhook_config_set` | `webhook/config` POST | `{targetUrl, secret, botUserIds:[], triggerMode, channelFilter?}` | 无 WS | 响应 = config |
| 87 | `im_webhook_config_get` | `webhook/config` POST† | 空 body | 无 WS | 响应 = `{configured?}` |
| 88 | `im_webhook_test` | `webhook/test` POST | `{message?}`（出站 OpenClaw）| 无 WS | 响应 = 测试结果 |

### 1.7 计数核对（gate 第一关）

| 域 | 命令数 |
|---|---|
| send arm（module.rs） | 1（`im_send_message`，不在 registry）|
| posts 写族 | 11 |
| channel 写族 | 14 |
| posts 读族 | 17 |
| user-misc | 11 |
| bot-agent | 35（gateway 2 + bot_manage 9 + config 10 + calls 6 + agents 5 + webhook 3）|
| **registry 小计** | **88** |
| **总计（registry 88 + send arm 1）** | **89** ✅ |

> 机器核对：`grep -rhA1 'fn name(&self)' src/outbound | grep -oE '"im_[a-z0-9_]+"' | sort -u | wc -l` = 88（registry）；`outbound_command_count()` 断言 == 88（`tests/cp_create_posts_test.rs`）；+ send arm = 89。

### 1.8 WS 事件覆盖核对（19 事件 ↔ 89 命令 反查）

> `5--backend-ws-events.md` 列 19 个 WS event。下表反查每个 event 由哪些 outbound 命令触发（或无 outbound）：

| WS event | 触发命令 | 备注 |
|---|---|---|
| `post` | S0, #1, #12(系统), #16(系统), #54, #55, #85 | 首发 + 系统 + bot/agent 三变体共用 |
| `posts_update` | #3(撤回) | 批量 post 更新 |
| `post_update` | #6/#7/#8(加急), #9(快回), #10(props), #11(模板), #14(makeTopic) | 单 post 字段更新 |
| `post_read` | #2 | ≤2 人 |
| `post_pin` | （无专 outbound，置顶/取消置顶路由未在 89 接管集）| 见 §1.9 缺口 |
| `post_schedule_created` | #4 | |
| `post_schedule_canceled` | #5 | |
| `increment_channel` | （服务端增量 worker 推，无前端直发 outbound；客户端经 sync/hello 触发）| inbound 侧 handler |
| `increment_channel_end` | 同上 | inbound 侧 handler |
| `channel_created` | #12, #13, #14 | |
| `channel_member_update` | #15, #16 | |
| `channel_member_role_updated` | #19 | |
| `channel_close` | #17 | |
| `update_channel` | #2, #20, #21, #22, #23, #25 | 最高频复用 |
| `update_channel_notice` | #24 | |
| `update_channel_member_nickName` | #18 | camelCase |
| `change_channel_approval` | **（无专 outbound，发言审批 toggle 端点未在 89 接管集）** | 见 §1.9：WS 事件存在但无 outbound 命令——inbound handler 仍需处理，无 outbound 测试行 |
| `quit_company` | #47(间接，服务端退公司事务推) | 非命令直产 |
| `post`(bot 变体) | #54, #55, #85 | = §2.19 |

### 1.9 已知缺口（self-consistency，联调前确认）

- **`post_pin` / `change_channel_approval` 两 WS event 无对应 outbound 命令**：置顶消息、发言审批 toggle 的 HTTP 触发端点未纳入 89 接管集。inbound WS handler 若收到这两事件仍需投影（否则前端漏渲染），但**没有 outbound 测试行**——CI-4 测这两个 event 需用其他用户/管理台触发或服务端造数。
- **`im_agent_callback`(#85) 推 `post` 但不推 cursor**：内存构造 post 不写 channel_event（与 mattermost 一致）→ `im:post:received` 不带 event_seq 推进，离线 sync 拉不到这条。联调断言：在线收到渲染即可，**勿**断言其 event_seq 单调推进。

---

## 2. 8 阶段可勾选 checklist（状态机式编排）

> **编排原理**：按依赖链状态机式排序——后一阶段的前置依赖由前一阶段产出（连接 → 列表 → 历史 → 实时 → 变更）。
> 每阶段：**前置依赖 → 操作步骤 → 期望结果 → 判据**。判据列勾选框 `[ ]`。

### 阶段 ① 连接层（connect → hello → connectionId）

**前置依赖**：Go 后端 `cses-im-server` 起；`IM_ENGINE` 开关已开（CI-3 灰度白名单命中）；有效 session token。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| 启动 cses-client，触发 IM 连接（`im_reconnect` 或自动）| WS 建连 → 收 hello 握手帧 | `[ ]` driver 收到 connectionId |
| 观察 `im:__bus__` | emit `im:connection:established`{connectionId} | `[ ]` payload.data 有 `connectionId` 非空 |
| 检查后续 outbound HTTP 头 | 所有命令带 `connectionId` 身份头（registry.rs `connection_id_headers`）| `[ ]` HTTP 请求头含 connectionId |

> **状态机门**：connectionId 未拿到 → 后续所有 outbound 的身份头缺失 → 全链路阻塞。**此阶段红则停**。

### 阶段 ② increment（冷启动会话列表 → 落库 channel）

**前置依赖**：① 通过（connectionId 在手）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| hello 后服务端推 `increment_channel` 批量（逐 channel 一条）| driver 收 N 条 increment 帧 | `[ ]` 收到 ≥1 条 `increment_channel` |
| 观察末尾哨兵 | 收 `increment_channel_end`（批次结束）| `[ ]` 收到 end（否则前端永远 loading）|
| 检查每条 `lastPost` 字段 | **JSON string**（非 map）| `[ ]` 铁律②：lastPost 是 string |
| 检查落库 | `channel` 表写入 N 行（patch.rs wire→column）| `[ ]` SQLite `SELECT count(*) FROM channel` = N |
| 观察 `im:__bus__` | emit `im:channel:increment`{channel_id,increment} 逐条 + `im:channel:update`{channel_id}(批次结束) | `[ ]` 投影 channel 命中 projection §1 表 |
| 检查 `last_event_seq` 列 | 每 channel 写入 sync cursor（HX-C008，`fillLastEventSeq`）| `[ ]` `channel.last_event_seq` 非空 |

### 阶段 ③ sync（离线补齐 → 落 message）

**前置依赖**：② 通过（channel 列表 + 每 channel `last_event_seq` 在手）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| 客户端按 `last_event_seq` 调 `POST /api/v4/channels/sync` 拉 channel_event 增量 | 收 sync 响应（type1 可见/type2 编辑/type3 撤回/type6 已读）| `[ ]` 收到 sync 响应 |
| cursor 锚点 | 续拉锚 **nextSeq**（非 max event）；续拉判据看 **len≥500**（真 wire 无显式 flag，HX-C008）| `[ ]` cursor 锚 nextSeq；len≥500 才续拉 |
| 落库 message | type1/type2/type3 落 `message` 表（gate peek→persist→commit，cursor 单调不回退）| `[ ]` `message` 表新增对应行 |
| too_long 兜底 | gap > 阈值 → `im:sync:too_long`{channelId,resetTo,mode,dropLocalMessages}（清内存 buffer + drop 本地消息派生行 + 重拉）| `[ ]` too_long 时 cursor=resetTo-1，message 表按 channel 清理后等待后续首屏/全量投影 |
| 观察 `im:__bus__` | type1→`im:channel:update-by-post`{channel_id,event_seq,msg_id}；type2→`im:post:updated`；type3→`im:post:deleted`；type6→`im:channel:read_echo` | `[ ]` 各 sync 类型投影 channel 命中 |

> **信号类铁律**：sync 信号事件只携 `channel_id`+`event_seq`+`msg_id` 三字段（权威行已落 DB），前端按 id 回读，**禁**塞半份 DialogItem（projection §2 C2 证伪点）。

### 阶段 ④ 发消息 echo（send → post 帧 → echo 对账）

**前置依赖**：①②③ 通过（连接稳 + 至少 1 个 channel 可发）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| 发消息：invoke `im_send_message`（S0）| 乐观上屏：emit `im:post:sending`{channel_id,temporary_id} | `[ ]` 乐观 UI 立即出现（temporaryId PK）|
| HTTP 出站 | `POST posts/create`，body 对齐 `真机curl真源.md`（体不对 = 无回声真因）| `[ ]` body 逐字对齐真抓包 |
| 服务端回声 | WS 推 **`post`**（首发 `SendPostMessage`，**非** `posted`）+ 顶层 `event_seq` | `[ ]` 铁律①：`data.event_seq` 顶层 int64 |
| echo 对账 | emit `im:post:received`{channel_id,event_seq,msg_id} → 乐观态 Sending→Acked | `[ ]` temporaryId 行 sendStatus 翻 Acked |
| 落库 | `message` 表 read_bits/createAt 等列写入（createAt 非 createdAt）| `[ ]` 铁律③：`createAt` camelCase |

### 阶段 ⑤ mutation（已读 / 编辑 / 撤回）

**前置依赖**：④ 通过（channel 内有消息）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| 已读：invoke `im_post_read`（#2）| WS `post_read`(≤2人) + `update_channel`(刷未读)；emit `im:post:read` + `im:channel:update` | `[ ]` readBits 由 Rust 投喂（前端禁算）|
| 撤回：invoke `im_revoke`（#3）| WS `posts_update`(顶层 max event_seq) + `update_channel`(刷 lastPost JSON string)；emit `im:post:deleted`/`im:post:batch-updated` | `[ ]` 铁律②：撤回刷 lastPost 是 string（2026-05-29 踩坑点）|
| 加急：invoke `im_urgent_post`(#6)/confirm/cancel | WS `post_update`(UserIds 定向，顶层 event_seq)；emit `im:post:updated` | `[ ]` 加急定向受众收到 |
| 快回/props/模板：#9/#10/#11 | WS `post_update`；emit `im:post:updated` | `[ ]` 各触发 post_update |

### 阶段 ⑥ channel 写族（create / member / 角色 / 昵称 / 公告 / 审批 / 解散）

**前置依赖**：①② 通过（连接 + 有权限的用户）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| 建群：`im_create_channel`(#12)/`im_create_specify_owner`(#13) | WS `channel_created`+`channel_member_update`+`post`；emit `im:channel:created`{channel_id,channel}+`im:channel:member-updated`+`im:post:received` | `[ ]` 三连 WS 齐 + 投影 channel 字段齐 |
| 成员增减：`im_channel_member_change`(#15) | WS `channel_member_update`；emit `im:channel:member-updated` | `[ ]` 成员表回读刷新 |
| 改角色：`im_channel_member_role`(#19) | WS `channel_member_role_updated`(broadcast channelId) | `[ ]` 角色更新广播 |
| 改昵称：`im_update_member_nickname`(#18) | WS `update_channel_member_nickName`(camelCase)；emit `im:channel:memberNickname`{channelId,userId,nickName} | `[ ]` camelCase nickName 字段 |
| 改公告：`im_channel_change_notice`(#24，body `{id,notice}`)| WS `update_channel_notice`；emit `im:channel:update` | `[ ]` body 用 `id` 键（非 channelId）|
| 改群信息：`im_channel_change_info/source/picture/top`(#21-23,#25)| WS `update_channel`；emit `im:channel:update` | `[ ]` source/picture 用 `id` 键陷阱 |
| 解散：`im_channel_close`(#17) | WS `channel_close`(自己也收)；emit `im:channel:closed`{channelId,deleteAt} | `[ ]` 解散广播 channelId |
| **审批 toggle（无 outbound，§1.9 缺口）** | 服务端推 `change_channel_approval`（wire **无 Status 后缀**）；inbound handler 渲染 | `[ ]` 铁律③：action 无 Status；inbound 能渲染 |

### 阶段 ⑦ 纯读族（posts 读 / user-misc / search · 无 WS·验 HTTP 响应）

**前置依赖**：①（连接 + connectionId 身份头）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| posts 读：#26-42（共 17）逐条 invoke | **无 WS**；HTTP 响应回灌 await 的语义 invoke（前端纯渲染消费返回值）| `[ ]` 各命令 HTTP 200 + 响应结构对 |
| 命名陷阱核验 | `im_top20`(#28) body snake `channel_id`；`im_get_updated_posts`(#34) `timeStamp` 大写 S；`im_get_posts_after_index`(#30) `postIds` 单 string | `[ ]` 铁律③：三陷阱 body 正确 |
| user-misc：#43-53（共 11） | 无 WS；HTTP 响应 | `[ ]` 各命令 HTTP 200 |
| presence：`im_get_user_statuses`(#44) | 无 WS；响应 = 批量在线状态（D2 接管，渲染必需）| `[ ]` presence 走 helix 接管通道（非老通道）|
| search 四件套：#50-53 | 无 WS；`search/do` 响应含 channels/users/channelDetails 四 key | `[ ]` userId 服务端注入（前端不传，传了 helix 拒）|
| 空 body 命令：`im_get_all_modules`(#48)/`im_webhook_config_get`(#87) | 无 WS；空 body POST | `[ ]` 空 body 发出且 HTTP 200 |

### 阶段 ⑧ bot-agent（createBot / 调用 → post(bot 变体)）

**前置依赖**：①⑥ 通过（连接 + 至少 1 个 channel）；bot/agent 已配置（config 族）。

| 操作步骤 | 期望结果 | 判据 |
|---|---|---|
| 建 bot：`im_bot_create`(#57) / config 族 #65-74 | 无 WS；HTTP 响应 = 创建结果 | `[ ]` config CRUD HTTP 200 |
| bot 调用（Bot 鉴权）：`im_bot_agent_send_channel`(#77) 等 #75-80 | `auth_kind=Bot`（driver 注 bot 凭据）；落 post → 服务端态推 `post` | `[ ]` 请求头 `X-Auth-Kind: Bot` |
| gateway 回调：`im_bot_callback`(#55) | WS **`post`**(bot 变体，§2.19)；emit `im:post:received` | `[ ]` bot 消息在线渲染 |
| agent 回调：`im_agent_callback`(#85) | WS **`post`**；**不推 cursor**（内存构造，§1.9）；emit `im:post:received`(无 event_seq 推进) | `[ ]` 渲染收到，**勿**断言 cursor 推进 |
| webhook：`im_webhook_config_set/get/test`(#86-88) | 无 WS；HTTP 响应 | `[ ]` webhook 三命令 HTTP 200 |

---

## 3. gate 自检清单（CI-4 收口前过一遍）

- `[ ]` 矩阵 §1 覆盖全部 **89 命令**（计数 §1.7 = 89，机器核对 `outbound_command_count()==88` + send arm）
- `[ ]` **8 阶段齐**（§2 ①-⑧ 全部有前置依赖 + 操作 + 期望 + 判据）
- `[ ]` 每个**写命令的 WS 映射与 `5--backend-ws-events.md` 一致**（§1.8 反查 19 event 全覆盖）
- `[ ]` 三铁律断言点（§0）在阶段 ②③④⑤⑥⑦ 落到具体判据行
- `[ ]` 投影 channel 与 `projection-schema.md` §1 表的 20 channel 逐一对应
- `[ ]` 已知缺口（§1.9：post_pin / change_channel_approval 无 outbound、agent_callback 不推 cursor）已标注
- `[ ]` 本矩阵与方法论 md（`2026-06-23-HTTP-WS接管联调方法论.md`）自洽（三铁律 + 因果模型 + 阶段编排一致）
