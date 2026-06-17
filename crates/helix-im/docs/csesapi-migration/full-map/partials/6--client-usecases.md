# 客户端 message-v3 用户用例全链路图（service/core/types/constants）

> 源：`cses-client/src/pages/message-v3/`（service + types + enum），Rust 端 `src-tauri/src/features/im/`（router / handlers / http）。
> 字段级引用 verbatim 摘自源码；HTTP 路径以 imHttp 调用为准（baseUrl 自动拼 `/api/cses` 前缀，故服务端实际为 `/api/cses` + 下方 path）。
> WS 事件链路：服务端 WS 帧（`msg.action`）→ Rust `router::route` 分发到 handler → handler `emit_im("im:*", ...)` → Angular `ipcEvent.service.on('im:*')` → `getXxx$()` Observable → `ipcEventHandler` `CoreEventManager.dispatch(MessageEventDispatch.*)`。

---

## 0. 双 HTTP 实例与解包契约（所有用例公共前提）

`message.service.ts` 注入两个 HTTP 客户端：

| 实例 | 注入 token | host | 前缀 | 解包 | 用途 |
|---|---|---|---|---|---|
| `imHttp` | `IM_HTTP` | `imConn.getImBaseUrl()`（= auth.mattermostHttp） | 调用 path 前自动拼 `/api/cses` | `ImUnwrapStrategy`：信封 `{status:'SUCCESS', data}` → 解出 `data`；业务失败弹窗 `MessageErrorReporter` | message-API / posts / channel / vote / average |
| `api` | `AppHttpService`（默认） | 默认网关 host | 无 `/api/cses` 前缀 | 默认 | 全局搜索 `/Im/search/*` |

- 信封类型 verbatim（`types/sync.ts:24`）：`interface CommonResEnvelope<T> { status: 'SUCCESS' | 'failed' | 'FINISH'; data?: T; message?: string; }`
- connectionId header 由 `networkInterceptor` 在请求阶段按需注入（hello 推送 connectionId 之前 IM 请求不排队）。
- 所有 imHttp 方法均为 `imHttp.post$<any>(path, body).pipe(catchError(...))`，**全部 POST**（无 GET）。

---

## 用例集合一：消息收发（messageSend.service / message.service / ipcEventHandler）

### UC-1.1 发送文本/富文本消息（模板用例）

**用户动作**：聊天输入框输入文本 → 回车/点发送。

**调用链**：
1. `MessageSendService.sendMessage({ data: ChatMessage, channelId })`（`messageSend.service.ts:66`）
   - 过滤空消息（`item.message.trim()` 或 `props` 非空对象）；空则拦截返回。
   - `buildMessageObject()`：注入 `userId/teamId(=companyId)/userSnapshot{orgName,deptName,userName,userId,teamId}/createAt`；`viewers` 空时补 `['all']`；有 `localRepliedMessage` 时合并 `findReplyRootId()` + `repliedMessage` 快照。
   - 文件类消息先 `fileUploadService.processMessageFiles()`（见 UC-1.3）。
   - 本地乐观写库：`tauriService.invoke('store:setItem', 'message', params, 'temporaryId')`（`formatAndUpdateLocalDB`）。
   - 派发本地 UI：`CoreEventManager.dispatch(UPDATE_MESSAGE_WITH_SEND, processedMessage)` + `SET_MESSAGE_LAST_MSG`。
2. `MessageService.sendMsg(processedMessage)`（`message.service.ts:137`）发出 HTTP。

**HTTP 请求**：`POST /api/cses/posts/create`
- body verbatim（`message.service.ts:139`）：
  ```
  { ...msg, id: msg.id||'', props: msg.props||{}, replyId: msg.replyId||undefined,
    replyRootId: msg.replyRootId||undefined, topicId: msg.topicId||'',
    viewers: msg.viewers||[], revoke: false, updateAt: msg.updateAt||undefined,
    teamId: this.ctx.userInfo.companyId||'' }
  ```
  发送前 `delete params.localRepliedMessage; delete params.createAt;`
- 关键字段约定：`temporaryId`（客户端去重/对账锚点）、`viewers`（`['all']` 或定向 userId 数组，密聊用）、`type`（`messageType` 枚举字符串）。
- 发送时启 15s 超时定时器 `createSendTimeout(temporaryId)`：超时则 `store:updateItem sendStatus:'unSend'` + dispatch `POST{action:'post',sendStatus:'unSend'}` + toast「消息发送失败」。

**期望响应**：信封 `{status:'SUCCESS', data}`，解包后 resolve（仅作 ACK，真正落地靠 WS 回声）。

