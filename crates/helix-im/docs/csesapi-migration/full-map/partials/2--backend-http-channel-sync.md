# csesapi 后端 HTTP 接口清点 — channel 域

> 源文件: `server/channels/csesapi/channel.go`(882行) + `server/channels/csesapi/sync.go`(105行)
> Init 注册: `InitChannel()` (channel.go:15-52) + `InitSync()` (sync.go:26-28)
> Base prefix: `/api/cses`（`api.go:167` APIRoot = `model.APIURLSuffixCSES`）
> 子路由前缀（api.go:176-186）:
> - `BaseRoutes.Channels`       = `/api/cses/channels`
> - `BaseRoutes.Channel`        = `/api/cses/channel`
> - `BaseRoutes.ChannelMembers` = `/api/cses/channel/members`（本域当前无活跃注册，仅注释掉的 DELETE）
> - `BaseRoutes.ChannelMember`  = `/api/cses/channel/member`
> - `BaseRoutes.Sync`           = `/api/cses/channel/sync`
>
> 所有 handler 走 `api.APIHandler(...)`（统一鉴权 + 错误封装），全部 `Methods("POST")`。
> 响应统一信封 `dto.NewCommonRes()`：`{ status, message, data }`（status 为字符串枚举 `"SUCCESS"`/`"FAILURE"`，**非** int）。
> App 方法全部落在 `server/channels/app/cses_channel.go`（2314行，唯一聚合文件）。
> Store 表（ent，CSES fork 命名，单数）：`channel` / `channelmember` / `channelmemberhistory` / `channel_event` / `channelmembersnapshot` / `post` / `channelnotice` / `postpinned` / `channeltopic`。

## 接口清单表

| # | Method + Path | Handler | App 方法 | 主要 DB 表 | 触发 WS 事件 |
|---|---|---|---|---|---|
| 1 | POST `/api/cses/channels/load/increment` | loadIncrementChannel | LoadIncrementChannels → RunIncrementWorker(goroutine) | channel, channelmember, channelmemberhistory, channel_event, postpinned, channeltopic | increment_channel / increment_channel_end |
| 2 | POST `/api/cses/channel/load/incrementByChannelId` | loadSingleIncrementChannel | LoadSingleIncrementChannel | channel, channelmember, channelmemberhistory, channel_event, postpinned | (无，HTTP 同步直返) |
| 3 | POST `/api/cses/channels/enableApproval` | changeApprovalStatus | ChangeApprovalStatus | channel | change_channel_approval |
| 4 | POST `/api/cses/channels/view` | viewChannel | CsesViewChannels | channel | (无) |
| 5 | POST `/api/cses/channels/member/byIds` | loadChannelMembersByIds | LoadChannelMembersByChannelIds | channelmember | (无) |
| 6 | POST `/api/cses/channel/member/snapshot` | getMembersSnapshot | GetMembersSnapshot | channelmembersnapshot (via CsesPost.GetMembersSnapshotByRange) | (无) |
| 7 | POST `/api/cses/channel/create` | createChannel | CreateCsesChannel(owner=nil) | channel, channelmember, channelmemberhistory, post | channel_created / channel_member_update / post |
| 8 | POST `/api/cses/channel/createSpecifyOwner` | createSpecifyOwner | CreateCsesChannel(owner=param.Owner) | channel, channelmember, channelmemberhistory, post | channel_created / channel_member_update / post |
| 9 | POST `/api/cses/channel/change/permission` | changeChannelPermission | ChangeChannelPermission | channel | update_channel |
| 10 | POST `/api/cses/channel/change/notice` | changeChannelNotice | ChangeChannelNotice | channelnotice, channel | update_channel_notice |
| 11 | POST `/api/cses/channel/change/info` | changeChannelInfo | ChangeCsesChannelInfo | channel | update_channel |
| 12 | POST `/api/cses/channel/change/source` | updateSource | UpdateCsesChannelSource | channel | update_channel |
| 13 | POST `/api/cses/channel/change/displayName` | UpdateCsesChannelDisPlayName | UpdateCsesChannelDisPlayName | channel | update_channel |
| 14 | POST `/api/cses/channel/change/picture` | updatePicture | UpdateCsesChannelPicture | channel | update_channel |
| 15 | POST `/api/cses/channel/change/props` | updateProps | UpdateCsesChannelProps | channel | (无 Publish) |
| 16 | POST `/api/cses/channel/change/orient` | updateOrient | UpdateCsesChannelOrient | channel | update_channel |
| 17 | POST `/api/cses/channel/change/purpose` | updatePurpose | UpdateCsesChannelPurpose | channel | update_channel |
| 18 | POST `/api/cses/channel/change/top` | updateChannelTop | UpdateCsesChannelTop | channelmember | update_channel |
| 19 | POST `/api/cses/channel/add/manger` | addChannelManger | AddCsesChannelManger | channelmember | (Publish 已注释；仅 GrpcInvoke 对端) |
| 20 | POST `/api/cses/channel/remove/manger` | deleteChannelManger | DeleteCsesChannelManger | channelmember | (Publish 已注释；仅 GrpcInvoke 对端) |
| 21 | POST `/api/cses/channel/add/postPinned` | addChannelPostPinned | PinnedCsesChannelPost | postpinned, post | post_pin |
| 22 | POST `/api/cses/channel/remove/postPinned` | deleteChannelPostPinned | DeleteCsesChannelPostPinned | postpinned | post_pin |
| 23 | POST `/api/cses/channel/load/notice` | loadChannelNotice | LoadChannelNotice | channelnotice | (无) |
| 24 | POST `/api/cses/channel/load/postPinned` | loadChannelPostPinned | LoadPinnedCsesChannelPost | postpinned, post | (无) |
| 25 | POST `/api/cses/channel/load/admin` | loadChannelAdmin | LoadChannelAdmin | channelmember | (无) |
| 26 | POST `/api/cses/channel/query` | channelQuery | ChannelQueryByCondition | channel | (无) |
| 27 | POST `/api/cses/channel/close` | channelClose | ChannelClose | channel | channel_close |
| 28 | POST `/api/cses/channel/onlineStatus` | channelOnlineStatus | ChannelOnlineStatus | channelmember (+ status / 在线判定) | (无) |
| 29 | POST `/api/cses/channel/member/change/role` | updateChannelMemberRole | UpdateCsesChannelMemberRoles | channelmember | channel_member_role_updated |
| 30 | POST `/api/cses/channel/member/change/notify` | updateNotify | UpdateCsesChannelMemberNotify | channelmember | update_channel |
| 31 | POST `/api/cses/channel/member/change/nickname` | updateChannelMemberNickname | UpdateCsesChannelMemberNickname | channelmember | update_channel_member_nickName |
| 32 | POST `/api/cses/channel/member/change` | channelMemberChange | ChannelMemberChange | channelmember, channelmemberhistory, post | channel_member_update / post / post_update |
| 33 | POST `/api/cses/channel/member/leave` | channelMemberLeave | ChannelMemberLeave | channelmember, channelmemberhistory, post | channel_member_update / post |
| 34 | POST `/api/cses/channel/sync/notify` | syncChannels | SyncChannels | channel_event, post, channelmember (membership 闸门) | (无，HTTP 同步直返) |

