# G16b V2 切换 Tasks

- [ ] M093 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M094 tests-first 接通 im.channels.close / im.channels.leave / im.channels.changeInfo、`message.channel-summary`、`message.left-dialog-list`、`message.composer`、`message.member-roster` 和 C++ V2 runtime。
- [ ] M095 首轮纯 V2 证明 `POST /api/cses/channel/member/leave` body `{channelId,userId?,newCreator?}`、`channel_member_update`, `quit_company`（公司域也可能推）、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M096 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
