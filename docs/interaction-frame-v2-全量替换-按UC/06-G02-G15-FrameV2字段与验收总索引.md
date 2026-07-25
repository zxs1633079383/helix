# G02-G15 InteractionFrameV2 字段与验收总索引

> 查证日期：2026-07-24 PDT。范围只包含 G02、G04、G05、G06、G07、G08、G10、
> G13、G14、G15 及其 a 子 Gate；G01 仅作为平台能力和文档形态参考。

## 1. 统一所有权

```text
UI intent / Mobile JS intent
-> canonical command 或 ActionInvocationEnvelope
-> Helix Tick::Command / Tick::Inbound / PortReply
-> Effect batch
-> matching PersistOk 或权威 readback
-> InteractionFrameV2
-> PC SurfaceRuntime / Mobile SurfaceRuntime 原子 apply
-> FrameApplied
-> typed DOM / im.surface.view
```

Helix 统一拥有权限、排序、计数、状态、typed VM 和 action。Go 拥有 HTTP、WS 与远端
持久化事实。PC/Mobile 只做协议转换、slot apply、ack 与展示，不得各自生长业务 reducer。

## 2. InteractionFrameV2 envelope

| 字段 | 类型与允许值 | 约束 |
|---|---|---|
| `schemaVersion` | `2` | 其他值 fail closed |
| `transactionId` | 非空 string | apply/ack/去重主键 |
| `causationId?` | 非空 string | 可选；command/request/authority correlation |
| `runtime.connection` | `connecting/online/offline` | `recovered` 时必须 online |
| `runtime.initialSync` | `idle/syncing/ready/failed` | renderer 不推导 |
| `runtime.recovery` | `idle/syncing/recovered/failed/blocked` | 与 surfaces 同帧原子发布 |
| `mutations` | 非空数组 | 同一帧禁止重复 canonical slot |
| `directives` | `[{directiveId,kind}]` | 两字段均非空；执行失败回 ack |

## 3. SurfaceMutationV2

| 字段 | 类型与允许值 | 约束 |
|---|---|---|
| `surface` | 下表固定枚举 | 必须与 mutation kind 匹配 |
| `scope` | surface 专属 tagged union | 所有身份字段非空 |
| `revision` | `1..=9007199254740991` | slot 内单调增加 |
| `baseRevision?` | 同上 | patch 必填且 `revision > baseRevision`；snapshot 禁止 |
| `surfaceEpoch` | `1..=9007199254740991` | 只能由 Helix 生产 |
| `causationId?` | 非空 string | mutation 级 authority correlation |
| `mutation` | typed tagged union | patch 为 `replaceView`，不暴露局部业务 reducer |

| Surface | Scope | mutation.kind | View |
|---|---|---|---|
| `message.timeline` | `Timeline{channelId,windowToken}` | `TimelineSnapshot/TimelinePatch` | `TimelineWindowVm` |
| `message.channel-summary` | `Channel{channelId}` | `ChannelSummarySnapshot/ChannelSummaryPatch` | `ChannelSummaryVm` |
| `message.thread-detail` | `Thread{channelId,rootMessageId}` | `ThreadDetailSnapshot/ThreadDetailPatch` | `ThreadDetailVm` |
| `message.member-roster` | `MemberRoster{channelId,cursor}` | `MemberRosterSnapshot/MemberRosterPatch` | `MemberRosterVm` |
| `message.participant-receipts` | `ParticipantReceipts{channelId,messageId,receiptKind}` | `ParticipantReceiptsSnapshot/ParticipantReceiptsPatch` | `ParticipantReceiptsVm` |

## 4. 关键 typed VM 字段

### TimelineWindowVm

`channelId, anchor{messageId?,mode=latest|locate|unread}, items[], firstUnreadMessageId?,
targetMessageId?, page{windowToken,hasOlder,hasNewer,hasMore},
state=loading|ready|empty|forbidden|failed`

每个 `MessageVm`：

`id, channelId, eventSeq, author{userId}, createdAt, content, delivery, actions,
quickActions, replyId?, replyRootId?, replyFirstLevelId?, replySummary, replyThread?,
reactions[{emoji,count}], readReceipt, templateConfirmation?, urgency?, pinned,
bookmarked, topicLink?`

`content.kind` 允许：

- `text{text}`
- `document{documentId,title,summary,access}`
- `media{mediaType,resource,thumbnail?,fileName,size,availability}`
- `forwarded{summary,items}`
- `systemEvent{eventKind,memberId?,text}`
- `recalled{text}`

`delivery.kind` 允许：
`uploading{progressPercent=0..100}/sending/sent/failed{reason,retryable}/recalled`。

### ChannelSummaryVm

`channelId,title{text},avatar{resource?,userIds,fallbackText},noticeSummary?,purpose?,
lifecycle{state,reason?},memberCount,viewerRole=owner|admin|member,onlineCount?,
topicCount,notificationMode=all|mentions|muted,actions,settingsVersion`

### ThreadDetailVm

`channelId,detailContext,root,orderedReplyIds,nodes,replyCount,page,state`。
`detailContext` 为 `reply`，或
`topic{parentChannelId,topicChannelId,rootMessageId,title}`。

### MemberRosterVm

`channelId,items,page,memberCount,onlineCount?,state`；每个成员为
`user{userId},nickname?,role,presence=online|away|offline,
membershipState=active|left|removed,actions`。

