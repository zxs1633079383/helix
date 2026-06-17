# 客户端 message-v3 组件与 UI 渲染逻辑梳理

> 范围: `cses-client/src/pages/message-v3/`（components/ + directives/ + pipe/ + consistency-checker/ + message.component.{ts,html} + type.d.ts + event.type.ts + enum/enum.ts）。
> 目的: 把「列表如何渲染 / 乐观更新与 WS 回声如何对账 / 已读未读计数如何反映 / 虚拟滚动分页增量如何驱动渲染 / 各消息类型如何呈现」映射回它依赖的数据来源（HTTP 响应字段 / WS 事件字段 / 本地 SQLite 列）。
> 字段引用均 verbatim 摘自源码。GitNexus 索引偏旧，本文以 Read 活体源码为准。

---

## 0. 组件树 / 容器布局

`message.component.html`（`<message-container>` 根，`MessageContainerComponent`）三栏：

```
#message-container [class.layout-push]=layoutMode()==='push' [class.dialog-compact]=dialogCompact()
├── .dialog-container        左栏（会话列表）
│   ├── <dialog-header>      标题/主题切换 (switchTheme)
│   ├── <dialog-search>      会话搜索 (dialogClick)
│   └── <dialog-list>        会话列表项 (dialogClick) → onDialogItemClick → dialogSwitchService.requestSwitch
├── .chat-container *ngIf="!!messageGlobalServer.currentDialog else noCurrent"   中栏（聊天主区）
│   ├── <chat-header>        会话头 (itemClick)→onHeaderClick
│   └── .chat-wrap
│       ├── .left-content
│       │   ├── <chat-content #chatContent [inputHeight]="chatInputHeight">   ← 消息列表渲染核心
│       │   ├── <chat-input #chatInput (expand)="onChatInputExpand()">
│       │   └── <top-message *ngIf="topMessages.length" [list]="topMessages">  置顶消息条
│       └── .right-drawer [class.show]="isShowDrawer"   右抽屉（按 curDrawer 切：setting/pin/reply/search）
│   ├── .bookmark-wrap       书签面板（message-bookmark-wrap，分页 scroll）
│   └── @if(isExpandEditorOpen()) <message-expand-editor>  全屏文档编辑器
└── #noCurrent              空态背景图「一念智慧开，好运自然来」
```

- 响应式布局: `createResponsiveLayoutState()` → `layoutMode()`（>=1440 `push` flex 兄弟 / <1440 overlay）+ `dialogCompact()`（<1200 会话列表收窄）。
- `chatInputHeight` 由 `ngAfterViewInit` 的 `ResizeObserver`（rAF debounce, `inputResizeRafId`）观测 `.chat-input` 高度写入，作为 `[inputHeight]` 喂给 chat-content 计算 `contentHeight`。**强约束**: 该 ResizeObserver 回调禁止同步写 DOM size，必须 rAF debounce，否则触发 Chrome ResizeObserver loop 上限丢 notification → virtuoso viewportHeight 卡 0 → 列表清空。
- 错误上报: `errorReporter.register(ModalSink)` + `errorBridge._instance` 静态门面。
- 组件级 `MessageDialogServer` 必须在 ngOnInit `ipcEventHandlerService.registerMessageDialogService` 注册到 root 单例（root injector get 不到组件级 provide），ngOnDestroy `unregister`，否则 logout→login 重建窗口期首个 POST 到达拿不到实例 → WS 写入被跳过。

---

## 1. 消息列表渲染（chat-content / 虚拟滚动）

### 1.1 虚拟滚动器（`@ccc/angular-virtuoso` `<ng-virtual-list #virtuoso>`）

`chat-content.component.html` 关键绑定（verbatim）:

