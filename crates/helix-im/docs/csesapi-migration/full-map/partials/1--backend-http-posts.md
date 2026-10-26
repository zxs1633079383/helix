# csesapi 「posts」域 HTTP 接口全清单（backend）

> 来源文件：`server/channels/csesapi/posts.go` / `post_approval.go` / `post_bookmark.go`
> base prefix：`/api/cses`（见 `csesapi/api.go` Init() PathPrefix 映射）
> 路由分组（`api.BaseRoutes.*`）→ path 段：
> - `Posts` → `/api/cses/posts`
> - `Post`  → `/api/cses/post`
> - `PostApproval` → `/api/cses/post/approval`
> - `PostBookmark`  → `/api/cses/post/bookmark`
>
> 所有响应统一走 `dto.NewCommonRes()`（CommonRes 信封：含 `success`/`code`/`message`/`data`），下文「响应结构」只列 `data` 内载荷。
> 全域请求体 JSON key 约定为 **camelCase**（个别历史接口用 snake_case，已逐条标注）。

## 接口清单总表

| # | method + path | handler | app 方法 | 触发 WS 事件 | 说明 |
|---|---|---|---|---|---|
| 1 | POST `/api/cses/posts/create` | createPost | CreateCsesPost | post_update（异步，经 Pulsar 落库后） | 发单条消息（走 Pulsar 异步落库） |
| 2 | POST `/api/cses/posts/createPosts` | createPosts | CreateCsesPosts | post_update | 转发/批量发到多个 channel |
| 3 | POST `/api/cses/posts/createSchedule` | createScheduledPost | CreateCsesScheduledMessage | post_schedule_created | 创建定时消息 |
| 4 | POST `/api/cses/posts/cancelSchedule` | cancelScheduledPost | CancelCsesScheduledMessage | post_schedule_canceled | 取消定时消息 |
| 5 | POST `/api/cses/posts/getSchedule` | getScheduledPost | GetCsesScheduledMessage | — | 查询定时消息 |
| 6 | POST `/api/cses/posts/createMock` | createPostMock | （直发 Pulsar MockTopic） | — | 压测/Mock 发消息 |
| 7 | POST `/api/cses/posts/urgentPost` | urgentPost | SetUrgentPost | update_channel ×N（被加急成员）+ post_update | 消息加急 |
| 8 | POST `/api/cses/posts/postContext` | getPostContext | LoadPostContextById | — | 取某消息上下文（前 N 条） |
| 9 | POST `/api/cses/posts/top20` | getTop20 | GetTop20Posts | — | 取频道 top20 消息 |
| 10 | POST `/api/cses/posts/urgentConfirm` | urgentConfirm | ConfirmPostUrgent | update_channel ×N + post_update | 确认收到加急 |
| 11 | POST `/api/cses/posts/urgentCancel` | urgentCancel | CancelPostUrgent | update_channel ×N + post_update | 取消加急 |
| 12 | POST `/api/cses/posts/get` | getPost | LoadPostsByIds | — | 按 postIds 批量取消息 |
| 13 | POST `/api/cses/posts/getPostsAfterIndex` | getPostsAfterIndex | GetPostsAfterIndex | — | 取某消息之后的消息（索引续拉） |
| 14 | POST `/api/cses/posts/revoke` | revoke | RevokePost | post_update | 撤回消息 |
| 15 | POST `/api/cses/posts/getReplies` | getReplies | GetReplies | — | 取某消息的回复列表 |
| 16 | POST `/api/cses/posts/getReplyBranch` | getReplyBranch | GetReplyBranch | — | 取回复分支（按一级回复 id） |
| 17 | POST `/api/cses/posts/makeTopic` | makeTopic | MakeTopic | channel_created + post_update | 消息转话题（建子频道） |
| 18 | POST `/api/cses/posts/queryTodoList` | queryTodoList | QueryTodoList | — | 按 postIds 查 todo 内容 |
| 19 | POST `/api/cses/posts/getUpdatedPosts` | queryUpdatedPosts | QueryUpdatedPosts | — | 增量拉已更新消息（按时间游标） |
| 20 | POST `/api/cses/posts/getLatestPost` | getLatestPost | GetLatestPost | — | 取频道最新消息 |
| 21 | POST `/api/cses/posts/quickReply` | quickReply | QuickReply | post_update | 快捷回复（emoji reaction） |
| 22 | POST `/api/cses/posts/updatePostProps` | updatePostProps | UpdateVotePostProps | post_update | 更新消息 props（投票等） |
| 23 | POST `/api/cses/post/read` | postRead | ReadPost | update_channel（按用户）+ post_read | 标记已读（区间/列表） |
| 24 | POST `/api/cses/post/read/list` | postReadList | ReadPostList | — | 批量查消息已读 bitmap |
| 25 | POST `/api/cses/post/templateReceived` | templateReceived | TemplateReceived | post_update | 模板消息「已收到」 |
| 26 | POST `/api/cses/post/announcement/save` | announcementSave | AnnouncementSave | post_update（视实现） | 公告保存 |
| 27 | POST `/api/cses/post/announcement/read` | announcementRead | AnnouncementRead | post_update（视实现） | 公告确认收到 |
| 28 | POST `/api/cses/post/announcement/acceptList` | AnnouncementAcceptList | AnnouncementAcceptList | — | 公告接受列表 |
| 29 | POST `/api/cses/post/announcement/delete` | AnnouncementDelete | AnnouncementDelete | — | 公告删除 |
| 30 | POST `/api/cses/post/announcement/list` | AnnouncementList | AnnouncementList | — | 公告列表查询 |
| 31 | POST `/api/cses/post/announcement/detail` | AnnouncementDetail | AnnouncementDetail | — | 公告详情（仅公告表） |
| 32 | POST `/api/cses/post/approval/approval` | approvalPost | ApprovePost | （视实现） | 消息审批 |
| 33 | POST `/api/cses/post/bookmark/create` | createPostBookmark | SavePostBookmark | — | 收藏消息 |
| 34 | POST `/api/cses/post/bookmark/delete` | deletePostBookmark | DeletePostBookmark | — | 取消收藏 |
| 35 | POST `/api/cses/post/bookmark/load` | loadPostBookmark | LoadPostBookmark | — | 加载收藏列表（分页） |