---

## 接口详情

### 1. POST /api/cses/channels/load/increment — 增量频道加载（异步推送）

- **App 方法**: `LoadIncrementChannels(ctx, session, param)` → 启 goroutine `RunIncrementWorker`（cses_channel.go:39-241）。HTTP 立即返回 success，结果走 WS 推送。
- **DB 表**: `channel`(LoadIncrementChannels) / `channelmember`(GetChannelMemberMapAndChannelIds, LoadAdminMemberIds, LoadChannelMemberCounts) / `channelmemberhistory`(LoadSinceUserJoinAndLeaveByChannelIds) / `channel_event`(GetMaxSeq → fillLastEventSeq) / `postpinned`(LoadChannelPinnedCount) / `channeltopic`(loadTopicChannelsAndMap)。
- **请求结构 verbatim**（`entity.IncrementChannelParam`，channel.go:22-35）:
```go
type IncrementChannelParam struct {
	TimeStamp *int64  `json:"timestamp"`
	ChannelId *string `json:"ChannelId"`        // 注意: PascalCase 的 ChannelId（异常约定）
	Cursors []SyncCursorLite `json:"cursors,omitempty"`
}
type SyncCursorLite struct {
	ChannelID string `json:"channelId"`
	FromSeq   int64  `json:"fromSeq"`
}
```
- **响应结构**: `dto.CommonRes` 仅 `{status:"SUCCESS", message:"loadIncrementChannel success"}`（无 data）。真实数据通过 WS `increment_channel` 分片推送，结束发 `increment_channel_end`。
- **触发 WS 事件**: `increment_channel`（NewWebSocketEventForUsers，data 为 IncrementChannel slice 分片，PublishSlice，cses_channel.go:224-231）、`increment_channel_end`（cses_channel.go:100/123）。
- **业务说明**: 客户端冷启/重连拉全量频道元数据 + per-channel last_event_seq + 未读 + 成员变更 + 置顶计数。`Cursors` 用于服务端 stamp `needSync`（根群链路）。`ctx` 必须透传以保 trace + companyId 给下游 GrpcInvoke。

### 2. POST /api/cses/channel/load/incrementByChannelId — 单频道增量同步（HTTP 同步直返）

- **App 方法**: `LoadSingleIncrementChannel(ctx, session, channelId)` (cses_channel.go:244-337)。
- **DB 表**: `channel`(LoadIncrementChannelByID) / `channelmember`(LoadMember, LoadAdminMemberIds, LoadChannelMemberCounts) / `channelmemberhistory` / `channel_event`(GetMaxSeq) / `postpinned`。
- **请求结构 verbatim**（handler 内联，channel.go:169-171）:
```go
var param struct {
	ChannelId string `json:"channelId"`
}
```
- **响应结构**: `dto.CommonRes.data = *entity.IncrementChannel`（单条），不走 WS。
- **触发 WS 事件**: 无（HTTP 直返 data）。
- **业务说明**: 单 channel 同步查询，复用与 increment 相同的 IncrementChannel 结构，HTTP 直接返回不推送。

### 3. POST /api/cses/channels/enableApproval — 开启/关闭群审批

- **App 方法**: `ChangeApprovalStatus(session, channelId, *param.ApprovalStatus)` (cses_channel.go:524)。返回 `*model.AppError`。
- **DB 表**: `channel`（ApprovalEnabled 列）。
- **请求结构 verbatim**（`entity.EnableApprovalParam`，channel.go:542-545）:
```go
type EnableApprovalParam struct {
	ChannelId      string `json:"channelId"`
	ApprovalStatus *bool  `json:"approvalStatus"`
}
```
- **响应结构**: `dto.CommonRes` 仅 status/message（无 data）。
- **触发 WS 事件**: `change_channel_approval`（cses_channel.go:1073-1082，NewWebSocketEvent broadcast 到 channelId）。
- **业务说明**: 修改群审批开关并广播。注意 handler 直接 `*param.ApprovalStatus` 解引用，未对 nil 做防御。