```html
<ng-virtual-list #virtuoso
    [data]="messages"
    [computeItemKey]="computeItemKey"          // 返回 temporaryId
    [itemSizeEstimator]="itemSizeEstimator"
    [increaseViewportBy]="{ top: 3000, bottom: 3000 }"
    [minOverscanItemCount]="{ top: 12, bottom: 12 }"
    [followBehavior]="jumpState ? 'none' : 'stickToBottom'"
    [firstItemIndex]="firstItemIndex"
    [enableHeightCache]="true"
    [heightCacheStabilityDelay]="500"
    [resizeAnchor]="'bottom'"
    [boundaryTriggerMode]="'wheel'" [boundaryRetriggerDistance]="60"
    (startReached)="onStartReached($event)"    // 上拉历史
    (endReached)="onEndReached($event)"        // 下拉更新
    (isScrolling)="onScrolling($event)"        // 已读上报轨 A
    (atBottomStateChange)="onAtBottomChange($event)">
```

item 模板按类型二分发：

```html
@if (NOTICE_TYPES.includes(item.type)) {
    <message-system [message]="item"></message-system>
} @else {
    <message-item [message]="item" (messageClick)="onMessageClick($event)"></message-item>
}
```

`NOTICE_TYPES`（enum.ts）= `[TIME, NOTICE, MEETING_NOTICE, SYSTEM_KR_EDIT, SYSTEM_KR_STATE_EDIT, SYSTEM_KR_VOTE, SYSTEM_KR_PUBLISH, SYSTEM_MONTH_ORIENT_SHARE]` → 走 `<message-system>`（系统/分隔条），其余走 `<message-item>`（气泡）。

### 1.2 SWR 首屏渲染策略（Stale-While-Revalidate）

`chat-content` 继承 `ChatContentBaseComponent`（Template Method）。`initMessage`:
1. `getCachedMessagesByChannelId(id)` 非空 → **立即赋值 `this.messages` 渲染**（`chat.render.cache`），`loadDialogMessagesIfNeeded` `.then()` 后台 revalidate 完成再赋值触发差异合并（`chat.revalidate.start/done`）。
2. 空缓存才走 `await loadDialogMessagesIfNeeded`（`chat.init.needLoad`）。
3. revalidate 失败仅 `console.warn`，保留缓存内容不回退。
4. `isStaleInitMessage(initToken)` 防快速切群老回调覆盖。

### 1.3 引用稳定化（差异合并，维稳滚动位置 + OnPush）

`set messages` setter 内 `stabilizeMessageRefs(_, _messageRefCache)`，key=`temporaryId`：未变更项保留旧引用。`isMessageRenderEqual` 决定「是否变」，**当前覆盖字段**: `temporaryId / message / type / isShowAvatar / revoke / props / userSnapshot / sendStatus / quickReply / uploadProgress / readBits / updateAt`，+ 2026-05-23 补的 6 壳层字段 `replyMessages / repliedMessage / viewers / expediteRecipients / expediteMap / expediteSender`。新增影响高度的字段必须同步加入，否则 stabilizer 误复用旧 ref。

### 1.4 时间分隔 + 头像折叠

- `insertTimeMessages()` 在跨日处注入 `type: TIME` 条（走 message-system 渲染「时间分隔」）。
- `message.isShowAvatar`（ChatMessage 字段，连续消息优化）: false 时 message-item `[class.hide-avatar]`，不渲染头像与 `.message-header`。

### 1.5 分页 / 增量驱动渲染

| 触发 | 事件/方法 | 数据来源 |
|------|-----------|----------|
| 上拉历史 | `(startReached)`→`onStartReached` prepend | 本地 DB + Rust `POST_INCREMENT`（向上加载） |
| 下拉更新 | `(endReached)`→`onEndReached` append | 同上（向下加载） |
| 跳转定位 | `JumpToItemIntent` / `onJumpToMessage` | 目标在 oldMessage 则先恢复原列表再定位 |
| 滚到底 | `requestScrollToBottom()`→`StickToBottomIntent({behavior:'auto'})` | **禁止 `'smooth'`**（scrollHeight 渲染期变化导致点 3-4 次才到底） |