**订阅/收到的 WS 事件**：服务端 `posted` →（cses 路径 Pulsar consumer → `post_core.go SendPostMessage` → WS `action:"post"`）→ Rust `router` `"post" => post::handle`（`router.rs:40`）→ `emit_im("im:post:received", ...)`（`handlers/post.rs:92/1261`）→ Angular `getPostReceived$()`（`ipcEvent.service.ts:86`）→ `handlePostReceived`（`ipcEventHandler.service.ts:234`）。

**后续触发**：
- `messageService.clearSendTimeout(temporaryId)`（取消 15s 计时）。
- viewers 白名单过滤 `isMessageVisibleToUser`；不可见则跳过 dialogCache（密聊定向消息隔离）。
- 主窗口 `updateDialogCacheFromWebSocket(channelId, [data])`。
- `dispatch(MessageEventDispatch.POST, message)` → messageDialog 渲染。
- Track 分流：自发回流 → `sendMessage end 发送成功`；他发 → `receivedMessage end 渲染成功`。

### UC-1.2 发送 DOCUMENT（文档）消息

**用户动作**：编辑器创建文档消息发送。
**调用链**：`MessageSendService.sendDocumentMessage({ snapshot, channelId })`（`messageSend.service.ts:242`）→ `snapshotToPlainText(snapshot, 2000)` 作 `message`，前 50 字作 `simpleMessage`，`type: messageType.DOCUMENT`，`props.document = { snapshot, version:1 }`，`viewers:['all']` → 复用 UC-1.1 的 `sendMessage`（`dispatchEvent:true`）。
**HTTP/WS**：同 UC-1.1（`POST /posts/create` → `im:post:received`）。

### UC-1.3 发送图片/文件消息（含上传进度）

**用户动作**：点「图片/文件」选择文件发送。
**调用链**：`messageSend.sendMessage`（`shouldProcessUpload` 命中 `props.file`/`props.files[]`/`props.template.type==='IMAGE'`）→ 先 dispatch `UPDATE_MESSAGE_WITH_SEND{sendStatus:'uploading',uploadProgress:0}` → `MessageFileUploadService.processMessageFiles(message, onProgress)` → 进度回调 dispatch 更新 `uploadProgress` → 上传完后走 UC-1.1 `POST /posts/create`。
**失败**：dispatch `UPDATE_MESSAGE_WITH_SEND{sendStatus:'unSend'}` + toast「文件上传失败」。

### UC-1.4 重发失败消息
**用户动作**：点击 `sendStatus:'unSend'` 消息的重发。
**调用链**：复用 UC-1.1（temporaryId 复用 → 本地行 upsert 覆盖）。dispatch enum `RE_SEND_MESSAGE`。

### UC-1.5 撤回消息
**用户动作**：消息右键 →「撤回」。
**调用链**：`MessageService.revokeMessage(postId)`（`message.service.ts:684`）。
**HTTP**：`POST /api/cses/posts/revoke`，body `{ postId }`。
**WS**：服务端 `post_deleted` → 在线走 WS `action:"posts_update"` → Rust `post::handle_batch_update` → `emit_im("im:post:batch-updated")` → `getPostsUpdated$()` → `handlePostsUpdated` → `dispatch(POSTS_UPDATE)`；离线 sync 删除走 Rust gate `emit_im("im:post:deleted")` → `getPostDeleted$()` → `handlePostDeleted` → `dispatch(POST_UPDATE,{action:'post_deleted',data:{id:msgId,channelId}})`。

### UC-1.6 编辑消息
**用户动作**：消息「重新编辑」后保存。
**HTTP**：走 `/posts/create`（带 `id` 非空 = update 语义，由服务端识别）或 `/posts/update`（视实现）。dispatch enum `RE_EDIT_MESSAGE`。
**WS**：`post_edited` → WS `action:"post_update"` → Rust `post::handle_update` → `emit_im("im:post:updated")` → `getPostUpdated$()` → `handlePostUpdated` → `dispatch(POST_UPDATE)`。

### UC-1.7 转发/合并转发消息
**用户动作**：选消息 →「转发」到多个会话。
**调用链**：`MessageService.sendRelayMessages({ channelIds, posts })`（`message.service.ts:802`）。
**HTTP**：`POST /api/cses/posts/createPosts`，body `{ channelIds, posts }`。
**WS**：各目标 channel 收到 `im:post:received`（同 UC-1.1）。