### 4. POST /api/cses/channels/view — 标记已读/查看频道

- **App 方法**: `CsesViewChannels(params, session.UserId)` (cses_channel.go:482)。
- **DB 表**: `channel`（ViewChannels）。
- **请求结构 verbatim**（`entity.ViewChannels`，view_channel.go）:
```go
type ViewChannel struct {
	Id     string `json:"id"`
	IsRoot *bool  `json:"isRoot"`
}
type ViewChannels struct {
	Channels []*ViewChannel `json:"channels"`
}
```
- **响应结构**: status/message。
- **触发 WS 事件**: 无。
- **业务说明**: 批量标记频道已查看。handler 内对每个 channel：`Id==""` 返回错误，`IsRoot==nil` 默认置 true（channel.go:221-229）。

### 5. POST /api/cses/channels/member/byIds — 批量按 channelIds 拉成员

- **App 方法**: `LoadChannelMembersByChannelIds(channelIds, session)` (cses_channel.go:2051)。
- **DB 表**: `channelmember`。
- **请求结构 verbatim**（handler 内联，channel.go:81-83）:
```go
var param struct {
	ChannelIds []string `json:"channelIds"`
}
```
- **校验**: 空 → 失败；`len > 200` → 失败（channel.go:89-96）。
- **响应结构**: `dto.CommonRes.data = map[string][]*entity.IdWithCompanyExt`（按 channelId 分组，每项 `{userId, teamId, role}`）。
- **触发 WS 事件**: 无。
- **业务说明**: 鉴权过滤——仅返回 session 用户实际所属的群；非成员 channelId 被丢弃。

### 6. POST /api/cses/channel/member/snapshot — 成员快照（时间范围）

- **App 方法**: `GetMembersSnapshot(channelId, startTime, endTime)` → `Store().CsesPost().GetMembersSnapshotByRange(...)` (cses_channel.go:520-521)。
- **DB 表**: `channelmembersnapshot`。
- **请求结构 verbatim**（`entity.GetMembersSnapshotParam`，channel_members_snapshot.go:107-111）:
```go
type GetMembersSnapshotParam struct {
	ChannelId string `json:"channelId"`
	StartTime *int64 `json:"startTime"`
	EndTime   *int64 `json:"endTime"`
}
```
- **校验**: ChannelId 非空 + StartTime 非 nil + EndTime 非 nil（Validate）。
- **响应结构 verbatim**（`[]*entity.GetMembersSnapshotDto`，via `snapshots.GetDto()`，channel_members_snapshot.go:153-157）:
```go
type GetMembersSnapshotDto struct {
	UserIds   *pq.StringArray `json:"userIds,omitempty"`
	StartTime int64           `json:"startTime,omitempty"`
	EndTime   int64           `json:"endTime,omitempty"`
}
```
- **触发 WS 事件**: 无。
- **业务说明**: 返回某 channel 在时间窗内的成员名单快照序列（用于历史回放成员组成）。

### 7. POST /api/cses/channel/create — 创建群聊（owner=当前用户）
### 8. POST /api/cses/channel/createSpecifyOwner — 创建群聊（指定 owner）

- **App 方法**: 两者共用 `CreateCsesChannel(session, param, ownerResult, sendPost=true, trackId)` (cses_channel.go:560-823)。
  - `/create`: `ownerResult=nil`（owner 即 session 用户，channel.go:283）。
  - `/createSpecifyOwner`: `ownerResult=param.Owner`（channel.go:69）。
- **DB 表**: `channel`(CreateChannel) / `channelmember`(随 CreateChannel 写) / `channelmemberhistory`(CreateChannelMemberHistoryMultiple) / `post`(系统消息 createChannelPostProp/Local) / `channeltopic`(话题校验)。
- **请求结构 verbatim**（`entity.CreateChannelSpecifyOwner`，create_channel.go:20-25）:
```go
type CreateChannelSpecifyOwner struct {
	*ent.Channel
	Users       []*model.ChannelMemberMinimal `json:"users"`
	ForceCreate *bool                         `json:"forceCreate"`
	Owner       *model.ChannelMemberMinimal   `json:"owner"`
}
```
  内嵌 `*ent.Channel`（关键字段 verbatim，ent/channel.go）:
```go
ID          string  `json:"id,omitempty"`
Name        *string `json:"name,omitempty"`
Type        *types.ChannelType `json:"type,omitempty"`     // "D"=私聊须 2 人
DisplayName *string `json:"displayName,omitempty"`
Picture     map[string]interface{} `json:"picture,omitempty"`
PictureType *types.ChannelPictureType `json:"pictureType,omitempty"`
Header      *string `json:"header,omitempty"`
Purpose     *string `json:"purpose,omitempty"`
Orient      *string `json:"orient,omitempty"`
Module      *string `json:"module,omitempty"`
TeamId      *string `json:"teamId,omitempty"`
Source      map[string]interface{} `json:"source,omitempty"`
Props       map[string]interface{} `json:"props,omitempty"`
DirectKey   *string `json:"direct_key,omitempty"`           // 注意 snake_case direct_key（异常）
ParentPostId string `json:"parentPostId,omitempty"`
```
  `model.ChannelMemberMinimal`（channel.go:168-173）:
