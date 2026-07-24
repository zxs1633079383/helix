# G02 V2 切换 Tasks

- [ ] M021 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M022 tests-first 接通 im.send、`message.composer`、`message.timeline` 和 C++ V2 runtime。
- [ ] M023 首轮纯 V2 证明 `POST /api/cses/posts/create`，同上，`type=DOCUMENT`；占位字段建议 `message/simpleMessage="【文档】 http://doc.cses7.com/doc/abc123"`，`props.document={version:1,url:"http://doc.cses7.com/doc/abc123",title:"打开文档",placeholder:true}`；后续替换为 message-v3 的 `props.document={snapshot,version:1}`、`post`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M024 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