- **Intent-based API**: 所有滚动/跳转走 `virtuoso.dispatch(new XxxIntent(...))`，不直接命令式调用。
- `firstItemIndex` 配合 prepend 维持滚动锚点；`[followBehavior]` 在 jumpState 时切 `'none'`。
- `latestPostsStatus`（DialogItem 字段 `'idle'|'loading'|'loaded'|'empty'|'failed'`）驱动空态 UI：messages.length===0 时区分「消息加载中 / 暂无消息 / 消息加载失败(可重试)」，普通 DB 空查不直接显示最终空态。来源: Rust 异步拉最近消息状态。

### 1.6 height cache 三层失效（异步任务改变气泡高度）

- 第 1 层（最权威）: setter `collectChangedTemporaryIds(prev,next)` 累进 `pendingHeightCacheIds`，由 handler `detectChanges()` 后 `flushPendingHeightCache()`（入口 A）+ setter rAF 兜底（入口 C）flush。
- 第 2 层: handler belt-and-braces `invalidateHeightCacheByTemporaryId`。
- 第 3 层: 内容组件 `effect()` 监听异步状态变化 dispatch `MESSAGE_HEIGHT_CHANGED {temporaryId}` + scope（链接预览异步出现 / 投票推送 / 公告 ack / message-img 上传完成服务端 width/height 回填 / message-status sendStatus 与 snapshotReady 切换 / message-item 壳层 `_shellHeightWatcher`）。
- chat-content / standalone `@CoreEventListener(MESSAGE_HEIGHT_CHANGED, { scope })` 收口 `virtuoso.clearHeightCache`。

### 1.7 「N 条新消息」/ 跳转按钮 / 未读跳转（chat-content.component.html `.jump-btn-box`）

| 按钮 | 条件 | 数据来源 |
|------|------|----------|
| 「你有 N 条加急 / N 条@ 未读」 | `todoPostIds.length` | `urgentPostList`(DialogItem) / `mentionList`，`syncTodoPostIds` 监听 `UPDATE_CURRENT_DIALOG` 同步 |
| 「N 条未读消息 点击跳转」 | `entryUnreadCount>0 && entryChannelId===currentDialog.id` | `entryUnreadCount`/`entryUnreadPostId` 在 `initMessage` 一次性 snapshot 自 `dialog.unreadCount`/`unreadPostId` |
| 「回到最新」 | `isShowJumpBackButton` / `isShowJumpBottom` | virtuoso 滚动位置 |
| 「N 条新消息 点击查看」 | `newMessageCount>0 && !isShowJumpBackButton` | 不在底部收到新消息时 `virtuoso.isAtBottom()` 判定累加；`(atBottomStateChange)` 回底清零 |

- `newMessageCount` 累加只算非 TIME 类型增量（避免 insertTimeMessages 虚高）；NOTICE/系统消息**按产品语义计入未读，禁止过滤**。
- `onJumpToUnreadMessage` 双源读 postId: `entryUnreadPostId`(snapshot) → `currentDialog.unreadPostId`(live) 兜底。
- **`dialog.unreadPostId` 端到端契约**: 服务端 socket 字段名 `unReadPostId`（大写 R），Rust 三处必须同步 rename: `types.rs::Channel #[serde(rename="unReadPostId")]` / `channel_state.rs::channel_update_program` / `common.rs::build_dialog_map_from_local`。

---

## 2. 乐观更新 与 WS 回声对账

### 2.1 关键事件（enum.ts `MessageEventDispatch`，CoreEventManager 派发，scope 隔离主窗口/独立窗口）

| 事件 | 触发源 | 渲染动作 |
|------|--------|----------|
| `SEND_MESSAGE` / `RE_SEND_MESSAGE` | 本地发送/重发 | 乐观插入（`sendStatus` 临时态） |
| `UPDATE_MESSAGE_WITH_SEND` | 发送 HTTP 回声 | `handleUpdateMessageWithSendResult` patch 落定 id/readBits |
| `POST` / `POSTS` | WS 新消息（单/批） | `handlePostEventResult` append + 计数 |
| `POST_READ` | WS 已读回声 | @消息清理 + readBits 更新 |
| `POST_UPDATE` / `POSTS_UPDATE` | WS 编辑/撤回/投票推送 | `handlePostUpdateResult` patch + height cache 失效 |
| `POST_INCREMENT*` | 离线同步增量 | started/empty/failed/数据 |