```go
type ChannelMemberMinimal struct {
	Id     string `json:"id"`
	Name   string `json:"name"`
	Role   string `json:"role"`
	TeamId string `json:"teamId"`
}
```
- **校验**（Validate）: Channel 非 nil + Users 非 nil 且 ≥1；Type=="D" 须恰 2 人。
- **响应结构**: `dto.CommonRes.data = *entity.Channel`（结果频道）。
- **触发 WS 事件**: `channel_created`（cses_channel.go:715-718，data=IncrementChannel.ToMap()，带 SetCsesTrackId）；`channel_member_update`（cses_channel.go:655-656）；`post`（系统加群消息，cses_channel.go:874-875）。
- **业务说明**: 私聊去重（IsExistPrivateChannelInMember）。透传 `Cses-Track-Id` HTTP header（`model.CsesTrackIdHTTPHeader`）→ App → channel_created envelope.tracing.csesTrackId。

### 9. POST /api/cses/channel/change/permission — 群权限范围更改

- **App 方法**: `ChangeChannelPermission(param, session)` (cses_channel.go:1847-1910)。
- **DB 表**: `channel`（MentionPermission/TopPermission/NoticePermission 列）。
- **请求结构 verbatim**（`command.ChangeChannelPermission`，command/channel.go:10-16）:
```go
type ChangeChannelPermission struct {
	ChannelId         string                 `json:"channelId"`
	MentionPermission *types.ChannelRoleType `json:"mentionPermission"`
	TopPermission     *types.ChannelRoleType `json:"topPermission"`
	NoticePermission  *types.ChannelRoleType `json:"noticePermission"`
	UserId            string                 `json:"-"`        // session 注入，不读 body
}
```
- **校验**: `ChannelId` 非空。`UserId` 由 handler 用 `session.UserId` 覆盖（channel.go:301）。
- **响应结构**: status/message。
- **触发 WS 事件**: `update_channel`（cses_channel.go:512-515 / 2024-2025）。
- **业务说明**: 改 @ / 置顶 / 公告三类权限角色并广播频道更新。

### 10. POST /api/cses/channel/change/notice — 修改群公告

- **App 方法**: `ChangeChannelNotice(param)` (cses_channel.go:1108)。
- **DB 表**: `channelnotice`(写) + `channel`(关联)。
- **请求结构 verbatim**（`command.ChangeChannelNotice`，command/channel.go:40-44）:
```go
type ChangeChannelNotice struct {
	ID     string          `json:"id"`      // channelId
	Notice *map[string]any `json:"notice"`
	UserId *string         `json:"-"`       // session 注入
}
```
- **响应结构**: status/message。
- **触发 WS 事件**: `update_channel_notice`（cses_channel.go:1190-1191，data=channelNotice.ToMap()）。可能伴随 `post`（公告系统消息，cses_channel.go:1089/1103）。
- **业务说明**: handler 用 session.UserId 覆盖 UserId（channel.go:348）。

### 11. POST /api/cses/channel/change/info — 修改群信息（聚合）

- **App 方法**: `ChangeCsesChannelInfo(cmd)` (cses_channel.go:2004-2028)。
- **DB 表**: `channel`。
- **请求结构 verbatim**（`command.ChangeChannelInfo`，command/channel.go:72-93）:
```go
type ChangeChannelInfo struct {
	Name        *string `json:"name,omitempty"`
	ChannelId   string  `json:"channelId"`
	DisplayName *string `json:"displayName,omitempty"`
	Picture     map[string]interface{} `json:"picture,omitempty"`
	PictureType *types.ChannelPictureType `json:"pictureType,omitempty"`
	Header      *string `json:"header,omitempty"`
	Purpose     *string `json:"purpose,omitempty"`
	Orient      *string `json:"orient,omitempty"`
	Module      *string `json:"module,omitempty"`
	Source      map[string]interface{} `json:"source,omitempty"`
	UserId      *string                `json:"-"`
}
```
- **校验**: `ChannelId` 非空。UserId 由 handler 用 session 覆盖（channel.go:331）。
- **响应结构**: status/message。
- **触发 WS 事件**: `update_channel`（cses_channel.go:2024-2025，data=result.ToMap()）。
- **业务说明**: 一次性聚合改名/头像/公告栏/简介/导向/模块/来源。

### 12. POST /api/cses/channel/change/source — 修改频道来源标题

- **App 方法**: `UpdateCsesChannelSource(cmd)` (cses_channel.go:1251)。返回 `*model.AppError`。
- **DB 表**: `channel`(Source 列)。
- **请求结构 verbatim**（`command.ChangeChannelSource`，command/channel.go:46-50）:
```go
type ChangeChannelSource struct {
	ID     string  `json:"id"`
	Title  string  `json:"title"`
	UserId *string `json:"-"`
}
```
- **校验**: `cmd.ID != ""`（handler，channel.go:471）。
- **响应结构**: status（success 无 data）。
- **触发 WS 事件**: `update_channel`（cses_channel.go:1270-1272）。

### 13. POST /api/cses/channel/change/displayName — 修改群展示名