> 注：`/posts/search`（searchPost）已在 posts.go L26 注释掉，不可达，不列入。
> 注：PostSegment 3 路由（`/getPostsAfterFromSegment`、`/remove`、`/getStartAndEndTime`）已于 2026-05-27 cutover 注销（channel_event v2 cursor sync 取代），不列入。

---

## 核心数据结构 verbatim

### entity.Post（贯穿全域，嵌套 *ent.Post）
`server/channels/entity/post.go` L39-43：
```go
type Post struct {
	*ent.Post
	Ack           func(isAck bool)       `json:"-" msgpack:"-"`
	TopicPostData map[string]interface{} `json:"topicPostData" msgpack:"topicPostData"`
}
```
> `*ent.Post` 的具体字段（ID/ChannelId/UserId/Type/Message/CreateAt/UpdateAt/Viewers/Mentions/Props/ReadBits/ExpediteMap/TemporaryId/Burst/TopicId/UserSnapshot 等）由 ent 生成，json key 多为 camelCase；`CreateAt`（非 createdAt）。完整定义见 `store/sqlstore/ent/post.go`，由 WS/序列化 agent 交叉核对。

### entity.Posts
`post.go` L278：`type Posts []*Post`

---

## 1. POST `/api/cses/posts/create` — 发单条消息

