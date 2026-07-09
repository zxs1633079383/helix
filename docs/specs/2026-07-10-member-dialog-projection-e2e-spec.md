# Member 维度 Dialog 投影与真 Go E2E Spec（2026-07-10）

## 0. 结论

本 spec 修正 `2026-07-09-dialog-projection-and-sync-reset-spec.md` 里对 mention / urgent 计数的简化描述：

1. `mentionCount`、`urgentCount`、`unreadCount` 的业务事实源是 **channel_member 的当前用户行**，不是纯 channel 全局行。
2. `post + event_seq` 仍是普通消息 lastPost / message 入库 / cursor 推进的主链路，不能等普通消息额外 `update_channel` 才显示最后一条。
3. `update_channel` 是后端对 **member 维度绝对 badge / 提及 / 加急 / 置顶 / 清未读** 的权威覆盖事件。Helix 可以先从 `post` 帧派生快速 DialogPatch，但遇到 `update_channel` 必须按后端绝对值覆盖。
4. Helix 本地 schema 已补齐当前 Dialog 需要的 `channel_member` member 事实列，并通过
   `update_channel -> channel_member upsert -> dialog.patchByMember` 闭环。

不进入本 spec：bot/agent、search、vote/average。

## 1. 后端事实源

Go 后端 `channel_member` 表包含 member 维度计数字段：

- `msg_count` / `msg_count_root` / `msg_count_private`
- `mention_list` / `mention_count` / `mention_count_root`
- `urgent_post_list` / `urgent_mention_count`
- `unread_count`
- `last_post` / `last_post_at` / `last_root_post_at`

代码证据：

- `cses-im-server/internal/store/sqlstore/ent/migrate/schema.go:444` 起的 `ChannelMemberColumns`。
- `cses-im-server/internal/entity/channel/channel_member_count.go:12` 的 `ChannelMemberCount`。
- `cses-im-server/internal/store/sqlstore/cses_post_store.go:223` 的 `UpdateChannelMemberCounter`。
- `cses-im-server/internal/store/sqlstore/cses_channel_urgent_store.go:17` 的 `SetPostUrgent`。

发送时的 member 维度更新：

```text
SendPosts
  -> buildCounts
  -> GeneratorChannelMemberCount
  -> SavePosts
  -> UpdateChannelMemberCounter(channel_member)
```

mention 后端广播：

```text
sendMentionFrames
  -> mentionTargets
  -> LoadChannelMemberInfoByChannelIds
  -> ConvertIncrementChannelsFromMembers
  -> fillSeqAndUnread
  -> WS update_channel 单播给被 @ 的 member
```

urgent 后端广播：

```text
SetUrgentPost / ConfirmPostUrgent / CancelPostUrgent
  -> channel_member.urgent_post_list / urgent_mention_count
  -> sendUrgentPostAndIncrement
  -> WS update_channel 单播目标 member
  -> WS post_update 刷新消息 item
```

## 2. Helix 当前事实

Helix 当前 `channel` 表保存了 Dialog 快照列：

- `unread_count`
- `mention_count`
- `mention_list`
- `urgent_count`
- `urgent_post_list`
- `last_post`

Helix `channel_member` 表当前保留成员关系字段，并已补齐 Dialog 需要的 member 事实字段：

- `channel_id`
- `user_id`
- `team_id`
- `user_name`
- `role`
- `nick_name`
- `joined_at`
- `updated_at`
- `unread_count`
- `unread_post_id`
- `last_post`
- `last_post_at`
- `last_root_post_at`
- `mention_count`
- `mention_count_root`
- `mention_list`
- `mention_user`
- `urgent_mention_count`
- `urgent_post_list`
- `urgent_mention_user`
- `msg_count`
- `msg_count_root`
- `msg_count_private`
- `last_read_seq`

代码证据：`crates/helix-im/src/schema.rs` 的 `channel_member` 建表语句与
`IM_SCHEMA_MIGRATIONS`。

这意味着：`im:channel:update-by-post.dialogPatch.mentionCount/urgentCount` 在
`projectionAuthority="helix-derived"` 时仍是本地快速投影；在
`projectionAuthority="server-member-absolute"` 时来自后端 `update_channel` 的当前用户
`channel_member` 绝对态，可覆盖本地快速投影。

## 3. 投影职责划分

### 3.1 在线普通 post