### 2.2 乐观态 → 落定（sendStatus 状态机）

`ChatMessage.sendStatus`（type.d.ts/event.type.ts，本地 SQLite `send_status` 列）取值与 message-status 渲染（见 §3.1）:
- `local` / `uploading` → 12px loading dot（乐观插入中 / 文件上传中）
- `unSend` → 红色感叹号「点击重新发送」(`onReSend` `@debounce(1000)`)
- `undefined`/`sended`（默认）→ 「N 人未读 / 全部已读」

发送链路（isMe）≥3 个晚于 stabilityDelay 的高度变化点（强约束清单）:
- ① `props.files` 服务端 `width`/`height` 回填（图片）→ aspect-ratio 重算 → message-img `_uploadTransitionEffect` dispatch
- ② `sendStatus` `uploading/local/unSend`→默认 → message-status `@switch` 切文字 → `_heightWatcher` dispatch
- ③ `snapshotReady` false→true（成员快照异步加载）→ `@if(snapshotReady())` 渲染 unread-members → dispatch

### 2.3 POST 事件 detectChanges 同步约束（强约束）

`handlePostEventResult` / `handlePostUpdateResult` / `handleUpdateMessageWithSendResult` 三处 `set messages` 后**必须**显式 `cdRef.detectChanges()`。setter 内部仅 `markForCheck`；CoreEventManager 出 NgZone 派发时若不显式 CD，virtuoso `listState` computed 读到旧 data + 新 viewport 返回 `ListState.empty()` 清空 DOM（症状 `content.messages.dom.check deadlocked:true`）。

### 2.4 self-message 本地 id 为空陷阱（关联 MEMORY）

用户自己发的消息本地 SQLite `id` 列为空串（仅 `temporaryId`）。任何 message.id-keyed 逻辑须以 temporaryId 为主键；锚点须 `messages.find(m=>m.id)`。`computeItemKey` 返回 `temporaryId` 正是规避此坑。

---

## 3. 已读 / 未读 / 计数 → UI

### 3.1 message-status（`<message-status [message] (itemClick)="onReSendMsg()">`）

渲染门槛 `shouldShowMessageStatus()`（message-item.ts）: 非 mini、`isMe()`、非 `isComplete`(会话终止)、非 `isReadStatusHidden()`（文本模板/群公告自带内嵌未读展示）。

`message-status.component.html` 按 `message().sendStatus` `@switch`（见 §2.2），`@default` 段渲染未读:

```html
@default {
  <div class="message-status-unread">
    @if (snapshotReady()) {
      <unread-members [people]="unReadMembers()" [readPeople]="readMembers()"
        [enableTabs]="true" [unreadTitle]="'未读'" [readTitle]="'已读'" [dialogType]="dialogType">
        <span class="message-status-unread-count">{{ unreadText() }}</span>
      </unread-members>
    }
  </div>
}
```

### 3.2 readBits 位图契约（核心数据来源）

- `ChatMessage.readBits`（本地 SQLite `read_bits` 列；服务端 wire 字段同 `readBits`）: 字符串，`'0'`=未读 `'1'`=已读，**按位置与 `allMembers` 严格一一对应**（`readBits.length === allMembers.length`）。
- 私聊(D)显示「已读/未读」，群聊显示「全部已读/N 人未读」(`unreadText()`)。
- 组件不在前端剔除发送者——按契约发送者位必为 `'1'`（服务端 SavePosts 写入时预置 sender 已读位，mattermost `6b053f0b`）。
- `readResult` 一次性按位图算 `unread`+`read` 两组 DialogMember（同一份 allMembers 反查），保证 `unreadIds+readIds=allMembers`。
- 发送者误入未读两条排查线: 线 A 成员快照缓存未刷新错位（`refreshMembersSnapshot`）；线 B 服务端建 bitmap 未预置发送者位（已修 `6b053f0b`）。