### UC-1.8 快捷回复（emoji 表态）
**调用链**：`MessageService.sendQuickRelayMessage({ userId, postId, emoji })`（`message.service.ts:817`）。
**HTTP**：`POST /api/cses/posts/quickReply`，body `{ userId, postId, emoji }`。

### UC-1.9 加急消息 + 加急已读
**调用链**：`onUrgentPost({ channelId, postId, targetIds })`（`message.service.ts:606`）/ `onPostUrgentConfirm({ channelId, postId })`（`:622`）。
**HTTP**：`POST /api/cses/posts/urgentPost` / `POST /api/cses/posts/urgentConfirm`。
**WS**：加急属性更新走 `im:post:updated`（`handlePostUpdated` 内 `createSysNotification(..., true)` 创建系统通知 + `normalizeExpediteFields` 归一化 `expediteMap→expediteRecipients+expediteSender`）。

### UC-1.10 定时消息（创建/取消/查询）
**调用链**：`createScheduleMessage` / `cancelScheduleMessage` / `queryScheduleMessage`（`message.service.ts:772/780/788`）。
**HTTP**：`POST /api/cses/posts/createSchedule` / `/posts/cancelSchedule` / `/posts/getSchedule`。
**WS**：`im:channel:schedule-created` / `im:channel:schedule-canceled`（`ipcEvent.service.ts:205/214`）→ payload `{ channelId, hasSchedulePost }`。

---

## 用例集合二：消息历史加载（chatContentShared / Rust http.rs）

### UC-2.1 切群首屏（最新 N 条）
**用户动作**：点击会话进入。
**调用链**：本地 SQLite 优先（`imDataSource:query`）；too_long 兜底走 `ImSyncService.fetchFirstScreenAfterTooLong`。
**HTTP（兜底）**：`POST /api/cses/posts/getLatestPost`，body `{ channelId, timestamp:0 }`（`im-sync.service.ts:114`；注：服务端硬编码最新 20 条，timestamp 被忽略，见 Rust `http.rs:87`）。
**后续**：`dispatch(POST_INCREMENT, { channelId, action:'too_long_reset', messages: list })`（键必须是 `messages`）。

### UC-2.2 上拉加载更早历史（远端兜底翻页）
**用户动作**：消息列表上拉到顶。
**调用链**：`chatContentShared.handlePullRefresh` 本地 `imDataSource:query{direction:'up'}` 耗尽（返回 0）→ 取 `messages.find(m=>m?.id)`（最旧带 server id 的一条，self 消息本地 `id=''` 不能作锚点）→ `tauriService.invoke('im_load_older_messages', { channelId, anchorPostId, anchorCreateAt:beforeTimestamp, limit:20 })`（`chatContentShared.service.ts:613`）→ Rust `channel_service::fetch_older_posts_by_context` 翻页循环。
**HTTP（Rust 发起）**：`POST /api/cses/posts/postContext`，body `{ postId: anchorPostId, before }`（`http.rs:96`，每轮 `batch=50`）。

### UC-2.3 按 postId 定位历史（跳转/搜索结果点击）
**调用链**：`MessageService.queryPostsById(postId)`（`message.service.ts:671`）。
**HTTP**：`POST /api/cses/posts/getPostsAfterIndex`，body `{ postIds: postId }`（单 postId 字符串）。响应解包后 `{ posts: Post[], count }`，调用方读 `res.posts`。dispatch enum `JUMP_TO_MESSAGE`。

### UC-2.4 一级回复 / 二级回复列表
**调用链**：`queryReplyMessage({ replyId, pageSize, pageNumber })`（`:637`）/ `queryReplyBranchMessage({ replyFirstLevelId, pageSize, offset })`（`:652`）。
**HTTP**：`POST /api/cses/posts/getReplies` / `POST /api/cses/posts/getReplyBranch`。dispatch enum `OPEN_REPLY_DRAWER` / `REPLY_MESSAGE`。

---

## 用例集合三：已读未读（message.service / messageDialog / pending-channel-view）

### UC-3.1 会话已读（进入/点击会话内消息）
**用户动作**：激活会话且点击 message-container（pending-view 闸门放行）。
**调用链**：`messageDialog.service.ts:1714/1810` → `MessageService.onChannelRead({ channels: [{ id: channelId }] })`（`message.service.ts:236`）。
**HTTP**：`POST /api/cses/channels/view`，body `{ channels: [{ id }] }`（注：`onChannelRead` 内部 fire-and-forget `.subscribe()`，无返回处理）。
**WS（read echo）**：服务端写 `channelmembers.last_read_seq` → 多设备 echo `event_type=6` → 在线走 `im:post:read`（`getPostRead$` → `handlePostRead` → `dispatch(POST_READ)`）；离线 sync read echo 走 Rust gate `im:channel:read_echo`（`handleReadEcho`，**actor==self 过滤** → `dispatch(POST_READ,{channelId,lastReadSeq:eventSeq,actorId})`）。