`post + event_seq` 必须由 Helix 立即处理：

```text
WS post
  -> parse event_seq / channelId
  -> viewers 本地复核
  -> gate 按 event_seq 入库 message / 推进 cursor
  -> 本地生成 lastPost / lastMessage / senderUserId
  -> emit im:post:received
  -> emit im:channel:update-by-post(source=online_post)
```

原因：后端普通消息主 WS 是 `post`，普通消息不应强依赖额外 `update_channel` 才刷新最后一条消息。否则 `post` 先到、`update_channel` 延迟或缺失时，左侧 Dialog 会卡旧 lastPost。

### 3.2 mention / urgent / read 绝对值

member 维度最终值必须依赖后端绝对事实，优先级如下：

| 来源 | 用途 | 优先级 |
|---|---|---|
| `update_channel` 单播 | `unreadCount` / `mentionCount` / `mentionList` / `urgentCount` / `urgentPostList` / `channelIsTop` / 清 badge | 最高，绝对覆盖 |
| `post + event_seq` | lastPost、lastMessage、senderUserId、messageClass 快速反馈 | 中，允许先显 |
| `/sync/notify` events + messages | 离线追赶、phantom cursor、可见消息补偿 | 中 |
| 本地 set/bump 推导 | 仅作低延迟 optimistic patch | 最低，必须可被后端覆盖 |

### 3.3 `im:channel:update-by-post` 字段语义修正

`im:channel:update-by-post` 继续存在，但必须标明字段来源：

```json
{
  "event": "im:channel:update-by-post",
  "data": {
    "source": "online_post | sync_events | too_long_reload",
    "projectionAuthority": "helix-derived",
    "memberScope": "current_user",
    "channelId": "c1",
    "eventSeq": 101,
    "messageClass": "normal | mention | urgent | mention_urgent",
    "senderUserId": "u2",
    "lastPost": {},
    "lastMessage": "hello",
    "dialogPatch": {
      "lastPost": {},
      "lastMessage": "hello",
      "unreadCount": 3,
      "mentionCount": 1,
      "urgentCount": 1,
      "mentionList": [],
      "urgentPostList": []
    }
  }
}
```

约束：

- `memberScope=current_user`：这些 badge 字段只表示当前登录用户视角。
- `projectionAuthority=helix-derived`：表示该 patch 来自 Helix 本地投影，不是后端 member 行权威快照。
- 后续收到 `update_channel` 时，必须按 `projectionAuthority=server-member-absolute` 覆盖本地 Dialog。

### 3.4 `update_channel` 消费语义

Helix `update_channel` handler 当前只做 channel PATCH + emit `im:channel:update` thin trigger。这个路径需要升级为：

```text
WS update_channel
  -> collect channel/member absolute fields
  -> 写本地 Dialog 快照列
  -> 如果 payload 是当前用户 member 视角：
       emit im:channel:update-by-post 或 im:channel:update-member-badge
       projectionAuthority="server-member-absolute"
  -> 前端只渲染，不再自行 +1/-1
```

注意：成员列表、owner、adminUsers、boss 仍不属于 `update_channel` patch 路径，继续走 `channel_member_update` / `channel_created` / `increment_channel`。

## 4. 离线 sync 语义

离线 sync 的 cursor 仍只锚 `channel_event.event_seq`。

`events` 分支：

- type=1 可见消息：Helix 可先按 `post` 同口径产生 `source=sync_events` 的 DialogPatch。
- type=1 phantom：只推进 cursor，不落 message，不更新 Dialog。
- type=6 read：只有 actor 是当前 user 时，才可清自己的 unread/read-state；否则只作为对端 read echo。

`too_long` 分支：

- 清本地 `message WHERE channel_id=?`。
- 清 channel 上消息派生 Dialog 字段。
- cursor reset 到 `resetTo-1`。
- emit `im:sync:too_long {mode:"drop_and_reload"}`。
- Helix 主动 `posts/getLatestPost` 重拉首屏。

## 5. E2E 验证口径

Helix 仓已有真 Go 联调入口：

- 快速全 UC：`scripts/host-cli-FULL-uc-e2e.sh`
- 通用三层探针：`scripts/dev-loop.sh`
- 手动持久会话：`scripts/host-cli-session.sh`
- 环境恢复卡：`docs/harness/HX-C016-e2e-env-recovery-and-fast-harness.md`

验证必须分三层记录：

