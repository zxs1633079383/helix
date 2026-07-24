# G06 V2 切换 Tasks

- [ ] M037 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M038 tests-first 接通 im.posts.quickReply、`message.timeline`、`message.thread-detail` 和 C++ V2 runtime。
- [ ] M039 首轮纯 V2 证明 `POST /api/cses/posts/quickReply` body `{postId,emoji}`；`req_id` 只透传 `Cses-Track-Id`，actor 由服务端鉴权 session 注入、`post_update`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M040 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
