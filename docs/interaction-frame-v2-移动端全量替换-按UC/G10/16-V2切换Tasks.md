# G10 V2 切换 Tasks

- [ ] M053 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M054 tests-first 接通 im.posts.queryMessagesByChannel / im.posts.loadOlderContext、`message.timeline` 和 C++ V2 runtime。
- [ ] M055 首轮纯 V2 证明 条件回源：`POST /api/cses/posts/getLatestPost` body `{channelId,timestamp:0}`；仅 Complete 分支不出网、无、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M056 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