| 层 | 证据 |
|---|---|
| 代码层 | Helix `Effect::Http`、`Effect::Persist`、`DomainEvent emit` |
| 日志层 | Go 日志中的 handler / publish / error delta |
| 效果层 | RAW WS inbound action、Helix emit payload、SQLite storage 结果 |

本轮 2026-07-10 尝试 / 实现结果：

- `scripts/lib-config.sh` 已补 Python fallback：本机缺 `yq` 时仍能读取 `helix.config.yaml`。
- `scripts/host-cli-FULL-uc-e2e.sh` 已默认给 sender/receiver 设置 `HELIX_DUMP_FRAMES=1`，且 matcher 同时支持 `RAW WS inbound` 与 `HOP2 ws frame received`。
- `create` / `makeTopic` harness body 已改为 Helix outbound 当前要求的 snake_case：`display_name/team_id/self_id/member_ids` 与 `root_id/post_id/...`。
- `channel_member` schema 已补 member 维度 dialog 字段，并加 `IM_SCHEMA_MIGRATIONS` 供 host/ffi 对旧库 additive 补列。
- `update_channel` handler 已识别 member badge payload，按 `(channel_id,user_id)` upsert `channel_member`，并 emit `im:channel:update-by-post`，`projectionAuthority="server-member-absolute"`。
- 真 Go `:8066` 已手动启动并通过 `/metrics` 探活。
- 快速全 UC live E2E：`WAIT_TO=6 HOLD_S=180 bash scripts/host-cli-FULL-uc-e2e.sh`。
- 结果：`PASS=30 FAIL=0 SKIP=7`。
- 日志：
  - TX: `/var/folders/3g/23s26mrd00g68hgr42h3wq_40000gn/T/helix_full_tx.XXXXXX.log.QT7ppgzOBB`
  - RX: `/var/folders/3g/23s26mrd00g68hgr42h3wq_40000gn/T/helix_full_rx.XXXXXX.log.KXfE1RNGmO`
- RX 日志证据：`DomainEvent emit event=im:channel:update-by-post` 计数 `2381`，
  `update_channel` WS 帧观测 `6` 次。
- focused storage 证据：receiver 本地 SQLite `channel_member` 出现
  `channel_id|678|unread_count=2|mention_count=0|urgent_mention_count=1|urgent_post_list=[postId]`。

所以当前判定是：

```text
代码级/单测级：member 维度 badge 绝对覆盖已接通。
live E2E：真 Go 8066 + host-cli 全 UC PASS=30 FAIL=0，剩余 SKIP 为 harness/能力边界。
```

## 6. 实现项状态

1. 已完成：扩展 Helix `channel_member` schema：补 `unread_count`、`last_post`、`last_post_at`、`last_root_post_at`、`mention_count`、`mention_count_root`、`mention_list`、`mention_user`、`urgent_mention_count`、`urgent_post_list`、`urgent_mention_user`、`msg_count`、`last_read_seq`。
2. 已完成：新增 member-scope storage op：按 `(channel_id,user_id)` upsert 当前用户 member 事实。
3. 已完成：改 `update_channel` handler：识别后端单播 member badge payload，写 member/cache 后 emit `projectionAuthority=server-member-absolute`。
4. 保持：`post` / sync type=1 的 Helix-derived DialogPatch 保留，用于低延迟 lastPost 与 optimistic badge；可被 `update_channel` 覆盖。
5. 已完成：修复 `scripts/host-cli-FULL-uc-e2e.sh`：
   - 无 `yq` 时使用 `helix.config.yaml` 的简单 Python fallback。
   - 默认设置 `HELIX_DUMP_FRAMES=1`，matcher 同时支持 `HOP2 ws frame received`。
   - `create` / `makeTopic` 命令体改为 Helix outbound 当前结构化 args。
6. 待环境恢复后重跑 E2E 绿线：
   - A 发普通消息：B 收 `post`，Helix emit `im:post:received` + `im:channel:update-by-post`。
   - A @B：B 收 `post` 后可先见 mention patch，随后 `update_channel` 绝对覆盖 `mentionCount`。
   - A 加急 B：B 收 `update_channel` + `post_update`，Helix 绝对覆盖 urgent badge。
   - 离线重登：`increment_channel.needSync` → `/sync/notify` → visible/phantom/too_long 三类均有日志与 storage 证据。
