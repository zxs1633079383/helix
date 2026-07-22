# G04-G16c 完整时序和数据索引

> 本目录把 G04-G16c 的目标 InteractionFrame v2 合同，与当前 Go server HTTP/WS 数据逐项对齐。
> 它细化上级 `05`-`08` 分册，不替代 Gate 状态台账，也不把目标合同声明为 runtime green。

## 1. 源码快照与事实等级

本轮核对基于：

- Helix：`ca79d82460fd1b681eb8a28cfd6c51cd03086e05`，并包含当前工作树中尚未提交的
  `interaction_frame.rs` / SurfaceRuntime 改动；
- Go server：`0ca4810dd627da61336433d4b2f8e1a2c6eea98d`，HTTP handler、DTO、application
  publish 路径按当前工作树读取；
- Frame 真源：`crates/helix-im/src/interaction_frame.rs`；
- Go HTTP 真源：`internal/api/post/`、`internal/api/channel/` 与 `internal/entity/`；
- Go WS 真源：`internal/app/cses_post_*`、`internal/app/cses_channel_*` 与 `internal/ws`。

本文使用三种标签：

| 标签 | 含义 |
|---|---|
| `CURRENT` | 当前源码已经存在的 endpoint、DTO、WS event 或 Frame 类型。 |
| `TARGET` | V2 应实现的 Tick/Effect/Persist/Frame 因果链。 |
| `GAP` | Go、Helix action dispatch、projector 或 wire 之间仍缺可执行映射。 |

关键现状：当前 `projector/action.rs::ActionDispatch` 的 live 映射只有 `ViewReceipt`。
G04-G16c 其余 ActionBinding 仍是 `TARGET`，不能因为 outbound builder 或 VM 类型已经存在就标成已上线。

## 2. 文档索引

| 范围 | 文档 |
|---|---|
| G04-G07 | [01-G04-G07-撤回转发回应与加急完整时序和数据.md](./01-G04-G07-撤回转发回应与加急完整时序和数据.md) |
| G08-G12 | [02-G08-G12-定时窗口定位与回复完整时序和数据.md](./02-G08-G12-定时窗口定位与回复完整时序和数据.md) |
| G13-G14a | [03-G13-G14a-未读回执与模板完整时序和数据.md](./03-G13-G14a-未读回执与模板完整时序和数据.md) |
| G15-G16c | [04-G15-G16c-建群话题与频道治理完整时序和数据.md](./04-G15-G16c-建群话题与频道治理完整时序和数据.md) |

## 3. Go HTTP 公共响应合同

Go 的 `dto.CommonRes` 是所有相关接口的外层信封，HTTP status 恒为 200：

```json
{
  "status": "SUCCESS | failed | FINISH",
  "message": "optional string",
  "data": "optional endpoint-specific value"
}
```

因此：

1. `HTTP 200` 只表示拿到了 Go 信封；
2. `status == "SUCCESS"` 才表示该 handler/application 调用成功；
3. mutation 接口的 `SUCCESS` 仍不能替代权威 WS/sync 和 Helix `PersistOk`；
4. `data` 缺失与 `data: null` 都不得被解释成业务实体已创建或已变更。

## 4. InteractionFrame v2 公共下行合同

当前 Rust 类型的准确外形是：

```jsonc
{
  "schemaVersion": 2,
  "transactionId": "persist:<correlation>",
  "causationId": "<requestId-or-trackId>",
  "runtime": {
    "connection": "online | connecting | offline",
    "initialSync": "idle | syncing | ready | failed",
    "recovery": "idle | syncing | recovered | failed | blocked"
  },
  "mutations": [
    {
      "surface": "message.timeline",
      "scope": {
        "kind": "timeline",
        "channelId": "C",
        "windowToken": "W"
      },
      "revision": 8,
      "baseRevision": 7,
      "causationId": "<requestId-or-trackId>",
      "mutation": {
        "kind": "timelinePatch",
        "patch": {
          "kind": "replaceView",
          "view": "<complete TimelineWindowVm>"
        }
      },
      "surfaceEpoch": 3
    }
  ],
  "directives": [
    { "directiveId": "directive-1", "kind": "<opaque-kind>" }
  ]
}
```

`patch` 也是完整 `replaceView`，不是 JSON Patch。每个 mutation 必须带完整 VM、单调 revision、
匹配的 baseRevision 和当前 surfaceEpoch。同一 Frame 不得重复同一个 `(surface, canonical scope)`。

