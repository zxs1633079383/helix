# G16c V2 切换 Tasks

- [ ] M097 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M098 tests-first 接通 im.channels.close / im.channels.leave / im.channels.changeInfo、`message.channel-summary`、`message.left-dialog-list`、`message.composer`、`message.member-roster` 和 C++ V2 runtime。
- [ ] M099 首轮纯 V2 证明 `POST /api/cses/channel/change/info` body `{channelId,name?,displayName?,header?,purpose?,orient?,module?,pictureType?,picture?,source?,expectedVersion?}`；其他专用命令见 UC-5.5、`update_channel`, `update_channel_notice`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M100 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