### UC-3.2 单条消息已读
**调用链**：`MessageService.onPostRead(params)`（`message.service.ts:226`）。
**HTTP**：`POST /api/cses/post/read`。

### UC-3.3 模板消息「已收到」
**调用链**：`MessageService.templateReceived({ postId, channelId })`（`message.service.ts:453`）。
**HTTP**：`POST /api/cses/post/templateReceived`。

### UC-3.4 未读数维护
- 写入时即算（服务端 SoT，count 域 total_msg_count_all + phantom）；客户端 dispatch enum `UPDATE_UNREAD_COUNT` / `TOGGLE_NEXT_LEVEL_UNREAD_COUNT` 由 messageDialog 内部维护，无独立 HTTP。

---

## 用例集合四：离线同步（im-sync.service / message.service requestChannelIncrement / types/sync.ts / Rust gate）

> 主路径：cursor 单一真源是 Rust gate（`handlers/post.rs` 状态机 on_ws_event/on_gate_fire/flush_contiguous）。Angular `ImSyncService` 仅保留 **too_long 清表 + 重拉首屏** 唯一活路径（`im-sync.service.ts` 头注释）。

### UC-4.1 hello 重连后根群全量增量
**用户动作**：WS 重连成功（hello）。
**调用链**：Rust `hello::handle` → `emit_im("im:channels:loaded")`（`handlers/hello.rs:112`）；Angular `MessageGlobalService.requestSubtopicChannels` / `MessageService.requestChannelIncrement({ timestamp:0, cursors? })`（`message.service.ts:296`）。
**HTTP**：`POST /api/cses/channels/load/increment`
- body 类型 `IncrementRequest`（`types/sync.ts:65`）verbatim：`{ timestamp: number; channelId?: string; cursors?: SyncCursorItem[] }`
- `SyncCursorItem`（`:37`）：`{ channelId: string; fromSeq: number }`（fromSeq = 本地已知 max event_seq，首次 0）。
- 规则：带 `channelId` 的话题请求 **不带 cursors**；不带 channelId 的根群全量增量可带 `cursors`（服务端据此在帧 stamp `needSync`）。
**WS**：服务端逐 channel 推 `increment_channel`（`IncrementChannelFrame`，顶层 `needSync?:boolean` 缺省按 true）→ Rust `channel::handle_increment`（`router.rs:31`）→ `emit_im("im:post:increment-started"/"im:post:increment"/"im:channel:increment")`（`handlers/channel.rs:45/65/153`）→ Angular `getChannelIncrement$` / `getPostIncrement$` → `dispatch(INCREMENT_CHANNEL / POST_INCREMENT / POST_INCREMENT_STARTED / POST_INCREMENT_EMPTY / POST_INCREMENT_FAILED)`；全部推完 `increment_channel_end` → `channel::handle_increment_end` 写 `subtopics_loaded_at` + `emit_im("im:channel:update")`。

### UC-4.2 按需 sync notify（needSync 命中后补齐 gap）
**调用链**：`increment_channel` 帧 `needSync:true` → `service/sync-filter.ts pickNeedSyncChannelIds` 筛出需补群 → 调 sync notify。
**HTTP**：`POST /api/cses/channel/sync/notify`，body 类型 `SyncRequest`（`types/sync.ts:48`）：`{ cursors: SyncCursorItem[] }`。
**响应**：`SyncResponse`（`:150`）：`{ entries: SyncEntry[]; serverTime: number }`；`SyncEntry`（`:119`）：`{ channelId; kind: 'no_change'|'events'|'snapshot'|'too_long'; events: ImEvent[]; messages: Record<string,unknown>; nextSeq: number|null; resetTo: number|null }`。`ImEvent`（`:89`）：`{ eventSeq; eventType: 1|2|3|6; msgId; actorId; createAt; payload? }`（`createAt` 非 createdAt）。