### ParticipantReceiptsVm

`channelId,messageId,kind=read|templateConfirmation|urgentConfirmation,summary,
completed[],pending[],page,state`；每行包含
`user{userId},memberRole,status=pending|completed,completedAt?`。

## 5. canonical inbound command

| Gate | command | required | optional | 禁未知字段 | Effect |
|---|---|---|---|---|---|
| `G-02` | `im_send_message` | `channel_id`、`type`、`props` | `text`、`replied`、`req_id` | 是 | `storage_upsert`、`http_write`、`timer`、`projection` |
| `G-04` | `im_revoke` | `post_id` | 无 | 是 | `http_write`、`storage_upsert`、`projection` |
| `G-04a` | `im_sync_channels` | `reason` | `channel_id` | 是 | `sync_request`、`storage_upsert`、`projection` |
| `G-05` | `im_relay_messages` | `post_ids`、`target_channel_ids`、`mode` | 无 | 是 | `storage_read`、`http_write`、`projection` |
| `G-06` | `im_quick_reply` | `post_id`、`emoji` | `req_id` | 是 | `http_write`、`storage_upsert`、`projection` |
| `G-07` | `im_urgent_post` | `post_id`、`target_ids` | 无 | 是 | `http_write`、`projection` |
| `G-07` | `im_urgent_confirm` | `post_id` | 无 | 是 | `http_write`、`projection` |
| `G-08` | `im_create_schedule` | `channel_id`、`message`、`schedule_post_at` | `type`、`props` | 是 | `http_write`、`storage_upsert`、`projection` |
| `G-10` | `im_query_messages_by_channel` | `channel_id` | `limit` | 是 | `local_scan`、`http_read`、`storage_upsert`、`projection` |
| `G-10a` | `im_load_older_context` | `channel_id`、`anchor_post_id` | `before` | 是 | `http_read`、`storage_upsert`、`projection` |
| `G-13` | `im_channels_view` | `channels` | 无 | 是 | `http_write`、`storage_upsert`、`projection` |
| `G-14` | `im_post_read` | `channel_id`、`posts` | 无 | 是 | `http_write`、`storage_upsert`、`projection` |
| `G-14a` | `im_template_received` | `post_id` | 无 | 是 | `http_write`、`projection` |
| `G-15` | `im_make_topic` | `root_id`、`user_ids`、`req_id` | 无 | 是 | `http_write`、`storage_upsert`、`projection` |
| `G-15a` | `im_create_channel` | `type`、`user_ids`、`display_name`、`force_create` | 无 | 是 | `http_write`、`storage_upsert`、`projection` |

上表是 Helix canonical snake_case command。Mobile/Angular 公共 facade 可以接收 camelCase
intent，但必须通过 Helix 共享 normalization 生成同一 canonical payload；不得在两端分别维护
字段别名和默认值。

## 6. ActionInvocationEnvelope

仅 ActionBinding 路径使用：

`schemaVersion=1, requestId, actorId, surface, scope, surfaceEpoch,
observedRevision, invocation{actionRef,input}`。

`schemaVersion=1` 是 action ingress 版本，不是 InteractionFrame V1。requestId/actorId/
surfaceEpoch/observedRevision/actionRef 均必须非空或正数；stale epoch/revision 必须拒绝。

## 7. 统一证据等级

| 等级 | 能证明 | 不能证明 |
|---|---|---|
| Helix unit/replay | Tick/Effect/Persist barrier/完整 Frame | Tauri、Go、SQLite executor、DOM |
| Fixture parity | 三仓 byte/hash/semantic contract 一致 | 真实 Rust/HTTP/WS |
| WDIO Mock | 真 Angular bus/runtime/DOM 消费完整 Frame | Tauri/Helix/Go 已执行 |
| real invoke | 真 `window.__TAURI__.core.invoke` 进入 Rust | Observer/WS/PG/双端完成 |
| single-real | Actor 的 Go/SQLite/Frame/DOM | Observer delivery |
| dual-real | Actor/Observer 同一 authority 闭环 | 未运行的失败/恢复分支 |
| Mobile GN GTest | QuickJS/C++/FFI/V2 apply/ack/view | 真机网络或 PC DOM |
| Venus real | Venus API 到真实 helix-driver-ffi 与 SurfaceRuntime | PC、Go 或双账号自动成立 |

## 8. 当前总 Handoff

- `CURRENT`：Helix V2 类型、selected Gate focused tests、PC 旧 desktop scenarios、
  Mobile UC command GTest/real-chain manifest、通用 Mobile/Venus V2 consumer。
- `HISTORICAL`：2026-07-22 的旧 Native Gate artifacts；只能帮助定位场景，不能证明当前
  pin、WDIO 平台或 Mobile V2 consumer。
- `TARGET`：每个 GN 的 Helix canonical Frame Fixture、provenance/hash、WDIO
  mock/integration/single/dual、GN GTest、Venus Frame 终态断言和统一 Report。
- `BLOCKED`：在未重新运行当前 pin 前，所有 GN 的 PC-Mobile canonical Green 均保持
  blocked/unknown。
- `FORBIDDEN`：Fixture、command receipt、HTTP 200、旧 artifacts 或单端 DOM 互相替代。