### 3.3 加急（urgent）UI（message-item）

- `isUrgent()` = `Array.isArray(msg.expediteRecipients) && msg.expediteRecipients.length>0`。
- 渲染: `.message-content-urgent`（闪电图标）；发送者(`expediteMap.sender.id===userInfo.userId`)显示「X 加急给 [接收人列表]」(`expediteRecipients` 逐个 `[ngClass]={isread:!!item.status}`)；被加急者(`isBeurged` = `expediteRecipients.find(r.id===self && r.status===0)`)显示「已读」按钮 → `onPostUrgentConfirm` → `messageServer.onPostUrgentConfirm({channelId,postId})`。
- 数据来源字段（type.d.ts Message.MessageItemData / event.type ChatMessage）: `expediteMap.recipients[uid]{name,status,teamId}` / `expediteMap.sender{id,name,teamId}` / `expediteSender` / `expediteRecipients[]{id,name,status,teamId}`。本地 SQLite `expedite_map` 列；`expediteRecipients`/`expediteSender` 一致性检查标 `compare:'ignore'`（无对应 DB 列，运行时派生）。
- 打开加急弹窗 `OPEN_EXPEDITED_USERS`(message.component.ts): 解构 `{createAt, readBits, id, viewers}`，未读人 = `allMembers.filter((_,i)=>readBits[i]==='0')`，再排除已加急 `expediteRecipients.map(r=>r.id)`，`/^1+$/.test(readBits)` 全已读则提示不需加急。

### 3.4 会话级未读数（DialogItem，左栏 + 跳转按钮）

DialogItem 字段（type.d.ts）: `unreadCount` / `unreadPostId` / `mentionCount` / `mentionCountRoot` / `mentionList[]` / `mentionUser` / `hasUrgentPost` / `urgentPostList[]` / `urgentCount`。

> 关联服务端决策（mattermost）: 未读数走 count 域（`total_msg_count_all + phantom`），seq 只做 sync cursor；客户端只消费 `unreadCount`，不自行用 maxSeq-lastReadSeq 推算。

### 3.5 已读上报双轨（onPostRead / inViewMsgRead）

- 轨 A 滚动驱动: `(isScrolling)=false`→`onScrolling`→`inViewMsgRead()`。
- 轨 B 代码主动落位补发: 所有 `virtuoso.scrollToBottom` 入口后**必须显式** `this.inViewMsgRead()`（已覆盖 `requestScrollToBottom`/`scrollToNewMessages`/`jumpToBottom`/首屏滚底/转发完成）。程序化滚底不触发 isScrolling 回调，不补发则发消息后已读不上报。靠 `@debounce(300)` + `unReadCheck` readBits 过滤保证幂等。

---

## 4. 各消息类型呈现（type → 组件分发）

### 4.1 分发机制

- 二分发（chat-content 模板）: NOTICE_TYPES → `<message-system>`；其余 → `<message-item>`。
- message-item 内 **CdkPortalOutlet 动态加载内容子组件**: `MessageRegistryService.resolve(message.type)`（topic 消息走 `resolve(TOPIC)`），兜底 `resolve(DEFAULT)`，通过 `MESSAGE_TOKEN` InjectionToken 注入 message。
- `loadComponent()` 触发: `ngOnChanges` 中 type 或 topic 变化重新加载；否则仅更新子组件 instance.message + `messageRef.set()`。
- `isMessageDataInvalid()` 关键数据缺失走兜底: IMAGE 查 `props.files.length`，FILE/AUDIO/VIDEO/VOICE 查 `props.file`，VOTE 查 `props.vote`，AVERAGE_SCORE 查 `props.averageScore`，TASK_CARD 查 `props.taskCard`。

### 4.2 messageType 枚举（enum.ts，verbatim 注意值与 key 不一致处）