### UC-4.3 too_long 清表重拉（唯一 Angular 活路径）
**触发**：Rust gate gap>5000 → `handlers/post.rs` 重置内存 cursor + `emit_im("im:sync:too_long",{channelId,resetTo})`（`post.rs:919`）。
**调用链**：Angular `ipcEventHandler.handleSyncTooLong`（`:1050`）订阅 → `ImSyncService.onRustTooLong(channelId, resetTo)`（`im-sync.service.ts:61`）→ `processTooLongEntry`：清 buffer/gate + `cursor = resetTo-1` + `fetchFirstScreenAfterTooLong`。
**HTTP**：`POST /api/cses/posts/getLatestPost`（同 UC-2.1）→ `dispatch(POST_INCREMENT,{channelId,action:'too_long_reset',messages:list})`。

### UC-4.4 心跳 gap 补偿（B4，Rust 心跳闭包组装）
**链路**：Rust 心跳 ping piggyback `PingData`（`types/sync.ts:242`）：`{ cursors: SyncCursorItem[]; allHash?: string }`（allHash = 全量根群 cursor 的 FNV-1a 64bit hash，16 位小写 hex）。pong 回 `PongData`（`:259`）：`{ gaps:[{channelId,fromSeq,maxSeq}]; hashMismatch: boolean }`。客户端逐 channelId 触发补偿 sync（去重+节流）。
**注**：此用例由 Rust `im_ws_client.rs build_ping_data` 组装，TS 类型仅作契约/调试定义。

### UC-4.5 独立窗口陌生 channel 兜底
**链路**：独立窗口（topic/independent/drag）本地无数据 → IPC `im:ensureChannelLoaded` → Rust `http.rs:50` `POST /api/cses/channel/load/incrementByChannelId`，body `{ channelId }`（**不**走 UC-4.1 的 `/channels/load/increment`）。

---

## 用例集合五：频道/群管理（message.service）

### UC-5.1 创建群聊
`createGroupChat(params)`（`:311`）→ `POST /api/cses/channel/create`。
**WS**：`channel_created` → Rust `channel::handle_create`（`router.rs:34`）→ `emit_im("im:channel:created",{channel,tracing})` → `getChannelCreated$` → `handleChannelCreated` → `dispatch(CREATE_DIALOG / CREATE_TOPIC / UPDATE_CHANNEL)`（按 type=='T' 与 owner 分流）。

### UC-5.2 创建话题（topic）
`createTopic(params)`（`:723`，body 含 `rootId/type='T'/postId/displayName/users[]` 等）→ `POST /api/cses/posts/makeTopic`。
**WS**：`channel_created`（type=T）→ `dispatch(CREATE_TOPIC)`。

### UC-5.3 关闭/退出群聊
`closeDialog(channelId)`（`:849`）→ `POST /api/cses/channel/close`；`leaveDialog({channelId,userId,newCreator})`（`:591`）→ `POST /api/cses/channel/member/leave`。
**WS**：`channel_close` → `channel::handle_close` → `emit_im("im:channel:closed",{channelId})` → `getChannelClosed$` → `dispatch(DELETE_DIALOG, channelId)`。

### UC-5.4 群属性修改（名称/公告/描述/导向/权限/置顶/通知等级）
| 方法（message.service） | HTTP path（+`/api/cses`） | enum |
|---|---|---|
| `setDialogName`（:373） | `/channel/change/displayName` | — |
| `setDialogNotice`（:387） | `/channel/change/notice` | — |
| `setDialogDesc`（:467） | `/channel/change/purpose` | — |
| `setDialogGuide`（:481） | `/channel/change/orient` | — |
| `setDialogPermission`（:497） | `/channel/change/permission` | — |
| `channelPinTop`（:326） | `/channel/change/top` | — |
| `setNotify`（:340） | `/channel/member/change/notify` | — |
**WS**：`update_channel` → `channel::handle_update` → `emit_im("im:channel:update")` → `getChannelUpdate$` → `dispatch(UPDATE_CHANNEL / UPDATE_TOPIC)`。

### UC-5.5 消息置顶（设置/取消/加载）
`setMessageTop`（:549）→ `/channel/add/postPinned`；`setMessageUnTop`（:563）→ `/channel/remove/postPinned`；`getTopMessages(channelId)`（:576）→ `/channel/load/postPinned`。dispatch enum `ADD_TOP_MESSAGE / TOP_MESSAGE_DRAWER`。

### UC-5.6 群公告（列表/保存/删除/确认收到）
`queryAnnouncementList`（:400）→ `/post/announcement/list`（body `{channelId, postId:channelId}` 兼容）；`saveAnnouncement(post)`（:416）→ `/post/announcement/save`；`deleteAnnouncement(postIds)`（:429）→ `/post/announcement/delete`（body `{postIds, postId:postIds}`）；`readAnnouncement({postId,channelId})`（:444）→ `/post/announcement/read`。

