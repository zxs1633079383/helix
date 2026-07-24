# G07 V2 切换 Tasks

- [ ] M041 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M042 tests-first 接通 im.posts.urgentPost / im.posts.urgentConfirm、`message.timeline`、`message.participant-receipts`、`message.unread-summary` 和 C++ V2 runtime。
- [ ] M043 首轮纯 V2 证明 `POST /api/cses/posts/urgentPost` body `{channelId,postId,targetIds,message?}`；`POST /api/cses/posts/urgentConfirm` body `{postId,channelId}`、`post_update`, `update_channel`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M044 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