`TEXT/IMAGE/FILE/VIDEO/AUDIO/VOICE`，`SYSTEM: 'SYSTEN'`（**值是 SYSTEN 拼写**），`TEMPLATE/GREETING/SOLITAIRE/ANNOUNCEMENT/NOTICE/MEETING_NOTICE/DOCUMENT/TIME/TOPIC/MULTIPLY`，`SYSTEM_KR_EDIT/SYSTEM_KR_STATE_EDIT/SYSTEM_KR_VOTE/SYSTEM_KR_PUBLISH/SYSTEM_MONTH_ORIENT_SHARE`，`VOTE/AVERAGE_SCORE/LOTTERY/TASK_CARD/DEFAULT`。

### 4.3 内容子组件（message-content/ 目录）

| type | 组件 | 数据来源 |
|------|------|----------|
| TEXT | message-text-content | `message`(文本) + 链接预览 `props.linkPreview`（异步） |
| IMAGE | message-img-content | `props.files[]`（1-9 图网格 + OCR + 服务端 width/height 回填） |
| FILE | message-file-content | `props.file`（预览/下载） |
| VIDEO | message-video-content | `props.file`（缩略图 + 独立窗口播放） |
| AUDIO | message-audio-content | `props.file` |
| VOICE | message-voice-content | `props.file`（SVG 波形 + 全局互斥播放） |
| MULTIPLY(合并转发) | message-merge-content | `props`（合并消息列表） |
| TOPIC | message-topic-content | `message.topic`（topic 非空时优先解析） |
| ANNOUNCEMENT | message-announcement-content | `props`（公告确认 ack 状态切换） |
| VOTE | message-vote-content | `props.vote` |
| AVERAGE_SCORE | message-average-score-content | `props.averageScore` |
| TEMPLATE | message-template-content | `props.template{type:'TEXT'|'IMAGE'...}` |
| TASK_CARD | message-task-content | `props.taskCard`（parse+normalize 脏数据） |
| DEFAULT/未知 | message-default-content | 兜底 |

- Footer Slot: 子组件 `@ViewChild('footerTpl',{static:true}) override footerTemplate` 声明全宽 footer，message-item portal attach 后读 `ref.instance.footerTemplate` → `contentFooterTemplate` signal 在 `.message-content` 外渲染。当前用: 模板消息、公告消息。
- props 持久化统一基类 `MessageContentBaseComponent`: `updateProps(patch)` / `persistLinkPreview(vm)`（写 `props.linkPreview` + Tauri `store:updateItem`）/ `parsePropsField`。禁止组件内 inline 调 `store:updateItem`。

### 4.4 系统/通知消息（message-system，`generateMessage` 文案合成）

`message-system.component.html` 按 `type` `[ngSwitch]`:
- `MEETING_NOTICE`/`NOTICE` → 文案 `content`（=`generateMessage(message, userId, allMembers)`）
- 默认(TIME 等) → `.message-system-time`（时间分隔条）
- `SYSTEM_KR_EDIT` → 「{props.userName}填写了{props.krCount}条kr」
- `SYSTEM_KR_STATE_EDIT` → 投票图标 + `EState[props.krState]`（Writing/Voting/Completed/WritingCompleted/Terminated）
- `SYSTEM_KR_PUBLISH` / `SYSTEM_MONTH_ORIENT_SHARE` → `message.message`
- `SYSTEM_KR_VOTE` → 「{props.userName}投了{props.krCount}票」

### 4.5 回复消息（reply-root-message / reply-sub-message）

- 气泡内引用块（message-item.html `.message-content-reply`，`shouldShowRepliedMessage()` 守卫）: 显示 `resolveRepliedUserDisplayName()` + `repliedMessage.message`；`repliedMessage.isRevoke` 则「消息已撤回」；点击 `jumpToMessage()` dispatch `JUMP_TO_MESSAGE` with `message.replyId`。数据来源 `ChatMessage.repliedMessage{repliedUserId,repliedUserName,message,isRevoke,type,viewers}`（本地 `replied_message` 列）。
- 气泡底部回复预览（`.message-content-replyme`，`replyMessages()` computed，slice 0,3）: 每条 `reply.replyUserName: reply.message`，>3 显示「全部回复（N）」→ `onOpenReply` dispatch `OPEN_REPLY_DRAWER`。数据来源 `ChatMessage.replyMessages{[id]:{replyUserId,replyUserName,repliedUserId,repliedUserName,message,viewers}}`（本地 `reply_messages` 列）。
- 回复抽屉（drawer reply 内）: reply-root-message + reply-sub-message 完整回复树，`replieChildren` + `loadMoreReplieChildren`（hasMore 分页），同样 CdkPortalOutlet 加载内容。
- `replyMessages()` 可见性过滤: `viewers` 含当前用户 / 含 `'all'` / 为空 / `replyUserId` 是当前用户 → 可见。

