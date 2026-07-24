# G15a V2 切换 Tasks

- [ ] M085 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M086 tests-first 接通 im.channels.makeTopic / im.channels.create、`message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` 和 C++ V2 runtime。
- [ ] M087 首轮纯 V2 证明 `POST /api/cses/channel/create` body `{type,userIds,displayName,forceCreate}`；session 身份、team、成员角色与默认 `pictureType=USER/picture.userIds` 由服务端派生，客户端不得提交 team/role/owner/picture、`channel_created`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M088 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