- **method + path**：POST `/api/cses/posts/create`
- **请求结构 verbatim**（`createPost` L344）：直接 decode 成 `*entity.Post`（顶层即一个 Post 对象，见上方 entity.Post 定义）。
- **响应结构**：CommonRes，无 data（`SetMessage("post sender success")`）。
- **app 方法**：`CreateCsesPost(ctx, *entity.Post)`（`cses_post.go` L743）。内部 `PrePostSend()` 校验 + Pulsar `Publish`（落库 topic）异步落库；真正的 DB 写入与 channel_event append 在 Pulsar consumer 侧。
- **DB 表**：`post`（消息表）、`channel`（lastPost 更新）、`channel_event`（append-only event log，event_type=1 new）。
- **触发 WS 事件**：异步落库后由 consumer 推 `post_update`（`WebsocketEventPostUpdate`）。
- **业务说明**：标准发消息入口，HTTP 仅入队 Pulsar，fire-and-forget。

## 2. POST `/api/cses/posts/createPosts` — 转发/批量发

- **请求结构 verbatim**（`createPosts` L222-225）：
```go
var param struct {
	Posts      *entity.Posts
	ChannelIds []string
}
```
> ⚠️ 无 json tag，默认 Go 字段名首字母大写匹配 → wire key 为 `Posts` / `ChannelIds`（PascalCase，迁移需注意）。`Posts` 即 `[]*entity.Post`。
- **响应**：CommonRes，无 data。
- **app 方法**：`CreateCsesPosts(ctx, *entity.Posts, channelIds []string)`（L885）。多 channel 协程并发落库（注意 commit 1daeb90b 修过共享 ent.Post 数据竞态）。
- **DB 表**：`post`、`channel`、`channel_event`。
- **触发 WS 事件**：`post_update`。
- **业务说明**：一条消息转发到多个 channel。

## 3. POST `/api/cses/posts/createSchedule` — 创建定时消息

- **请求结构 verbatim**（`entity.CreateSchedulePostReq`，`schedule_post.go` L7-10）：
```go
type CreateSchedulePostReq struct {
	Post          *Post `json:"post"`
	ScheduledTime int64 `json:"schedulePostAt"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`CreateCsesScheduledMessage(post *entity.Post, scheduledTime int64)`（L530）。写 Redis ZSET + Pulsar schedule topic。
- **DB 表**：主要 Redis（schedule key，见 `Post.GenerateScheduleRedisKey()`）+ Pulsar；不直接落 post。
- **触发 WS 事件**：`post_schedule_created`（`WebsocketEventPostScheduleCreated`，按 `post.UserId` 单播，L571-573）。

## 4. POST `/api/cses/posts/cancelSchedule` — 取消定时消息

- **请求结构 verbatim**（handler 用 `entity.CancelSchedulePostReq`，`post.go` L491-493）：
```go
type CancelSchedulePostReq struct {
	ChannelId string `json:"channelId"`
}
```
> 注：handler 取 `req.ChannelId` + `c.AppContext.Session().UserId`。
- **响应**：CommonRes，无 data。
- **app 方法**：`CancelCsesScheduledMessage(channelId, userId)`（L581）。删 Redis ZSET 项。
- **触发 WS 事件**：`post_schedule_canceled`（`WebsocketEventPostScheduleCanceled`，按 userId 单播，L593-595）。

## 5. POST `/api/cses/posts/getSchedule` — 查询定时消息

- **请求结构 verbatim**（`entity.GetSchedulePostReq`，`schedule_post.go` L22-25）：
```go
type GetSchedulePostReq struct {
	ChannelId string `json:"channelId"`
	UserId    string `json:"userId"`
}
```
- **响应**：CommonRes data = `*entity.Post`（定时消息体）。
- **app 方法**：`GetCsesScheduledMessage(userId, channelId)`（L599）。读 Redis。
- **WS**：无。

## 6. POST `/api/cses/posts/createMock` — Mock 发消息

- **请求结构 verbatim**（`createPostMock` L514）：decode 成 `*entity.Post`。
- **响应**：CommonRes，无 data。
- **app 方法**：无（handler 直接 `pulsar.GetPulsarClient().Publish` 到 `pulsar.MockTopic`，ProducerTypeBatch）。
- **业务说明**：压测/Mock，绕过 app 层直发 Pulsar。**迁移时可能废弃**。