### 4.6 撤回（revoke）

- message-item.html: `@if (!message.revoke) { 正常气泡 } @else { <p class="message-item-revoke">{{revokeUser()}}撤回了一条消息 @if(showReedit()) <span (click)="reEdit()">重新编辑</span> }`。
- `revokeUser()`: 自己→「您」；私聊→「对方」；群聊→`channelMemberResolver.resolveDisplayName`。
- `showReedit()` = `type==='TEXT' && isMe()`；`reEdit()` dispatch `RE_EDIT_MESSAGE` with `message.message` + scope。
- `revoke` 字段来源: 本地 SQLite `revoke` 列 / WS `POST_UPDATE` 推送。reply 组件同样 `*ngIf="!message?.revoke; else revoke"` → 「已撤回」。

### 4.7 编辑（edit）

编辑走服务端 `POST_UPDATE` → `handlePostUpdateResult` patch `message`/`props`/`updateAt` + height cache 失效。`updateAt`（本地 `update_at` 列）参与 `isMessageRenderEqual` 比较触发重渲染。

### 4.8 emoji 快捷回复（quickReply）

- message-item.html `.message-content-quickreply`: 遍历 `message.quickReply`，每项 emoji + count(`userIds.length`)，`[class.active]=userIds.includes(userInfo.userId)`，popover 显示 `quickreplyContent(users)`，点击 `onQuickReplyClick` → `messageServer.sendQuickRelayMessage({emoji,userId,postId,teamId})`。
- 数据来源 `ChatMessage.quickReply[]{emoji,userIds,users[]{userId,userName}}`（本地 `quick_reply` 列）。

---

## 5. 发送者身份 / 显示名解析（强约束）

- `.sender-name` = `resolveSenderDisplayName(message)`: 优先 `allMembers()` 查 userId 命中 live member，再 `resolveDisplayNameWithFallback(live, userSnapshot, channelType)`——live 解析为「未知用户」时回退 userSnapshot。群聊/开放频道(P/O)优先 `nickName`，私聊(D)回退 `userName`。头像 `[alt]` 同源（a11y）。
- 身份标签 `senderDisplay()`（`utils/sender-display.ts resolveSenderDisplay`）四 kind: `guest`(senderTeamId 多重兜底后仍空)/`cross-company`(senderTeamId≠currentDialog.teamId,只显示公司名)/`internal`(同公司,显示部门/职位)/`none`。
- **`senderTeamId` 多重 fallback**: Rust Message 表 `team_id` 已移除（DB 读出 `msg.teamId` 恒 undefined），按 `msg.teamId → userSnapshot.teamId → userSnapshot.companyId → member.teamId → member.companyId` 兜底。`messageSend.service.ts` 发消息时把 teamId 写入 userSnapshot 作稳定兜底来源。
- 密聊首位查看者 `firstViewUser()` / `firstViewUserDisplay()` 同套规则（仅 isSecretChat 时）。
- 密聊判定 `isSecretChat()`: `viewers` 非空且非 `['all']` 且 viewer 集合 ≠ allMembers 集合。

---

## 6. 数据来源映射汇总（page 字段 ← 本地 SQLite 列 ← 服务端 wire）

来源 `consistency-checker/schemas/message.schema.ts`（idField `temporary_id`↔`temporaryId`），verbatim DB列↔page字段:

