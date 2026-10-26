# Gap 分析 — channel 域（csesapi → cses-im-server）

> 对照源：`docs/csesapi-migration/full-map/partials/backend-http-channel.md`（34 接口）
> 目标实现：`cses-im-server/internal/api/{channel.go, channel_change.go, channel_create.go, channel_member.go, channel_manager_pin.go, sync.go, handlers.go(register)}` + `internal/app/cses_channel*.go` + `internal/ws/event_type.go`
> 已读交叉验证：`docs/04-coverage-gap-map.md`、`docs/2026-06-15-cutover-checklist`、`NEED_FIX_channel_change_systempost.md`（已闭合）
>
> **结论速记**：channel 域 **路由面 34/34 全部已注册**，**wire 契约 verbatim 对齐**（casing / 枚举 / 阈值 / WS event 字符串），**WS event 常量齐备**（含故意不实现的 add/remove manager，与 mattermost 注释态一致）。差异**不在路由/字段层**，而在 **CreateCsesPost 系统消息消费侧编排未完整（P0-2）**，影响 displayName/orient/purpose/permission/manager/pin 这些「靠系统消息表达」的路由的**接收方可感知效果**（落库 + 计数 + `post` 广播）。

## 三态对照表

| # | csesapi 接口 | im-server 实现（路由 + app 方法） | 状态 | 差异 / 缺口 |
|---|---|---|---|---|
| 1 | POST `/channels/load/increment` | `loadIncrementChannel` → `LoadIncrementChannels`→`RunIncrementWorker` | 已迁移 | `IncrementChannelParam{TimeStamp,ChannelId(PascalCase),Cursors[]SyncCursorLite}` verbatim 对齐；WS `increment_channel`/`increment_channel_end` 齐；needSync(cursors stamp) 已移植 |
| 2 | POST `/channel/load/incrementByChannelId` | `loadSingleIncrementChannel` → `LoadSingleIncrementChannel` | 已迁移 | 内联 `{channelId}` + 空校验对齐；HTTP 同步直返 `*IncrementChannel` |
| 3 | POST `/channels/enableApproval` | `changeApprovalStatus` → `ChangeApprovalStatus` | 已迁移 | `EnableApprovalParam` 对齐；**im-server 加了 nil 防御**（源直接解引用 panic 风险）→ 更稳，非缺口；WS `change_channel_approval` 齐 |
| 4 | POST `/channels/view` | `viewChannels` → `CsesViewChannels` | 已迁移 | `ViewChannels{channels[]{id,isRoot}}` 对齐；无 WS |
| 5 | POST `/channels/member/byIds` | `loadChannelMembersByIds` → `LoadChannelMembersByChannelIds` | 已迁移 | 内联 `{channelIds}` + 1..200 校验对齐；成员归属过滤；data=按 channelId 分组 map |
| 6 | POST `/channel/member/snapshot` | `getMembersSnapshot` → `GetMembersSnapshot` | 已迁移 | `GetMembersSnapshotParam` + Validate 对齐 |
| 7 | POST `/channel/create` | `createChannel`→`handleCreateChannel(false)` → `CreateCsesChannel(owner=nil)` | 部分迁移 | 路由/`CreateChannelSpecifyOwner`(嵌 ent.Channel,DirectKey snake_case)/Validate/WS `channel_created`+`channel_member_update` 对齐。**欢迎系统消息 `post` 帧走 CreateCsesPost 链**，受 P0-2 消费侧编排不完整影响（见末节） |
| 8 | POST `/channel/createSpecifyOwner` | `createChannelSpecifyOwner`→`handleCreateChannel(true)` → `CreateCsesChannel(owner=param.Owner)` | 部分迁移 | 同 #7 |
| 9 | POST `/channel/change/permission` | `changeChannelPermission` → `ChangeChannelPermission` | 部分迁移 | `ChangeChannelPermission` cmd + Validate + session 覆盖 UserId 对齐。**行为差异**：源走 `update_channel` WS（清单标注）；im-server 走**逐字段系统消息 `post`**（NEED_FIX 已闭合，IsChannelMaster 鉴权就位）→ 表达手段不同 + 受 P0-2 影响 |
| 10 | POST `/channel/change/notice` | `changeChannelNotice` → `ChangeChannelNotice` | 已迁移 | `ChangeChannelNotice` 对齐；WS `update_channel_notice`(notice.ToMap) 齐 |
| 11 | POST `/channel/change/info` | `changeChannelInfo` → `ChangeCsesChannelInfo` | 已迁移 | `ChangeChannelInfo` 聚合 cmd 对齐；WS `update_channel`(channel.ToMap) 齐 |
| 12 | POST `/channel/change/source` | `changeChannelSource` → `UpdateCsesChannelSource` | 已迁移 | `ChangeChannelSource` 对齐；WS `update_channel` 齐 |
| 13 | POST `/channel/change/displayName` | `changeChannelDisplayName` → `UpdateCsesChannelDisplayName` | 部分迁移 | `ChangeChannelDisPlayName` 对齐。**行为差异**：源仅落库（清单 #13 标 update_channel）；im-server 落库 + **系统消息 `post`**（field=displayName，NEED_FIX 闭合）→ 受 P0-2 影响 |
| 14 | POST `/channel/change/picture` | `changeChannelPicture` → `UpdateCsesChannelPicture` | 已迁移 | 解码 entity.Channel(picture/pictureType/id) + nil 校验对齐；WS `update_channel`(ch.ToMap) 齐 |
| 15 | POST `/channel/change/props` | `changeChannelProps` → `UpdateCsesChannelProps` | 已迁移 | `ID!=""` 校验对齐；**与源一致：仅落库不广播**（注释标 update_channel 但 props 路径源无 Publish；im-server 同样无广播）✅ |
| 16 | POST `/channel/change/orient` | `changeChannelOrient` → `UpdateCsesChannelOrient` | 部分迁移 | `UpdateChannelGuidePurposeCommand` 对齐。**行为差异**：im-server 走**系统消息 `post`**（field=orient，NEED_FIX 闭合）→ 受 P0-2 影响 |
| 17 | POST `/channel/change/purpose` | `changeChannelPurpose` → `UpdateCsesChannelPurpose` | 部分迁移 | 同 #16（field=purpose） |
| 18 | POST `/channel/change/top` | `changeChannelTop` → `UpdateCsesChannelTop` | 已迁移 | `UpdateChannelTopCommand` 对齐；WS `update_channel` **单播**到 result.UserId({id,userId,channelIsTop}) 齐 |
| 19 | POST `/channel/add/manger` | `addChannelManger` → `AddCsesChannelManger` | 部分迁移 | 路由/`AddChannelMangerCommand`/OwnerId 覆盖对齐。**WS 与源一致：`add_manager` 故意不实现**（event_type.go 注释「源已注释」）。im-server 改走 **addManager 系统消息 `post`**（受 P0-2 影响）；**未见 GrpcInvoke 对端透传**（源有，迁移侧靠 Pulsar 系统消息替代）|
| 20 | POST `/channel/remove/manger` | `deleteChannelManger` → `DeleteCsesChannelManger` | 部分迁移 | 同 #19（removeManager 系统消息）|
| 21 | POST `/channel/add/postPinned` | `addChannelPostPinned` → `PinnedCsesChannelPost` | 部分迁移 | `ChannelPostPinnedCommand` + Validate(ChannelId/PostId) 对齐。**WS 差异**：源走 `post_pin`；im-server 走 **addPostPin 系统消息 `post`**（handlers.go:139 注释）→ 表达手段不同 + 受 P0-2 影响。需 live 验 `post_pin` 是否仍发 |
| 22 | POST `/channel/remove/postPinned` | `deleteChannelPostPinned` → `DeleteCsesChannelPostPinned` | 部分迁移 | 同 #21（removePostPin 系统消息）|
| 23 | POST `/channel/load/notice` | `loadChannelNotice` → `LoadChannelNotice` | 已迁移 | `ChannelIdCommand` 对齐；data=ChannelNoticeSlice；无 WS |
| 24 | POST `/channel/load/postPinned` | `loadChannelPostPinned` → `LoadPinnedCsesChannelPost` | 已迁移 | data=`[]PostPinnedRes`(嵌 ent.Post + operator) 对齐；无 WS |
| 25 | POST `/channel/load/admin` | `loadChannelAdmin` → `LoadChannelAdmin` | 已迁移 | `ChannelIdStruct` 对齐；data=ChannelMemberEntitySlice；无 WS |
| 26 | POST `/channel/query` | `channelQuery` → `ChannelQueryByCondition` | 已迁移 | 内联匿名 struct 嵌 `entity.Channel`+`entity.PageOpts` 对齐；data=[]Channel |
| 27 | POST `/channel/close` | `channelClose` → `ChannelClose` | 已迁移 | 内联 `{channelId}` 对齐；WS `channel_close`(closeMsg.ToMap) 齐 |
| 28 | POST `/channel/onlineStatus` | `channelOnlineStatus` → `ChannelOnlineStatus` | 已迁移 | 内联 `{channelIds}` 对齐；data=[]ChannelOnlineStatusGroup；无 WS |
| 29 | POST `/channel/member/change/role` | `updateChannelMemberRole` → `UpdateCsesChannelMemberRoles` | 已迁移 | `UpdateChannelMemberRole` + Validate + OwnerId 覆盖对齐；WS `channel_member_role_updated` 齐 |
| 30 | POST `/channel/member/change/notify` | `updateNotify` → `UpdateCsesChannelMemberNotify` | 已迁移 | `UpdateChannelNotify` 对齐；WS `update_channel` 单播({id,notify,userId}，`id` key 非 channelId 的 quirk 对齐) |
| 31 | POST `/channel/member/change/nickname` | `updateChannelMemberNickname` → `UpdateCsesChannelMemberNickname` | 已迁移 | `UpdateChannelMemberNickname` + Validate(trim/≤128rune/仅BMP) 对齐；WS `update_channel_member_nickName` 齐 |
| 32 | POST `/channel/member/change` | `channelMemberChange` → `ChannelMemberChange` | 部分迁移 | `ChannelMemberEvent{joinUsers,leaveUsers}` 对齐；WS `channel_member_update` 齐。**入退群系统消息 `post`/`post_update`** 走 CreateCsesPost 链 → 受 P0-2 影响 |
| 33 | POST `/channel/member/leave` | `channelMemberLeave` → `ChannelMemberLeave` | 部分迁移 | `ChannelIdStruct` + Validate + UserId 强制 session 覆盖(只能退自己)对齐；WS `channel_member_update` 齐。退群/群主转让系统消息 `post` 受 P0-2 影响 |
| 34 | POST `/channel/sync/notify` | `syncChannels` → `SyncChannels` | 已迁移 | `syncRequest{cursors[]SyncCursor{channelId,fromSeq}}` verbatim；`SyncResult{entries,serverTime}` + `SyncEntry{channelId,kind,events,nextSeq,resetTo,messages}` verbatim；`ChannelEvent`(createAt/eventSeq/eventType/msgId/actorId/payload) verbatim；**阈值全对齐**(events≤500/too_long>5000/cursor≤500)；4 分支决策树 + 成员归属闸门(no_change 回显 fromSeq 不泄露 maxSeq) + phantom 路径齐 |

