# G04 V2 切换 Tasks

- [ ] M025 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M026 tests-first 接通 im.posts.revoke、`message.timeline`、`message.thread-detail`、`message.left-dialog-list` 和 C++ V2 runtime。
- [ ] M027 首轮纯 V2 证明 `POST /api/cses/posts/revoke` body `{postId}`、`posts_update`, `post_update`, `update_channel`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M028 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
