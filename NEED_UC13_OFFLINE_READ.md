# NEED UC13 Offline Read Contract

## 状态

`RESOLVED_CODE_DONE`，最终真实链验收待统一 gate。Helix 已能在 sync type6 同时携带
`msgId`、`actorId`，且 `messages[msgId].readBits` 存在时，确定性重放
`im:post:read`。Go 已在 `d6fe376` 为 `POST /api/cses/post/read` 的每条实际 0->1 变化
同事务写入这种可同步事实，幂等重试不追加重复事件。

## 当前代码事实

- Go `internal/store/sqlstore/cses_post_read_store.go` 在同一事务持久化 `post.read_bits`、
  badge/mention/urgent 变化与带 `msgId`/`actorId` 的 post 级 type6 `channel_event`。
- `/channel/sync/notify` 按 `channel_event` cursor 返回该事实，并把该 post 放进 `messages`；
  Helix 因此可以同时取得读者身份与提交后的权威 `readBits`。
- `channels/view` 现有 type6 `channel_event` 只有 `actorId`，`msgId` 为空，只能证明“某人读过
  channel 水位”，不能证明 UC13 指定的哪条 post 已读。
- Helix 不得从 readBits 下标猜 reader；下标解释还依赖消息发送时成员快照，且当前 sync 没有
  提供这次变化对应的唯一 reader。

## 已兑现的最小跨仓合同

Go 在每条实际发生 `read_bits` 0→1 的 post 上，持久化一条可由 `/channel/sync/notify` 拉取的
type6 事实：

```json
{
  "channelId": "<channel>",
  "eventSeq": 42,
  "eventType": 6,
  "msgId": "<post>",
  "actorId": "<reader>",
  "createAt": 1700000123
}
```

同一 sync entry 的 `messages[postId].readBits` 必须是该 mutation 提交后的权威值。事件追加、
readBits 写入和 cursor 可见性必须满足原子提交或等价的 persist barrier，不能出现 cursor 已推进但
readBits 仍旧的窗口。幂等重试若没有新的 0→1 变化，不追加重复事件。

`channels/view` 仍保留既有 channel 级 type6 水位事件；逐 post sender 回执由其内部调用
`ReadPosts` 时产生的 post 级事实承担。仅有 channel 级 `actorId` 不生成逐消息回执。

## Helix 消费规则

- 三项齐全：保留既有 `im:channel:read_echo`，并追加
  `im:post:read{postId,msgId,readerId,readBits,receiptRevision}`。
- `msgId`、`actorId` 或权威 `readBits` 任一缺失：不生成 `im:post:read`。
- Helix 不解析 bitmap 反推 reader，不从当前成员列表或登录用户猜测 reader。