## 状态计数

- **已迁移（route+wire+WS 全对齐）**：21 个（#1-6,10-12,14,15,18,23-31,34）
- **部分迁移（route+wire 对齐，差异在系统消息消费侧 P0-2 / WS 表达手段）**：13 个（#7,8,9,13,16,17,19,20,21,22,32,33）
- **缺失**：0 个

> 注：所有「部分迁移」的根因是**同一个**——它们靠 `CreateCsesPost` 落系统消息 `post` 帧来表达副作用，而消费侧编排（P0-2）未完整。路由本身、入参契约、鉴权、主 WS 帧（如 channel_member_update / update_channel）**都已就位**。不是接口缺失，是**消费落库闭环未完工**。

## WS 事件齐备性核对（internal/ws/event_type.go）

channel 域触发的 WS 事件常量**全部已定义**：
`increment_channel` / `increment_channel_end` / `channel_created` / `channel_member_update` / `channel_close` / `channel_member_role_updated` / `change_channel_approval` / `update_channel_notice` / `update_channel` / `update_channel_member_nickName` / `post_pin` / `post_update` / `post` ✅

- `add_manager` / `remove_manager`：**故意不实现**（event_type.go:24 注释「源已注释，暂不实现」）——与 mattermost csesapi 当前注释态一致，**非缺口**。
- `post_pin` 常量存在，但 #21/#22 当前注释改走系统消息 `post` 表达——需 live 验证 `post_pin` 帧是否仍发（可能行为收窄）。