- **App 方法**: `UpdateCsesChannelDisPlayName(cmd, session)` (cses_channel.go:1276)。
- **DB 表**: `channel`(DisplayName 列)。
- **请求结构 verbatim**（`command.ChangeChannelDisPlayName`，command/channel.go:52-56）:
```go
type ChangeChannelDisPlayName struct {
	ID          string  `json:"id"`
	DisPlayName string  `json:"displayName"`     // 字段名 DisPlayName，json key displayName
	UserId      *string `json:"-"`
}
```
- **校验**: `cmd.ID != ""`。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel`（cses_channel.go:1300-ish，via 1245-1247/1270-1272 路径）。

### 14. POST /api/cses/channel/change/picture — 修改群头像

- **App 方法**: `UpdateCsesChannelPicture(pictureType, picture, channelId, userId)` (cses_channel.go:1194)。返回 `*model.AppError`。
- **DB 表**: `channel`(Picture/PictureType 列)。
- **请求结构 verbatim**: 直接解码到 `*entity.Channel`（= `*ent.Channel`，channel.go:425-426），读取 `PictureType` / `Picture` / `ID` 字段（见 #7-8 内嵌 ent.Channel 定义）。
- **校验**: `PictureType==nil || Picture==nil` → 失败（channel.go:432）。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel`（cses_channel.go:1220-1222，data=ch.ToMap()）。

### 15. POST /api/cses/channel/change/props — 修改群扩展属性

