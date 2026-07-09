# Helix Dialog 指令化投影与 too_long 重拉 Spec（2026-07-09）

## 0. 范围

本 spec 只覆盖 IM 主链路里会影响左侧 Dialog 群/会话列表的两类行为：

- 在线发送/接收 `post + event_seq` 后，Helix 如何更新 storage 并发出可直接渲染的 Dialog patch。
- 离线 sync `events/too_long` 后，Helix 如何重放消息最终态，或丢弃单 channel 本地消息后重拉。

不进入本轮：`bot/agent`、`search`、`vote/average`。

当前基线：

| 仓库 | worktree | baseline |
|---|---|---|
| helix | `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709` | `8e17a0d700ddc8998d57b06e392a3616fbf29d0c` |
| cses-im-server | `/System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709` | `876464f1a02af82a84e4627bc312c313ed4e6114` |
| mobile-quick | `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709` | `5a0d14e811900dff3f2ed7adb1398368ffe20e16` |
| cses-client | `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709` | `0d2ce5fe37e228ac37dbe3743fb534e6d54aa62a` |

## 1. 设计原则

1. 前端只渲染，不计算业务状态。
2. `post + event_seq` 是普通发送/接收的主真源；不能等普通发送额外推 `update_channel` 才刷新 lastPost。
3. viewers 是消息可见性。后端 WS 会按 viewers 收窄投递；Helix 仍必须在本地 apply/sync 时复核 viewers，避免 phantom 被落成可见消息。
4. Dialog patch 必须是 Helix 业务投影，不是前端回读 DB 后再组装。
5. `too_long` 不是“cursor 往前挪一下”。该 channel 的本地消息增量视图已经不可信，必须丢弃本地消息并触发首屏/全量覆盖。

## 2. 在线 post 链路

后端发送主链路：

```text
POST /api/cses/posts/create
  -> app.SendPosts
  -> SavePosts 写 post/channel/channel_member
  -> append channel_event(type=1)
  -> sender self-read: channel_member.last_read_seq/msg_count_all
  -> WS post + event_seq
```

Helix 收到 `post + event_seq` 后的顺序：

```text
parse post
  -> 校验 channelId/event_seq
  -> 计算 visibility/sender/mention/urgent/unread 决策
  -> gate 按 event_seq 放行
  -> 写 message
  -> 写 channel last_post/unread/mention/urgent/cursor
  -> 读取 channel 单行形成 render-ready DialogPatch
  -> emit im:post:received
  -> emit im:channel:update-by-post
```

现有代码差距：

| 项 | 当前事实 | spec 要求 |
|---|---|---|
| message 入库 | `event_to_storage_op(PostUpsert)` 已有 | 保持 |
| channel lastPost/unread | `GuardedBump` 已写 `last_post/unread_count/unread_post_id/last_root_post_at` | 扩展为 mention/urgent set 语义，避免前端算 |
| cursor | `channel_event_cursor.last_event_seq` 已写 | 保持单调推进 |
| Dialog 投影 | `im:channel:update-by-post` 当前只有 `channel_id/event_seq/msg_id` | 升级为 render-ready Dialog patch，保留 legacy 三键 |
| 最终 unreadCount | 当前 emit 时不读 channel 行，拿不到最终 count | 写后用 `Scan(channel WHERE id=?)` 或新增 returning op，由 Helix 发最终 count |

## 3. `im:channel:update-by-post` 新契约

事件名保持不变，兼容现有监听；payload 从 thin signal 升级为“左侧 Dialog 行 patch 指令”。

```json
{
  "event": "im:channel:update-by-post",
  "data": {
    "op": "dialog.patchByPost",
    "source": "online_post",
    "channel_id": "c1",
    "channelId": "c1",
    "event_seq": 101,
    "eventSeq": 101,
    "msg_id": "p1",
    "msgId": "p1",
    "messageClass": "mention_urgent",
    "visible": true,
    "senderUserId": "u2",
    "lastPost": {
      "id": "p1",
      "temporaryId": "tmp1",
      "channelId": "c1",
      "userId": "u2",
      "type": "TEXT",
      "message": "hello",
      "simpleMessage": "hello",
      "props": {},
      "viewers": ["all"],
      "mentions": ["u1"],
      "createAt": 1783610000000,
      "eventSeq": 101
    },
    "dialogPatch": {
      "id": "c1",
      "channelId": "c1",
      "lastPost": {},
      "lastMessage": "hello",
      "unread": 3,
      "unreadCount": 3,
      "mention": true,
      "mentionCount": 1,
      "urgent": true,
      "urgentCount": 1,
      "unreadPostId": "p1",
      "lastPostAt": 1783610000000,
      "lastRootPostAt": 1783610000000,
      "lastEventSeq": 101
    }
  }
}
```

字段规则：

| 字段 | 规则 |
|---|---|
| `channel_id/event_seq/msg_id` | legacy anchor，必须保留 |
| `channelId/eventSeq/msgId` | render/UI 友好字段，和 legacy anchor 等值 |
| `messageClass` | `normal` / `mention` / `urgent` / `mention_urgent` |
| `visible` | viewers 本地复核结果；false 时不应发此事件 |
| `senderUserId` | `userSnapshot.userId` 优先，回退 `userId` |
| `lastPost` | Post 对象，不是 `channel.last_post` 的字符串形式 |
| `dialogPatch.unreadCount` | Helix storage 最终值，不允许前端 `+1` |
| `dialogPatch.mentionCount` | Helix storage 最终值，不允许前端从 mentions 自增 |
| `dialogPatch.urgentCount` | Helix storage 最终值，不允许前端从 props 自增 |
| `dialogPatch.readLastSeq/msgCount` | 当前不输出；等 `NEED_HELIX_CHANNEL_MEMBER_FIX.md` 完成 `channel_member` schema/write path 后再恢复，消费者不得默认 `0` |

