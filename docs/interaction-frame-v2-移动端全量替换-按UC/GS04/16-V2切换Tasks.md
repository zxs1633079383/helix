# GS04 V2 切换 Tasks

- [ ] M013 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M014 tests-first 接通 im.channels.ensureLoaded、`message.channel-summary`、`message.composer`、`message.timeline` 和 C++ V2 runtime。
- [ ] M015 首轮纯 V2 证明 `POST /api/cses/channel/load/incrementByChannelId` body `{channelId}`、无直接 WS；后续可能收到 `increment_channel`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M016 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
