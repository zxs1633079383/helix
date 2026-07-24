# G15 V2 切换 Tasks

- [ ] M081 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M082 tests-first 接通 im.channels.makeTopic / im.channels.create、`message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` 和 C++ V2 runtime。
- [ ] M083 首轮纯 V2 证明 `POST /api/cses/posts/makeTopic` body `{rootId,userIds}`；`req_id` 只用于 Helix/客户端关联，不能进入 HTTP body；未提供自定义图片时服务端从归一化成员派生 `pictureType=USER/picture.userIds`，客户端不得提交空 picture 占位、`channel_created`, `post_update`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M084 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
