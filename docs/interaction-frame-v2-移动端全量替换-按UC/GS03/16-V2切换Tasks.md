# GS03 V2 切换 Tasks

- [ ] M009 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M010 tests-first 接通 im.sync.channels、`message.left-dialog-list`、`message.unread-summary`、`message.timeline` 和 C++ V2 runtime。
- [ ] M011 首轮纯 V2 证明 `POST /api/cses/channel/sync/notify` body `{cursors:[{channelId,fromSeq}]}`、`pong.data` 固定包含 `text=pong`，对账时可含 `gaps:[{channelId,fromSeq,maxSeq}]` 与 `hashMismatch`；普通 pong 可只有 `text`。随后触发的 HTTP `/channel/sync/notify` 本身不产生业务 WS；并发 `post` / `posts_update` / `increment_channel_end` 不得替代补偿 HTTP、本地持久化或最终投影、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M012 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