## 阻塞优先级清单（对替代 mattermost 的影响）

> 本域**无缺失路由**，阻塞全部来自共享的「系统消息消费侧」根因（与 post 域共担），按客户端可感知程度排序：

1. **[P0 / 最高] CreateCsesPost 消费侧编排不完整（P0-2）** — 影响 #7,8,9,13,16,17,19,20,21,22,32,33 共 12 个「靠系统消息表达」的路由。具体缺：`total_msg_count(_all)` 计数 / `posts_update` 批帧 / reply-chain echo / notification·push fanout（详见 cutover P0-2）。**后果**：建群欢迎语、改群名/介绍/描述/权限的系统提示、加管理员/置顶/入退群系统消息可能**不落库 / 接收方未读计数不准 / 无 push**。此为 cutover 关键路径，非 channel 域独有。
2. **[P1 / 高] #21/#22 postPinned 的 WS 表达从 `post_pin` 改为系统消息 `post`** — 若客户端仍监听 `post_pin` 帧渲染置顶变更，需 live 验证是否回归。建议保留 `post_pin` 帧或确认客户端已切系统消息渲染。
3. **[P1 / 高] #9 change/permission、#13/16/17 displayName·orient·purpose 从 `update_channel` 改为系统消息 `post`** — mattermost 原走 `update_channel` 让客户端实时刷新群信息；im-server 改走系统消息。若客户端 UI 依赖 `update_channel` 帧更新群头部信息，需确认系统消息能等效驱动 UI，否则群信息变更**不实时反映**。
4. **[P2 / 中] #19/#20 manager 变更缺 GrpcInvoke 对端透传** — 源 add/remove manager 有 GrpcInvoke 到 paas 对端；迁移侧仅靠 Pulsar 系统消息。若 paas 侧依赖该 gRPC 通知，需补（取决于 paas 是否仍在链路）。
5. **[P2 / 中] consume-hop trace 透传缺失（P2-14）** — 系统消息经 Pulsar 消费边界时不透传 traceparent，分布式 trace 断链（违反 Trace 父子约束）。不影响功能，影响可观测性。

## 结论

channel 域**接口面已 100% 移植**（34/34 路由 + wire 契约 + 鉴权 + 主 WS 帧 verbatim 对齐），是迁移完成度最高的域之一。**唯一实质阻塞**是与 post 域共享的「系统消息消费侧编排（P0-2）」——它让 13 个路由的「系统消息表达」副作用未完整闭环。这不是 channel 域要单独补的接口，而是 cutover 关键路径 P0-2 解锁后自然消除。**WS event 齐备**，add/remove manager 不实现与源一致。