### UC-5.7 在线状态 / 群分组
`queryUserLineStatusByChannelId(channelIds)`（:841）→ `/channel/onlineStatus`（body `{channelIds}`）；`queryDialogModule()`（:825）→ `/modules/getAll`（Promise 包装）。

---

## 用例集合六：成员管理（message.service / channelMemberResolver）

### UC-6.1 拉人/踢人
`setDialogMemebers({channelId, options:[{option,userIds}]})`（:515）→ `POST /api/cses/channel/member/change`。
**WS**：`channel_member_update` → `channel::handle_member_update` → `emit_im("im:channel:member-updated")` → `getChannelMemberUpdated$` → `dispatch(UPDATE_CHANNEL)`。

### UC-6.2 设/撤管理员
`setDialogManager(type, params)`（:529）→ `type==='join'` → `/channel/add/manger`；`type==='leave'` → `/channel/remove/manger`。

### UC-6.3 改群昵称
`changeChannelMemberNickname({channelId,userId,nickname})`（:355）→ `POST /api/cses/channel/member/change/nickname`（返回类型 `{status:'SUCCESS'|'failed';message?;data?:null}`）。
**WS**：`update_channel_member_nickName` → `channel::handle_member_nickname_update` → `emit_im("im:channel:memberNickname",{channelId,userId,nickName})` → `getChannelMemberNickname$` → `dispatch(MEMBER_NICKNAME_UPDATE)`。

### UC-6.4 成员快照/全量成员
`queryMembersSnapshot(params)`（:249）→ `/channel/member/snapshot`（成员排序快照）；`getChannelMembersByIds(channelId)`（:266）→ `/channels/member/byIds`（`allMembers().length ≠ memberCount` 时自愈，覆盖 SQLite `channel.members`）。

---

## 用例集合七：搜索（message.service，走 `api`/AppHttpService 非 imHttp）

> 注意：搜索用 `this.api.post$`（默认网关，**无** `/api/cses` 前缀），path 形如 `/Im/search/*`。

| 方法 | HTTP path | 入参关键字段 |
|---|---|---|
| `searchGlobal`（:877） | `POST /Im/search/global` | `{ keyword, bizType:'im', pageNumber, pageSize, index:'channel'|'user'|'post'|'' }` |
| `searchHistoricalByChannel`（:891） | `POST /Im/search/searchByChannel` | `{ bizType, channelId, keyword, pageNumber, pageSize }` |
| `searchByChannelWithCategory`（:915） | `POST /Im/search/searchByChannelWithCategory` | `{ keyword, bizType, pageNumber, pageSize, channelId?, startTime?, endTime?, containerUserId?, searchScope? }` |
| `searchByChannelWithCategoryTopic`（:940） | `POST /Im/search/searchByChannelWithCategoryTopic` | `{ keyword, bizType, pageNumber, pageSize, channelId, searchScope }` |

dispatch enum `OPEN_TOPIC_WIN_SEARCH`。

---

## 用例集合八：互动卡片（投票 / 平均分，走 `api`/AppHttpService）

> 全部 `this.api.post$`（默认网关，无 `/api/cses` 前缀），path `/vote/*` `/average/*`。

| 方法 | path | 关键 body |
|---|---|---|
| `createVote`（:1032） | `/vote/createVote` | `VoteCreateParams{fromUserId,fromUserName,title,content,votes,isReal,finishTime,options[],orgIds[],joinCount,source?}` |
| `vote`（:951） | `/vote/vote` | `{ id, postId?, indexes:string[] }` |
| `readVote`（:960） | `/vote/readVote` | `{ id }` |
| `closeVote`（:978） | `/vote/closeVote` | `{ id }` |
| `deleteVote`（:969） | `/vote/deleteVote` | `{ id }` |
| `publishAverageScore`（:1023） | `/average/publish` | `AveragePublishParams{title,content,maxScore,minScore,isDelMaxMin,isAnonymous,cutoff,members[],hasDecimal?,decimalPlaces?,source?}` |
| `averageAttend`（:987） | `/average/attend` | `{ id, score, postId? }` |
| `readAverageScore`（:996） | `/average/read` | `{ id }` |
| `closeAverageScore`（:1005） | `/average/close` | `{ id, postId? }` |
| `deleteAverageScore`（:1014） | `/average/delete` | `{ id }` |

---

## 用例集合九：书签（message.service，imHttp）