当前 `UiDirective` 只有 `directiveId` 与 `kind`，没有 payload。G11 的 `targetMessageId` 必须先放在
`TimelineWindowVm.targetMessageId`；若要让 directive 自带 target，必须先扩展并版本化 Rust wire，
不能在文档中虚构现有字段。

## 5. 九个 Surface 的完整 VM 数据

| Surface | scope | 完整 view 的必填数据 |
|---|---|---|
| `message.left-dialog-list` | `User{userId,teamId}` | `sections[] -> {kind,items[],page}`、`state`；每行含 channel/title/avatar/preview/activity/unread/mention/urgent/topic/schedule/top/notify/lifecycle/actions。 |
| `message.unread-summary` | `User` | `unreadTotal`、`mentionTotal`、`urgentTotal`、`unreadChannelCount`。 |
| `message.timeline` | `Timeline{channelId,windowToken}` | `channelId`、`anchor`、完整 `items[]`、`firstUnreadMessageId?`、`targetMessageId?`、`page`、`state`。 |
| `message.thread-detail` | `Thread{channelId,rootMessageId}` | `channelId`、完整 `root`、`orderedReplyIds[]`、`nodes{}`、`replyCount`、`page`、`state`。 |
| `message.channel-summary` | `Channel{channelId}` | title/avatar/notice/purpose/lifecycle/memberCount/onlineCount/topicCount/notificationMode/actions/settingsVersion。 |
| `message.composer` | `Channel` | `channelId`、`state`、`draftPolicy`、`actions[]`、`capabilityEpoch`。 |
| `message.member-roster` | `MemberRoster{channelId,cursor}` | `items[]`、page/memberCount/onlineCount/state；每行含 user/nickname/role/presence/membershipState/actions。 |
| `message.topic-list` | `TopicList{parentChannelId,cursor}` | `items[]`、page/topicCount/state；每行含 topic/root/title/preview/activity/messageCount/avatar/lifecycle/actions。 |
| `message.participant-receipts` | `ParticipantReceipts{channelId,messageId,receiptKind}` | `summary`、`completed[]`、`pending[]`、page/state；成员行含 user/role/status/completedAt。 |

`MessageVm` 的完整必填字段为 `id/channelId/author/createdAt/content/delivery/actions/replySummary/readReceipt/pinned/bookmarked`，
并可带 `templateConfirmation/urgency/topicLink`。任何 Gate 更新消息行时，都要重新给出该完整 row，
不得只下发 reaction count、recalled 标记或 receipt bitmap。

## 6. 共同时序与完成判定

```text
Frame 中的 ActionBinding / HostObservation
  -> ActionInvocationEnvelope / normalized intent
  -> Tick::Command
  -> 校验 actor + scope + epoch + revision + actionRef + input
  -> Effect::Http(Go endpoint)
  -> Go HTTP CommonRes(status=SUCCESS)              // 请求层成功，非 UI 终态
  -> authoritative Go WS / sync / read response
  -> Helix normalize domain facts
  -> Effect::PersistAtomic(facts + cursor + identity/index)
  -> Tick::PortReply(PersistOk)
  -> SurfaceComposer(only attached affected slots)
  -> InteractionFrame v2(causationId matches request)
  -> PC/Mobile validate whole frame, atomically replace cache, then run directive
```

查询类 G10-G12、G14 receipt detail 可以由权威 HTTP data 直接进入 `PersistAtomic`；mutation 类不能。
重复/乱序 WS 必须按 event identity、entity revision、eventSeq/cursor 幂等。复杂度目标是按 id/index
做 `O(1)` 或 `O(log n)` 关联；有界窗口/分页 compose 为 `O(visible)`，禁止跨全库或模板内 `O(n^2)`。

## 7. 明确缺口

- G04-G16c 大部分 action 还没有 live `ActionDispatch`，文档只定义目标映射。
- 多数 VM 的 `SurfaceDataVm` 仍是有界通用值，reaction、urgency、reply summary 等尚未全部收敛为独立强类型结构。
- G14a 的 Go `/post/templateReceived` 表示“收到”，不是独立“业务确认”；在 Go 增加明确语义前，
  不得把它投影为 `ReceiptKind::TemplateConfirmation` 的 completed。
- `UiDirective` 当前无 payload；定位参数必须来自 Frame VM，不能从 raw WS 或端侧缓存旁路获得。
- 真实完成仍需要 Go HTTP/WS/DB、Helix Persist/Frame、PC/Mobile apply 与双用户证据；本文不提供这些 runtime 证据。