- **App 方法**: `UpdateCsesChannelProps(channel.Props, channel.ID, userId)` (cses_channel.go:1226-1250)。
- **DB 表**: `channel`(Props 列)。
- **请求结构 verbatim**: 解码到 `entity.Channel`（=`ent.Channel`），用 `Props map[string]interface{}` + `ID`（channel.go:450-452）。
- **校验**: `channel.ID != ""`。
- **响应结构**: status。
- **触发 WS 事件**: 此方法在 1226-1250 区间未见 a.Publish（无广播）。
- **业务说明**: 仅落库 Props，不推送（与其他 change/* 不同）。

### 16. POST /api/cses/channel/change/orient — 修改群导向

- **App 方法**: `UpdateCsesChannelOrient(orient, channelId, session)` (cses_channel.go:1361)。
- **DB 表**: `channel`(Orient 列)。
- **请求结构 verbatim**（`command.UpdateChannelGuidePurposeCommand`，command/channel.go:58-63）:
```go
type UpdateChannelGuidePurposeCommand struct {
	ID      string  `json:"id"`
	Orient  *string `json:"orient"`
	Purpose *string `json:"purpose"`
	UserId  *string `json:"-"`
}
```
- **校验**: `ID != ""`（Validate）。handler 用指针解码并防 nil（channel.go:376-382）。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel`（cses_channel.go 区间内 Publish）。

### 17. POST /api/cses/channel/change/purpose — 修改群简介

- **App 方法**: `UpdateCsesChannelPurpose(purpose, channelId, session)` (cses_channel.go:1403)。
- **DB 表**: `channel`(Purpose 列)。
- **请求结构 verbatim**: 同 #16 `command.UpdateChannelGuidePurposeCommand`。
- **校验**: `ID != ""`。UserId 由 handler 覆盖（channel.go:412）。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel`。

### 18. POST /api/cses/channel/change/top — 频道置顶（per-member）

- **App 方法**: `UpdateCsesChannelTop(cmd)` (cses_channel.go:1665-1713)。
- **DB 表**: `channelmember`(ChannelIsTop 列)。
- **请求结构 verbatim**（`command.UpdateChannelTopCommand`，command/channel.go:102-106）:
```go
type UpdateChannelTopCommand struct {
	ChannelId string  `json:"channelId"`
	Top       *bool   `json:"top"`
	UserId    *string `json:"-"`
}
```
- **校验**: `ChannelId != ""`。UserId 由 handler 覆盖（channel.go:645）。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel`（cses_channel.go:1685-1686，targeted 到 result.UserId，data=topMap）。
- **业务说明**: 当前用户对某频道的置顶标记（个人维度），定向推送给本人。

### 19. POST /api/cses/channel/add/manger — 添加群管理员

- **App 方法**: `AddCsesChannelManger(ctx, cmd, cookieId, session)` (cses_channel.go:1458-1521)。
- **DB 表**: `channelmember`(Role 列)。另有 GrpcInvoke 透传对端 paas。
- **请求结构 verbatim**（`command.AddChannelMangerCommand`，command/channel.go:122-126）:
```go
type AddChannelMangerCommand struct {
	ChannelId string                       `json:"channelId"`
	Users     []model.ChannelMemberMinimal `json:"users"`
	OwnerId   *string                      `json:"-"`
}
```
- **校验**: `ChannelId != ""` 且 `len(Users) > 0`。OwnerId 由 handler 用 session 覆盖（channel.go:597）。读取 `cookieId` HTTP header 传 App。
- **响应结构**: status。
- **触发 WS 事件**: **当前已注释**（cses_channel.go:1514-1516 `WebsocketEventAddManager` 被注掉）→ 无 WS 广播；仅 GrpcInvoke 对端。
- **业务说明**: ⚠️ 迁移注意：原 `add_manager` WS 已停用，目标后端需确认是否恢复或改走 sync。

### 20. POST /api/cses/channel/remove/manger — 移除群管理员

- **App 方法**: `DeleteCsesChannelManger(ctx, cmd, cookieId, session)` (cses_channel.go:1522-1581)。
- **DB 表**: `channelmember`(Role 列)。GrpcInvoke 对端。
- **请求结构 verbatim**（`command.DeleteChannelMangerCommand`，command/channel.go:138-142）:
```go
type DeleteChannelMangerCommand struct {
	ChannelId string                       `json:"channelId"`
	Users     []model.ChannelMemberMinimal `json:"users"`
	OwnerId   *string                      `json:"-"`
}
```
- **校验**: 非 nil + `ChannelId != ""` + `len(Users) > 0`。
- **响应结构**: status。
- **触发 WS 事件**: **当前已注释**（cses_channel.go:1576-1578 `WebsocketEventRemoveManager` 被注掉）→ 无 WS。
- **业务说明**: 同 #19，原 `remove_manager` WS 停用。

### 21. POST /api/cses/channel/add/postPinned — 置顶消息

- **App 方法**: `PinnedCsesChannelPost(cmd, session)` (cses_channel.go:1714-1772)。
- **DB 表**: `postpinned`(写) + `post`(读)。
- **请求结构 verbatim**（`command.ChannelPostPinnedCommand`，command/channel.go:157-162）:
```go
type ChannelPostPinnedCommand struct {
	Id        string  `json:"id"`
	ChannelId string  `json:"channelId"`
	PostId    string  `json:"postId"`
	UserId    *string `json:"-"`
}
```
- **校验**: `ChannelId != ""` 且 `PostId != ""`。UserId 由 handler 覆盖（channel.go:670）。
- **响应结构**: status。
- **触发 WS 事件**: `post_pin`（cses_channel.go:1840-1844，broadcast 到 channelId）。
- **业务说明**: 置顶消息走独立 `postpinned` 表。

### 22. POST /api/cses/channel/remove/postPinned — 取消置顶消息

- **App 方法**: `DeleteCsesChannelPostPinned(cmd, session)` (cses_channel.go:1911-1975)。
- **DB 表**: `postpinned`(删)。
- **请求结构 verbatim**: 同 #21 `command.ChannelPostPinnedCommand`。
- **校验**: handler 未强制（注释掉了 Id 校验，channel.go:690-694）；UserId 覆盖（channel.go:696）。
- **响应结构**: status。
- **触发 WS 事件**: `post_pin`（同 #21 路径推送，表示置顶变更）。

### 23. POST /api/cses/channel/load/notice — 加载群公告

- **App 方法**: `LoadChannelNotice(*param)` (cses_channel.go:1178)。
- **DB 表**: `channelnotice`。
- **请求结构 verbatim**（`command.ChannelIdCommand`，command/channel.go:174-176）:
```go
type ChannelIdCommand struct {
	ChannelId string `json:"channelId"`
}
```
- **响应结构**: `dto.CommonRes.data = *entity.ChannelNoticeSlice`。
- **触发 WS 事件**: 无。

### 24. POST /api/cses/channel/load/postPinned — 加载置顶消息列表

- **App 方法**: `LoadPinnedCsesChannelPost(cmd, &postPinnedRes)` (cses_channel.go:1773)。
- **DB 表**: `postpinned`(读) + `post`(关联)。
- **请求结构 verbatim**: 同 #21 `command.ChannelPostPinnedCommand`（实际只用 ChannelId）。
- **响应结构 verbatim**（`[]*entity.PostPinnedRes`，post.go:78-81）:
```go
type PostPinnedRes struct {
	*ent.Post `json:"post"`     // 内嵌完整 post，ToMap 拍平所有 post 字段
	Operator  string `json:"operator"`
}
```
  注意：实际序列化时 `data` 为 `[]*entity.PostPinnedRes`，每项嵌 `ent.Post`（含 id/createAt/message/viewers/readBits/replyId 等，见 copy_postpinnedres.go ToMap 全字段）。
- **触发 WS 事件**: 无。

### 25. POST /api/cses/channel/load/admin — 加载群管理员

- **App 方法**: `LoadChannelAdmin(cmd)` (cses_channel.go:2029)。
- **DB 表**: `channelmember`（role 过滤管理员）。
- **请求结构 verbatim**（`entity.ChannelIdStruct`，channel.go:609-613）:
```go
type ChannelIdStruct struct {
	ChannelId  string                      `json:"channelId"`
	UserId     *string                     `json:"userId"`
	NewCreator *model.ChannelMemberMinimal `json:"newCreator"`
}
```
- **校验**: `ChannelId != ""`（Validate）。
- **响应结构**: `dto.CommonRes.data = *entity.ChannelMemberEntitySlice`。
- **触发 WS 事件**: 无。

### 26. POST /api/cses/channel/query — 条件分页查询频道

- **App 方法**: `ChannelQueryByCondition(cmd.Channel, cmd.PageOpts, session)` (cses_channel.go:2191)。
- **DB 表**: `channel`。
- **请求结构 verbatim**: handler 内联匿名 struct 嵌入两个 entity（channel.go:804-807）:
```go
cmd := &struct {
	entity.Channel    // = ent.Channel（全字段，见 #7-8）
	entity.PageOpts
}{}
```
  `entity.PageOpts`（common.go:3-7）:
```go
type PageOpts struct {
	PageNumber int `json:"pageNumber"`
	PageSize   int `json:"pageSize"`
	Offset     int `json:"offset"`
}
```
- **响应结构**: `dto.CommonRes.data = []*ent.Channel`。
- **触发 WS 事件**: 无。

### 27. POST /api/cses/channel/close — 关闭频道

- **App 方法**: `ChannelClose(channelId, session)` (cses_channel.go:2199-2220)。
- **DB 表**: `channel`(Close 列)。
- **请求结构 verbatim**（handler 内联，channel.go:826-828）:
```go
var cmd = struct {
	ChannelId string `json:"channelId"`
}{}
```
- **响应结构**: status。
- **触发 WS 事件**: `channel_close`（cses_channel.go:2211-2212，data=closeMsg.ToMap()，broadcast 到 channelId）。

### 28. POST /api/cses/channel/onlineStatus — 频道成员在线状态

- **App 方法**: `ChannelOnlineStatus(channelIds, session)` (cses_channel.go:2221)。
- **DB 表**: `channelmember`（取成员）+ 在线状态判定（status / 内存）。
- **请求结构 verbatim**（handler 内联，channel.go:108-110）:
```go
var param struct {
	ChannelIds []string `json:"channelIds"`
}
```
- **响应结构**: `dto.CommonRes.data = []entity.ChannelOnlineStatusGroup`。
- **触发 WS 事件**: 无。
- **业务说明**: 批量查询频道是否有在线成员（典型用于私聊在线指示）。

### 29. POST /api/cses/channel/member/change/role — 修改成员角色

- **App 方法**: `UpdateCsesChannelMemberRoles(*cmd)` (cses_channel.go:1441)。
- **DB 表**: `channelmember`(Role 列)。
- **请求结构 verbatim**（`command.UpdateChannelMemberRole`，command/channel_member.go:10-15）:
```go
type UpdateChannelMemberRole struct {
	ChannelId string                 `json:"channelId"`
	UserIds   []string               `json:"userIds"`
	Role      *types.ChannelRoleType `json:"role"`
	OwnerId   string                 `json:"-"`
}
```
- **校验**: `len(UserIds) > 0` 且 `ChannelId != ""`。OwnerId 由 handler 用 session 覆盖（channel.go:573）。
- **响应结构**: status。
- **触发 WS 事件**: `channel_member_role_updated`（cses_channel.go:1448-1452，broadcast 到 ChannelId）。

### 30. POST /api/cses/channel/member/change/notify — 修改成员免打扰

- **App 方法**: `UpdateCsesChannelMemberNotify(cmd, session.UserId)` (cses_channel.go:1338-1360)。
- **DB 表**: `channelmember`(Notify 列)。
- **请求结构 verbatim**（`command.UpdateChannelNotify`，command/channel_member.go:27-31）:
```go
type UpdateChannelNotify struct {
	ChannelId string            `json:"channelId"`
	Notify    *types.NotifyType `json:"notify"`
	UserId    *string           `json:"-"`
}
```
- **校验**: `cmd.ChannelId != ""`（handler，channel.go:511）。UserId 覆盖（channel.go:516）。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel`（cses_channel.go:1356-1357，targeted 到 userId，data=channelNotifyVo.ToMap()，⚠️ ToMap 用 `UpdateChannelNotifyVo` 的 `id` key 而非 `channelId`）。

### 31. POST /api/cses/channel/member/change/nickname — 修改群昵称

- **App 方法**: `UpdateCsesChannelMemberNickname(channelId, userId, nickname)` (cses_channel.go:1317-1337)。
- **DB 表**: `channelmember`(Nickname 列)。
- **请求结构 verbatim**（`command.UpdateChannelMemberNickname`，command/channel_member.go:54-58）:
```go
type UpdateChannelMemberNickname struct {
	ChannelId string `json:"channelId"`
	UserId    string `json:"userId"`
	Nickname  string `json:"nickname"`
}
```
- **校验**（Validate，返回归一化 *string）: ChannelId/UserId 非空；trim 后空串→清空(nil)；含非 BMP(emoji)→error；rune 长度 >128→error。handler 在 UserId 为空时用 session 覆盖（channel.go:539-541）。
- **响应结构**: status。
- **触发 WS 事件**: `update_channel_member_nickName`（cses_channel.go:1330-1334，broadcast 到 channelId）。

### 32. POST /api/cses/channel/member/change — 成员加入/退出变更

- **App 方法**: `ChannelMemberChange(ctx, param, session)` (cses_channel.go:881-1107)。
- **DB 表**: `channelmember`(ChannelMembersJoin/Leave) / `channelmemberhistory`(LoadSince...) / `post`(系统消息) / GrpcInvoke 对端 paas（ReadIdNamesByUserIdsRaw）。
- **请求结构 verbatim**（`entity.ChannelMemberEvent`，channel.go:632-636）:
```go
type ChannelMemberEvent struct {
	ChannelId  string                       `json:"channelId"`
	JoinUsers  []model.ChannelMemberMinimal `json:"joinUsers"`
	LeaveUsers []model.ChannelMemberMinimal `json:"leaveUsers"`
}
```
- **响应结构**: status/message。
- **触发 WS 事件**: `channel_member_update`（cses_channel.go:974-975，per-user targeted，data=value.ToMap()）；`post`（加群/退群系统消息，cses_channel.go:874-875）；`post_update`（cses_channel.go:1018-1019）。
- **业务说明**: 单次可同时含 join + leave 两组成员；transactional 写成员/历史/系统消息。ctx 必须透传保 trace+companyId。

### 33. POST /api/cses/channel/member/leave — 退出频道（自己）

- **App 方法**: `ChannelMemberLeave(cmd, session)` (cses_channel.go:2107-2190)。
- **DB 表**: `channelmember`(删/标记) / `channelmemberhistory` / `post`(退群系统消息)。
- **请求结构 verbatim**: `entity.ChannelIdStruct`（同 #25，用 ChannelId + UserId）。
- **校验**: `ChannelId != ""`（Validate）；**鉴权**: `*cmd.UserId != session.UserId` → 拒绝（channel.go:789-793，只能自己退）。
- **响应结构**: status。
- **触发 WS 事件**: `channel_member_update`（成员变更广播）；`post`（退群系统消息）。
- **业务说明**: `NewCreator` 字段用于群主退群时转让群主。

### 34. POST /api/cses/channel/sync/notify — 离线增量同步（cursor-based）

- **App 方法**: `SyncChannels(ctx, userID, cursors)` (sync.go:71 → app/sync.go:132+)。
- **DB 表**: `channel_event`(append-only SoT, GetMaxSeq / 区间查 events) / `post`(批量 GetPosts 填 Messages map) / `channelmember`(FilterMemberChannelIds 越权闸门)。
- **请求结构 verbatim**（`syncRequest`，sync.go:31-33 + `app.SyncCursor`，app/sync.go:91-94）:
```go
type syncRequest struct {
	Cursors []app.SyncCursor `json:"cursors"`
}
type SyncCursor struct {
	ChannelID string `json:"channelId"`
	FromSeq   int64  `json:"fromSeq"`
}
```
- **校验**: `len(Cursors)==0` → 失败；`len > SyncMaxChannelsPerRequest(500)` → 失败（sync.go:54-69）。
- **响应结构 verbatim**（`dto.CommonRes.data = *app.SyncResult`，app/sync.go:98-115）:
```go
type SyncResult struct {
	Entries    []*SyncEntry `json:"entries"`
	ServerTime int64        `json:"serverTime"`
}
type SyncEntry struct {
	ChannelID string                 `json:"channelId"`
	Kind      SyncEntryKind          `json:"kind"`        // "no_change"|"events"|"snapshot"|"too_long"
	Events    []*entity.ChannelEvent `json:"events"`
	NextSeq   *int64                 `json:"nextSeq,omitempty"`
	ResetTo   *int64                 `json:"resetTo,omitempty"`
	Messages  map[string]interface{} `json:"messages,omitempty"`
}
```
  `entity.ChannelEvent`（=`ent.ChannelEvent`，ent/channelevent.go）字段 verbatim:
```go
ID        string  `json:"id,omitempty"`
CreateAt  int64   `json:"createAt,omitempty"`
UpdateAt  int64   `json:"updateAt,omitempty"`
ChannelId string  `json:"channelId,omitempty"`
EventSeq  int64   `json:"eventSeq,omitempty"`     // sync cursor SoT 序号
EventType int8    `json:"eventType,omitempty"`    // 1=new 2=edit 3=delete 6=read
MsgId     *string `json:"msgId,omitempty"`
ActorId   string  `json:"actorId,omitempty"`
Payload   map[string]interface{} `json:"payload,omitempty"`
```
- **触发 WS 事件**: 无（纯 HTTP 同步直返）。
- **业务说明**: 决策树（app/sync.go:120-131）: gap==0→no_change；0<gap≤500→events(+Messages 自包含 post 快照)；gap≤5000→events 分页(NextSeq 续拉)；gap>5000→too_long(ResetTo=maxSeq 清表重拉)。**越权闸门**: FilterMemberChannelIds 批量校验成员归属，完全非成员 → no_change + 空 events + NextSeq 回显 fromSeq（不泄露真实 maxSeq）；成员中非 viewer(密聊) → 放行走 phantom（events 返回但 Messages 不含正文）。SyncEntryKind enum 字面量 snake_case，**不可改**。

---

## 迁移注意点（给目标后端 cses-im-server）

1. **WS 已注释**: `add/manger` (#19) 与 `remove/manger` (#20) 的 WS 推送（`add_manager` / `remove_manager`）当前在 mattermost 已注释停用，仅走 GrpcInvoke 对端。迁移时需确认是否恢复，或改走 channel/sync 同步。
2. **不推送的 change/***: `change/props` (#15) 落库后**不广播**，与其他 change/* 不一致。
3. **字段大小写异常**:
   - `IncrementChannelParam.ChannelId` 是 **PascalCase** `json:"ChannelId"`（#1），与其余 camelCase `channelId` 不一致。
   - `ent.Channel.DirectKey` 是 **snake_case** `json:"direct_key"`（#7-8）。
   - `UpdateChannelNotify` 经 `UpdateChannelNotifyVo.ToMap()` 序列化时 key 是 `id` 而非 `channelId`（#30 WS payload）。
4. **WS event 字符串值**（snake_case，verbatim 自 websocket_message.go）: `increment_channel` / `increment_channel_end` / `channel_created` / `channel_member_update` / `channel_close` / `channel_member_role_updated` / `change_channel_approval` / `update_channel_notice` / `update_channel` / `post_update` / `update_channel_member_nickName` / `post_pin` / `post`。
5. **sync cursor 字段名**: wire 用 `fromSeq` / `eventSeq`（非 `seq`），too_long 阈值 5000、events 上限 500、单请求 cursor 上限 500——这些是硬契约。
6. **channel_event 表上线 SQL 非自动**（见项目 CLAUDE.md「上线部署 TODO」），新环境需手动跑 migration + backfill。