## 7. POST `/api/cses/posts/urgentPost` — 消息加急

- **请求结构 verbatim**（`entity.UrgentPostVO`，`post.go` L502-509）：
```go
type UrgentPostVO struct {
	ChannelId string   `json:"channelId"`
	PostId    string   `json:"postId"`
	TargetIds []string `json:"targetIds"` // 目标用户ID，可以是用户，也可以是部门领导
	Message *string `json:"message"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`SetUrgentPost(*entity.UrgentPostVO, session)`（L36）→ tx 内 `updateUrgentState`（L188）：`Store().CsesPost().SetPostUrgent` + `Store().CsesChannel().SetPostUrgent` + `Store().Notification().BatchSave` + `Store().NotificationRecipient().BatchSave` + co-tx `AppendEvent`（channel_event event_seq）。
- **DB 表**：`post`、`channel`、`channelmembers`（加急列表/计数）、`notification`、`notification_recipient`、`channel_event`。
- **触发 WS 事件**：
  - `update_channel`（`WebsocketEventUpdateChannel`）逐个被加急 memberId 单播（L78-79 / L106-107，携带 LastEventSeq + phantom UnReadCount 增量）。
  - `post_update`（`WebsocketEventPostUpdate`，`NewWebSocketEventForUsers` 给 userIds，L114-116）。
- **业务说明**：加急 = 设成员被加急列表 + 计数 + 加急状态 + notification 两表落库。

## 8. POST `/api/cses/posts/postContext` — 消息上下文

- **请求结构 verbatim**（`getPostContext` L637-640）：
```go
var param struct {
	PostId string `json:"postId"`
	Before int    `json:"before"`
}
```
- **响应**：CommonRes data = posts（上下文消息列表）。
- **app 方法**：`LoadPostContextById(postId, before, userId)`（cses_post 域外，见 cses_channel/post load 文件）。
- **DB 表**：`post`、`channelmembers`（viewers 鉴权）。
- **WS**：无。

## 9. POST `/api/cses/posts/top20` — Top20 消息

- **请求结构 verbatim**（`getTop20` L550-552）：
```go
var param struct {
	ChannelId string `json:"channel_id"`
}
```
> ⚠️ 此接口 json tag 为 **snake_case** `channel_id`（全域唯一例外，迁移注意）。
- **响应**：CommonRes data = posts。
- **app 方法**：`GetTop20Posts(channelId)`。
- **WS**：无。

## 10. POST `/api/cses/posts/urgentConfirm` — 确认收到加急