## 4. MessageClass 与 count 计算

Helix 需要把这几条业务规则从前端移到 Rust：

| 规则 | 说明 |
|---|---|
| visible | `type == NOTICE` 恒可见；否则 viewers 为空/含 `all`/含当前 user 为可见；sync 的 messages map 缺消息体则视为 phantom |
| self echo | `senderUserId == auth_user_id` 时 unread 不增加，但 lastPost 仍更新 |
| mentionHit | `mentions` 含当前 user；如后端只给 channel_member 绝对 patch，则以 `update_channel` 绝对值覆盖 |
| urgentHit | `expediteMap`、`props.expedite`、`props.urgent` 命中当前 user 或非空 all-target 语义 |
| mention/urgent count | 本地用 `mention_list`、`urgent_post_list` set 语义防重复；count 为 set size；后端 `update_channel` 绝对字段到达时覆盖 |

因此 Helix 需要补字段：

| 结构 | 需要新增/保留字段 |
|---|---|
| `PostFields` | `simple_message`, `user_snapshot`, `mentions`, `expedite_map` 或可解析 urgent 的 props 视图 |
| `PostChannelUpdate` | `mention_hit`, `urgent_hit`, `mention_post_id`, `urgent_post_id`, `dialog_class` |
| `channel` storage | 继续使用 `mention_count`, `mention_list`, `urgent_count`, `urgent_post_list`, `last_post` |
| `CorrelationContext` | 新增单 channel DialogPatch 查询上下文，例如 `DialogPatchByPost { channel_id, event_seq, msg_id, source }` |

## 5. 离线 sync 链路

Go `/api/cses/channel/sync/notify` 返回：

```text
no_change | events | too_long | snapshot(保留)
```

Helix 对 `events` 的规则：

| eventType | Helix 行为 |
|---|---|
| `1 new` + messages map 有消息 | 写 message，写 channel lastPost/unread/mention/urgent，emit `im:post:received`，emit `im:channel:update-by-post{source:"sync_events"}` |
| `1 new` + messages map 无消息 | phantom，只推进 cursor，不落 message，不更新 lastPost，不发 Dialog patch |
| `2 edit` | patch message；如果被编辑消息是当前 lastPost，需要补 Dialog patch 或触发单 channel dialog reload |
| `3 delete` | revoke/delete message；如果删的是当前 lastPost，需要补 Dialog patch 或触发单 channel dialog reload |
| `6 read` | `channels/view` 读水位事件；需要保留 `actorId`，只有 actor 是当前 user 才清自己的 unread/read-state |

## 6. `too_long` 新契约

当前只 emit `im:sync:too_long` 并清内存 buffer/cursor，不足以保证 UI 和本地 storage 自洽。

新的规则：

```text
sync too_long(channelId, resetTo)
  -> 清 gate buffer
  -> 删除本地 message WHERE channel_id = channelId
  -> 清 channel 的 last_post/unread_post_id/has_more 等消息派生字段，或标记 loading
  -> cursor 暂置 resetTo-1
  -> emit im:sync:too_long { mode:"drop_and_reload" }
  -> 由 Helix 主动发起 getLatestPost 或 channel/load/incrementByChannelId 首屏覆盖
  -> 首屏结果重新写 message/channel
  -> emit im:messages:query_result / im:channels:projection 或 Dialog patch
```

`im:sync:too_long` payload 扩展：

```json
{
  "event": "im:sync:too_long",
  "data": {
    "channelId": "c1",
    "resetTo": 9001,
    "mode": "drop_and_reload",
    "dropLocalMessages": true
  }
}
```

前端收到该事件只能展示 loading/等待 Helix 后续投影，不能自己发 HTTP、不能自己拼首屏。

## 7. 对接仓库任务边界

| 仓库 | 本轮 spec 后续任务 |
|---|---|
| helix | 实现 DialogPatch 投影、mention/urgent set 语义、type6 actorId/read-state、too_long drop+reload |
| cses-im-server | 只校验 viewers WS 收窄、sync messages map 可见性、`post` payload 是否带 mentions/urgent 所需字段；无证据前不改业务 |
| mobile-quick | 重新 pin Helix 后验证 FFI event schema，移动端只渲染 DialogPatch |
| cses-client | 重新 pin Helix 后 message-v4 只消费投影，不计算 unread/mention/urgent/lastPost |

## 8. 验收

最小验收用例：

1. A 发普通消息，B 收到 `im:post:received` 和 `im:channel:update-by-post(messageClass=normal)`，Dialog lastMessage/unreadCount 更新。
2. A 发 @B，B 收到 `messageClass=mention`，mention/mentionCount 已由 Rust 给出。
3. A 发/设置加急给 B，B 收到 `messageClass=urgent` 或后续 `update_channel` 绝对 patch，urgent/urgentCount 由 Rust 给出。
4. A 自己发送 echo，A 的 lastPost 更新但 unreadCount 不增加。
5. 离线 B 上线 sync，visible message 产生同样 DialogPatch；phantom 只推进 cursor，不渲染。
6. sync too_long 后，本地该 channel 旧 messages 被丢弃，Helix 触发首屏覆盖，前端不自行 HTTP。