```
id↔id  channel_id↔channelId  user_id↔userId  type↔type  message↔message
send_status↔sendStatus  create_at↔createAt  update_at↔updateAt
reply_id↔replyId  reply_root_id↔replyRootId  thread_id↔threadId
reply_count↔replyCount  read_bits↔readBits  revoke↔revoke
user_snapshot↔userSnapshot(json)  props↔props(json)  viewers↔viewers(json)
mentions↔mentions(json)  expedite_map↔expediteMap(json)
replied_message↔repliedMessage(json)  reply_messages↔replyMessages(json)
topic↔topic(json)  quick_reply↔quickReply(json)
(无DB列,运行时派生 compare:ignore) expediteRecipients / expediteSender
```

- WS 推送 wire 契约（关联 MEMORY im_sync_wire_contract）: HTTP camelCase / WS snake_case 两铁律；坑位 `createAt`(非 createdAt) / `eventType` 数字枚举 / `omitempty`。
- DialogItem 离线同步 cursor: `lastEventSeq`（后端 d4748d62 引入，该 channel 当前服务端 max event_seq）；`memberCount`（来自 socket increment_channel，allMembers().length≠memberCount 触发 `/channels/member/byIds` 重拉）；`subtopicsLoadedAt`（Rust 权威源，空则调 `/channels/load/increment`）。

---

## 7. 辅助件

- **directives/message-context-menu.directive.ts**: `messageContextMenu=".message-content"` 绑定右键菜单到气泡，`[messageGetter]="getMessageByTemporaryId"`，`ToolbarRegistry.resolveFlat()` 解析菜单项 → `contextMenuService.currentMenuItems()` signal 驱动 `<nz-dropdown-menu #contextMenu>` 渲染。emoji 子菜单内嵌 `<message-emoji>`。
- **pipe/username.pipe.ts** (`| username:teamId`): `userId`→`tauriService.invoke('UCenter:getUser',{userId,companyId:teamId||''})`→`userInfo.username`，Observable，error 兜底空串。（头像用 `| avatar` pipe，来自 `@ccc/cses-common`。）
- **consistency-checker/**: DB-Page 一致性检查（dev only），`init.ts` 在 message.component ngOnInit 调 `initConsistencyChecker`。Core/UI 分离（Facade ConsistencyChecker + Strategy DiffCalculator + Registry SchemaRegistry + Adapter DbSource/PageSource + TimelineStore）。`window.__CSES_DB_CHECKER__` console API；Ctrl+Shift+T 开 Debug Window（「数据核对」按表+ID 对比 SQLite vs 页面缓存 / 「数据链路」查 trace DB channel/message 事件链）。**对迁移价值**: schemas/ 是「page 字段 ↔ 本地 SQLite 列」最干净的真源映射表。

---

## 8. 对迁移的关键提示（服务端字段契约必须满足的渲染依赖）

1. **readBits**: 字符串位图，长度=visibleMembers 数，按 index 对应；sender 位写入时必须预置 `'1'`（密聊 no-op 不 blanket -1）。
2. **unReadPostId**（大写 R wire 名）+ **unreadCount**: 配对下发，驱动「N 条未读跳转」。
3. **sendStatus**: 服务端 ack 后字段切默认态触发 message-status 文字渲染 + height 重算。
4. **props.files width/height**: 图片消息服务端必须回填尺寸（aspect-ratio 渲染）。
5. **expediteMap/expediteRecipients/expediteSender**: 加急 UI 依赖（status 0/1 区分已读）。
6. **repliedMessage / replyMessages**（含 viewers）: 回复引用块/预览/抽屉渲染 + 密聊可见性过滤。
7. **quickReply[]{emoji,userIds,users}**: emoji 回复气泡。
8. **viewers**: 密聊判定 + 首位查看者 + 回复可见性。
9. **lastEventSeq**（channel 级 event cursor）: 离线增量同步驱动 `POST_INCREMENT*`。
10. **type 值注意 `SYSTEM: 'SYSTEN'`** 拼写，迁移时不可「纠正」否则 message-system 分发失配。