- **请求结构 verbatim**（`urgentConfirm` L458-461）：
```go
var param = struct {
	PostId    string `json:"postId"`
	ChannelId string `json:"channelId"`
}{}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`ConfirmPostUrgent(postId, channelId, session)`（L622）→ tx：`Store().CsesChannel().ConfirmPostUrgent` + `Store().Notification().ConfirmUrgent` + `Store().NotificationRecipient().UpdateActionByNotificationId` + co-tx `AppendEvent`，后 `SendUrgentPostAndIncrement`。
- **DB 表**：`post`、`channel`、`notification`、`notification_recipient`、`channel_event`。
- **触发 WS 事件**：`update_channel` ×N + `post_update`（经 `SendUrgentPostAndIncrement` 同 urgentPost 推送路径）。

## 11. POST `/api/cses/posts/urgentCancel` — 取消加急

- **请求结构 verbatim**（`urgentCancel` L486-489）：同 urgentConfirm（`postId`/`channelId`）。
- **app 方法**：`CancelPostUrgent(postId, channelId, session)`（L977）→ tx：`Store().Notification().DeleteByRelationIdAndType` + `Store().CsesChannel().RemoveUrgentPostAndUpdateLast`。
- **DB 表**：`post`、`channel`、`channelmembers`、`notification`、`channel_event`。
- **触发 WS 事件**：`update_channel` ×N + `post_update`（`SendUrgentPostAndIncrement` types.ConfirmUrgent/Cancel 路径）。

## 12. POST `/api/cses/posts/get` — 批量取消息

- **请求结构 verbatim**（`getPost` L613-615）：
```go
var param struct {
	PostIds *[]string `json:"postIds"`
}
```
- **响应**：CommonRes data = posts。
- **app 方法**：`LoadPostsByIds(*[]string)`（L627）。
- **DB 表**：`post`。

## 13. POST `/api/cses/posts/getPostsAfterIndex` — 索引续拉

- **请求结构 verbatim**（`getPostsAfterIndex` L318-320）：
```go
var param = struct {
	PostId string `json:"postIds"`
}{}
```
> ⚠️ 字段名 `PostId`（单数）但 json tag 为 `postIds`（复数），实际是单个 id 字符串，迁移注意此命名陷阱。
- **响应**：CommonRes data = `{ "posts": [...], "count": <int> }`（map）。
- **app 方法**：`GetPostsAfterIndex(postId, session)`（L328，返回 posts + count）。
- **DB 表**：`post`、`channelmembers`。

## 14. POST `/api/cses/posts/revoke` — 撤回消息

- **请求结构 verbatim**（`revoke` L300-302）：
```go
var param = struct {
	PostId string `json:"postId"`
}{}
```
- **响应**：CommonRes，无 data（`SetMessage("revoke post success")`）。
- **app 方法**：`RevokePost(session, postId)`（L326）。软删/置撤回状态 + co-tx AppendEvent（event_type=3 delete/revoke）。
- **DB 表**：`post`、`channel_event`。
- **触发 WS 事件**：`post_update`（`WebsocketEventPostUpdate`，按 channelId 广播，L1574-1576 路径携带撤回 props）。

## 15. POST `/api/cses/posts/getReplies` — 取回复列表

- **请求结构 verbatim**（`getReplies` L281-284）：
```go
var param = struct {
	entity.GetPostOpts
	ReplyId string `json:"replyId"`
}{}
```
> `entity.GetPostOpts`（`post.go` L88-91）= 嵌 `*PageOpts` + `Revoke bool`（分页 + 是否含撤回）。
- **响应**：CommonRes data = `entity.GetRepliesResp`（`post.go` L83-86）：
```go
type GetRepliesResp struct {
	RootPost *ent.Post   `json:"rootPost"`
	Replies  []*ent.Post `json:"replies"`
}
```
- **app 方法**：`GetReplies(replyId, GetPostOpts)`（L291）。
- **DB 表**：`post`。

## 16. POST `/api/cses/posts/getReplyBranch` — 回复分支

- **请求结构 verbatim**（`getReplyBranch` L262-265）：
```go
var param = struct {
	entity.GetPostOpts
	ReplyFirstLevelId string `json:"replyFirstLevelId"`
}{}
```
- **响应**：CommonRes data = 回复分支结果。
- **app 方法**：`GetReplyBranch(replyFirstLevelId, GetPostOpts)`（L272）。
- **DB 表**：`post`。

## 17. POST `/api/cses/posts/makeTopic` — 消息转话题

- **请求结构 verbatim**（`makeTopic` L662-665）：
```go
var param struct {
	PostId string `json:"postId"`
	entity.CreateChannelSpecifyOwner
}
```
> `entity.CreateChannelSpecifyOwner` 字段见 channels 域（建子频道参数）。
> 另读 HTTP header `Cses-Track-Id`（`model.CsesTrackIdHTTPHeader`）透传到 WS envelope.tracing.csesTrackId。
- **响应**：CommonRes，无 data。
- **app 方法**：`MakeTopic(postId, CreateChannelSpecifyOwner, session, csesTrackId)`（L1059）。建子频道 + 关联原 post。
- **DB 表**：`channel`、`channelmembers`、`post`、`channel_event`。
- **触发 WS 事件**：`channel_created`（建频道）+ `post_update`（`WebsocketEventPostUpdate`，L1161-1180，PrecomputeJSON + 根频道 scope + eventSeq/source tag，携带 csesTrackId）。

## 18. POST `/api/cses/posts/queryTodoList` — Todo 内容

- **请求结构 verbatim**（`queryTodoList` L695-697）：
```go
var param struct {
	PostIds []string `json:"postIds"`
}
```
- **响应**：CommonRes data = todoList（`[]*entity.TodoPost`，`post.go` L93-96，嵌 *ent.Post + `MessageType string json:"messageType"`）。
- **app 方法**：`QueryTodoList(postIds, session)`（L710）。
- **DB 表**：`post`、`channelmembers`。

## 19. POST `/api/cses/posts/getUpdatedPosts` — 增量拉更新消息

- **请求结构 verbatim**（`queryUpdatedPosts` L243-246）：
```go
var param = struct {
	CursorTs int64 `json:"timeStamp"`
	Limit    int   `json:"limit"`
}{}
```
> ⚠️ json tag `timeStamp`（camelCase 变体，注意大小写 S）。
- **响应**：CommonRes data = 更新后的 posts 列表。
- **app 方法**：`QueryUpdatedPosts(userId, cursorTs, limit)`（L253）。
- **DB 表**：`post`。

## 20. POST `/api/cses/posts/getLatestPost` — 最新消息

- **请求结构 verbatim**（`getLatestPost` L202-205）：
```go
var param struct {
	ChannelId string `json:"channelId"`
	Timestamp int64  `json:"timestamp"`
}
```
> 注：handler 仅用 `param.ChannelId` + session userId，`Timestamp` 未用。
- **响应**：CommonRes data = posts。
- **app 方法**：`GetLatestPost(channelId, userId)`（L212）。
- **DB 表**：`post`、`channel`（lastPost）。

## 21. POST `/api/cses/posts/quickReply` — 快捷回复

- **请求结构 verbatim**（`quickReply` L182-186）：
```go
var param struct {
	PostId string `json:"postId"`
	UserId string `json:"userId"`
	Emoji  string `json:"emoji"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`QuickReply(postId, userId, emoji, session)`（L1464）。
- **DB 表**：`post`（reaction props 或独立 reaction）、`channel_event`。
- **触发 WS 事件**：`post_update`（L1747-1748，按 channelId 广播 post.ToMap()）。

## 22. POST `/api/cses/posts/updatePostProps` — 更新消息 props

- **请求结构 verbatim**（`updatePostProps` L751-754）：
```go
var param struct {
	PostId string                 `json:"postId"`
	Props  map[string]interface{} `json:"props"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`UpdateVotePostProps(postId, props)`（L1799）。
- **DB 表**：`post`、`channel_event`。
- **触发 WS 事件**：`post_update`（L1820-1821，按 channelId 广播）。
- **业务说明**：主要用于投票消息 props 更新。

## 23. POST `/api/cses/post/read` — 标记已读

- **请求结构 verbatim**（`entity.PostRead`，`post.go` L527-536）：
```go
type PostRead struct {
	ChannelID string `json:"channelId,omitempty"`
	EndTime *int64   `json:"endTime,omitempty"`
	Posts   []string `json:"posts,omitempty"`
	StartTime *int64  `json:"startTime,omitempty"`
	UserId    *string `json:"userId,omitempty"`
}
```
> 注：`UserId` 为 nil 时 handler 用 session userId 兜底（L577-579）。支持「按 posts 列表」或「按 channelId + 时间区间」两种已读模式。
- **响应**：CommonRes，无 data。
- **app 方法**：`ReadPost(entity.PostRead)`（L682）。
- **DB 表**：`post`（ReadBits bitmap）、`channelmembers`（last_read_seq / phantom_count）、`channel_event`。
- **触发 WS 事件**：
  - `update_channel`（`WebsocketEventForUsers`，给 `*postRead.UserId`，L700-701，携带 updateResultPost）。
  - `post_read`（`WebsocketEventPostRead`，给 userIDs，L726-730）。

## 24. POST `/api/cses/post/read/list` — 批量查已读 bitmap

- **请求结构 verbatim**（`entity.PostReadListParam`，`post.go` L554-556）：
```go
type PostReadListParam struct {
	PostIds []string `json:"postIds"`
}
```
- **响应**：CommonRes data = `[]*entity.PostReadDto`（`post.go` L558-561）：
```go
type PostReadDto struct {
	PostId   string
	ReadBits *types.BitMap
}
```
> ⚠️ `PostReadDto` 无 json tag → wire key 为 `PostId` / `ReadBits`（PascalCase）。
- **app 方法**：`ReadPostList(*entity.PostReadListParam)`（L712）。
- **DB 表**：`post`。

## 25. POST `/api/cses/post/templateReceived` — 模板「已收到」

- **请求结构 verbatim**（`templateReceived` L722-724）：
```go
var param struct {
	PostId string `json:"postId"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`TemplateReceived(postId, userId)`（L1753）。
- **DB 表**：`post`、`channel_event`。
- **触发 WS 事件**：`post_update`（L1747-1748，按 channelId 广播 post.ToMap()）。

## 26. POST `/api/cses/post/announcement/save` — 公告保存

- **请求结构 verbatim**（`announcementSave` L77）：decode 成 `*entity.Post`（校验 `ChannelId`/`UserId`/`Type` 非空）。
- **响应**：CommonRes data = `{ "announcementId": <string> }`。
- **app 方法**：`AnnouncementSave(ctx, *entity.Post, session)`（L1648，返回 announcementID）。
- **DB 表**：`post`、`announcement`（公告表）、`channel_event`。
- **触发 WS 事件**：`post_update`（视实现，L1794-1795 announcement 路径按 channelId 广播）。

## 27. POST `/api/cses/post/announcement/read` — 公告确认收到

- **请求结构 verbatim**（`announcementRead` L58-61）：
```go
var param struct {
	PostId    string `json:"postId"`
	ChannelId string `json:"channelId"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`AnnouncementRead(ctx, channelId, postId, session)`（L1619）。
- **DB 表**：`announcement`、`post`、`channel_event`。
- **触发 WS 事件**：`post_update`（视实现）。

## 28. POST `/api/cses/post/announcement/acceptList` — 公告接受列表

- **请求结构 verbatim**（`AnnouncementAcceptList` L102-104）：
```go
var param struct {
	PostId string `json:"postId"`
}
```
- **响应**：CommonRes data = acceptList。
- **app 方法**：`AnnouncementAcceptList(ctx, postId)`。
- **DB 表**：`announcement`、`announcement_props`（接受记录）。

## 29. POST `/api/cses/post/announcement/delete` — 公告删除

- **请求结构 verbatim**（`AnnouncementDelete` L120-122）：
```go
var param struct {
	PostIds []string `json:"postIds"`
}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`AnnouncementDelete(postIds)`。
- **DB 表**：`announcement`。

## 30. POST `/api/cses/post/announcement/list` — 公告列表查询

- **请求结构 verbatim**（`AnnouncementList` L138-141）：
```go
var param struct {
	ChannelId string `json:"channelId"`
	PostId    string `json:"postId"`
}
```
> 注：`ChannelId` 空时回退用 `PostId` 当 channelID（L147-150）。
- **响应**：CommonRes data = postList。
- **app 方法**：`AnnouncementList(ctx, channelID)`。
- **DB 表**：`announcement`、`post`。

## 31. POST `/api/cses/post/announcement/detail` — 公告详情

- **请求结构 verbatim**（`AnnouncementDetail` L162-164）：
```go
var param struct {
	PostIds []string `json:"postIds"`
}
```
- **响应**：CommonRes data = announcementRows（仅公告表）。
- **app 方法**：`AnnouncementDetail(ctx, postIds)`。
- **DB 表**：`announcement`。

## 32. POST `/api/cses/post/approval/approval` — 消息审批

- **来源**：`post_approval.go` L11 + L14。
- **请求结构 verbatim**（`approvalPost` L16-18）：
```go
param := &struct {
	PostID string `json:"postId"`
}{}
```
- **响应**：CommonRes，无 data（`SetSuccess()`）。
- **app 方法**：`ApprovePost(postId, session)`（`cses_post.go` L936）。
- **DB 表**：`post`、`csespostapproval`（审批记录，见 entity.CsesPostApproval）、`channel_event`。
- **触发 WS 事件**：视实现（审批通过常伴 `post_update`）。

## 33. POST `/api/cses/post/bookmark/create` — 收藏消息

- **来源**：`post_bookmark.go` L12 + L54。
- **请求结构 verbatim**（`createPostBookmark` L55-59）：
```go
param := &struct {
	ChannelId string   `json:"channelId"`
	UserId    string   `json:"userId"`
	PostIds   []string `json:"postIds"`
}{}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`SavePostBookmark(channelId, userId, postIds)`。
- **DB 表**：`post`（书签/收藏标记表，CSES fork 命名注意）。
- **WS**：无。

## 34. POST `/api/cses/post/bookmark/delete` — 取消收藏

- **请求结构 verbatim**（`deletePostBookmark` L37-40）：
```go
param := &struct {
	UserId string `json:"userId"`
	PostId string `json:"postId"`
}{}
```
- **响应**：CommonRes，无 data。
- **app 方法**：`DeletePostBookmark(userId, postId)`。
- **DB 表**：书签/收藏表。

## 35. POST `/api/cses/post/bookmark/load` — 加载收藏列表

- **请求结构 verbatim**（`loadPostBookmark` L18-22）：
```go
param := &struct {
	ChannelId string `json:"channelId"`
	UserId    string `json:"userId"`
	entity.PageOpts
}{}
```
> `entity.PageOpts`（分页参数，见 `entity/page.go`）。
- **响应**：CommonRes data = posts（收藏消息列表）。
- **app 方法**：`LoadPostBookmark(channelId, userId, PageOpts)`。
- **DB 表**：书签/收藏表 + `post`。

---

## 迁移注意点（给目标后端 cses-im-server）

1. **大小写约定不一致**（迁移需逐条对齐）：
   - 全域多数 camelCase，但 `/top20` 用 snake_case `channel_id`；`createPosts` 请求体无 tag = PascalCase（`Posts`/`ChannelIds`）；`PostReadDto` 响应无 tag = PascalCase（`PostId`/`ReadBits`）。
2. **命名陷阱**：`getPostsAfterIndex` 字段名 `PostId`（单数）json tag `postIds`（复数），实为单 id。
3. **CreateAt（非 createdAt）**：ent.Post 时间字段 wire key 是 `createAt`，符合 IM-Sync wire 契约（见 MEMORY im_sync_wire_contract）。
4. **channel_event co-tx 铁律**：所有 mutation（create/revoke/urgent/read/template/quickReply/updateProps/approval/announcement）必须同事务 AppendEvent（event_type 1=new/2=edit/3=delete），离线 sync v2 依赖。
5. **Pulsar 异步落库**：`/create`、`/createPosts` 走 Pulsar consumer 落库，HTTP 仅入队；WS push 在 consumer 侧。
6. **createMock 可废弃**：纯压测旁路，迁移可不带。
7. **AnnouncementSave 入参是裸 *entity.Post**（嵌 *ent.Post），公告复用 post 表 + 独立 announcement 表。

> WS 事件清单（供 WS 追踪 agent 交叉核对）：`post_update`、`post_read`、`update_channel`、`post_schedule_created`、`post_schedule_canceled`、`channel_created`。
> 事件常量定义见 `server/public/model/websocket_message.go`（L32 post_read / L138 post_schedule_created / L139 post_schedule_canceled / L141 update_channel / L142 post_update）。