| 方法 | path（+`/api/cses`） | body |
|---|---|---|
| `createBookMark`（:737） | `/post/bookmark/create` | `{ channelId, postIds:string[], userId }` |
| `deleteBookMark`（:750） | `/post/bookmark/delete` | `{ postId, userId }` |
| `queryBookMarks`（:763） | `/post/bookmark/load` | `{ channelId, userId, pageSize, pageNumber?, offset? }` |

dispatch enum `BOOKMARK_MESSAGE / REFRESH_BOOKMARKS`。

---

## 用例集合十：通知/待办（messageNotice / Rust todo）

### UC-10.1 待办列表
**链路**：Rust `http.rs:68` `POST /api/cses/posts/queryTodoList`；推送 `im:todo:updated`（`ipcEvent.service.ts:246`）→ `getTodoUpdated$` → `dispatch(INIT_TODO_LIST_DATA)`。
**前端 enum**：`SEND_TODO_LIST_DATA / INIT_TODO_LIST_DATA / DELETE_TODO_ITEM / UPDATE_TODO_ITEM`。

### UC-10.2 系统通知
`handlePostReceived`/`handlePostUpdated` 内 `messageNoticeService.createSysNotification(message, isUpdate)` + `createTodoListData(data)`（无独立 HTTP，由 WS 帧触发）。

---

## 附：服务端 WS 帧 → Rust handler → Tauri 事件 → Angular dispatch 全表

| 服务端 WS `action` | Rust handler（router.rs） | Tauri emit（`im:*`） | Angular `getXxx$()` | dispatch（MessageEventDispatch） |
|---|---|---|---|---|
| `post` | `post::handle` | `im:post:received` | `getPostReceived$` | `POST` |
| `post_update` | `post::handle_update` | `im:post:updated` | `getPostUpdated$` | `POST_UPDATE` |
| `posts_update` | `post::handle_batch_update` | `im:post:batch-updated` | `getPostsUpdated$` | `POSTS_UPDATE` |
| `post_read` | `post::handle_read` | `im:post:read` | `getPostRead$` | `POST_READ` |
| （sync type=3） | Rust gate | `im:post:deleted` | `getPostDeleted$` | `POST_UPDATE{action:'post_deleted'}` |
| （sync type=6） | Rust gate | `im:channel:read_echo` | `getReadEcho$` | `POST_READ`（actor==self） |
| `increment_channel` | `channel::handle_increment` | `im:post:increment` / `im:post:increment-started` / `im:post:increment-empty` / `im:post:increment-failed` / `im:channel:increment` | `getPostIncrement$` 等 | `POST_INCREMENT*` / `INCREMENT_CHANNEL` |
| `increment_channel_end` | `channel::handle_increment_end` | `im:channel:update` | `getChannelUpdate$` | `UPDATE_CHANNEL` |
| `update_channel` | `channel::handle_update` | `im:channel:update` | `getChannelUpdate$` | `UPDATE_CHANNEL`/`UPDATE_TOPIC` |
| `channel_created` | `channel::handle_create` | `im:channel:created` | `getChannelCreated$` | `CREATE_DIALOG`/`CREATE_TOPIC`/`UPDATE_CHANNEL` |
| `channel_close` | `channel::handle_close` | `im:channel:closed` | `getChannelClosed$` | `DELETE_DIALOG` |
| `channel_member_update` | `channel::handle_member_update` | `im:channel:member-updated` | `getChannelMemberUpdated$` | `UPDATE_CHANNEL` |
| `update_channel_member_nickName` | `channel::handle_member_nickname_update` | `im:channel:memberNickname` | `getChannelMemberNickname$` | `MEMBER_NICKNAME_UPDATE` |
| `post_schedule_created` | `schedule::handle_created` | `im:channel:schedule-created` | `getChannelScheduleCreated$` | — |
| `post_schedule_canceled` | `schedule::handle_canceled` | `im:channel:schedule-canceled` | `getChannelScheduleCanceled$` | — |
| `hello` | `hello::handle` | `im:channels:loaded` | `getChannelsLoaded$` | — |
| （Rust gate too_long） | `post.rs` | `im:sync:too_long` | `getSyncTooLong$` | → `ImSyncService.onRustTooLong` |
| （连接建立） | — | `im:connection:established` | — | connectionId 注入 |

---

## 附：完整用户用例清单汇总表

| 用例集合 | 用例 | HTTP 接口（imHttp 自动拼 `/api/cses`） | WS 事件 |
|---|---|---|---|
| 消息收发 | 发文本/富文本 | POST /posts/create | posted → im:post:received |
| 消息收发 | 发文档 | POST /posts/create | im:post:received |
| 消息收发 | 发图片/文件 | （上传）+ POST /posts/create | im:post:received |
| 消息收发 | 重发 | POST /posts/create | im:post:received |
| 消息收发 | 撤回 | POST /posts/revoke | posts_update/post_deleted → im:post:batch-updated / im:post:deleted |
| 消息收发 | 编辑 | POST /posts/create(或/posts/update) | post_edited → im:post:updated |
| 消息收发 | 转发 | POST /posts/createPosts | im:post:received |
| 消息收发 | 快捷回复 | POST /posts/quickReply | im:post:* |
| 消息收发 | 加急/加急已读 | POST /posts/urgentPost · /posts/urgentConfirm | im:post:updated |
| 消息收发 | 定时消息 | POST /posts/createSchedule · /posts/cancelSchedule · /posts/getSchedule | im:channel:schedule-created/canceled |
| 历史加载 | 切群首屏(兜底) | POST /posts/getLatestPost | — |
| 历史加载 | 上拉更早(Rust) | POST /posts/postContext | — |
| 历史加载 | 按 postId 定位 | POST /posts/getPostsAfterIndex | — |
| 历史加载 | 一级/二级回复 | POST /posts/getReplies · /posts/getReplyBranch | — |
| 已读未读 | 会话已读 | POST /channels/view | type=6 → im:post:read / im:channel:read_echo |
| 已读未读 | 单条已读 | POST /post/read | — |
| 已读未读 | 模板已收到 | POST /post/templateReceived | — |
| 离线同步 | hello 根群增量 | POST /channels/load/increment | increment_channel(+_end) → im:post:increment* / im:channel:increment/update |
| 离线同步 | 按需 sync notify | POST /channel/sync/notify | （响应内 entries 自处理） |
| 离线同步 | too_long 重拉 | POST /posts/getLatestPost | im:sync:too_long（触发） |
| 离线同步 | 心跳 gap 补偿 | （ping/pong piggyback，Rust 组装） | ping/pong |
| 离线同步 | 独立窗口兜底 | POST /channel/load/incrementByChannelId | — |
| 频道管理 | 创建群 | POST /channel/create | channel_created → im:channel:created |
| 频道管理 | 创建话题 | POST /posts/makeTopic | channel_created → im:channel:created |
| 频道管理 | 关闭/退出 | POST /channel/close · /channel/member/leave | channel_close → im:channel:closed |
| 频道管理 | 改名/公告/描述/导向/权限/置顶/通知 | POST /channel/change/{displayName,notice,purpose,orient,permission,top} · /channel/member/change/notify | update_channel → im:channel:update |
| 频道管理 | 消息置顶 | POST /channel/add/postPinned · /channel/remove/postPinned · /channel/load/postPinned | — |
| 频道管理 | 群公告 | POST /post/announcement/{list,save,delete,read} | — |
| 频道管理 | 在线状态/分组 | POST /channel/onlineStatus · /modules/getAll | — |
| 成员管理 | 拉/踢人 | POST /channel/member/change | channel_member_update → im:channel:member-updated |
| 成员管理 | 设/撤管理员 | POST /channel/add/manger · /channel/remove/manger | — |
| 成员管理 | 改群昵称 | POST /channel/member/change/nickname | update_channel_member_nickName → im:channel:memberNickname |
| 成员管理 | 成员快照/全量 | POST /channel/member/snapshot · /channels/member/byIds | — |
| 搜索 | 全局/会话/分类/话题搜索 | POST /Im/search/{global,searchByChannel,searchByChannelWithCategory,searchByChannelWithCategoryTopic}（走 api，无 /api/cses） | — |
| 互动卡片 | 投票 CRUD | POST /vote/{createVote,vote,readVote,closeVote,deleteVote}（走 api） | im:post:updated |
| 互动卡片 | 平均分 CRUD | POST /average/{publish,attend,read,close,delete}（走 api） | im:post:updated |
| 书签 | 创建/删除/查询 | POST /post/bookmark/{create,delete,load} | — |
| 通知/待办 | 待办列表 | POST /posts/queryTodoList（Rust） | im:todo:updated |
| 通知/待办 | 系统通知 | （WS 帧触发，无 HTTP） | im:post:received/updated |

> bot 用例集合：message-v3 service 层未见独立 bot 端点；bot token 认证（X-Bot-Token）属服务端能力，客户端无对应 service 方法，故本梳理无 bot 用例（如需覆盖请从服务端 `csesapi` 侧梳理）。
