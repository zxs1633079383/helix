# G01 V2 切换 Tasks

- [ ] M017 冻结 [V1 类图](./01-V1当前类图.mmd)、[流程](./02-V1当前执行流程.mmd) 与真实页面/制品证据。
- [ ] M018 tests-first 接通 im.send、`message.composer`、`message.timeline`、`message.left-dialog-list`、`message.unread-summary` 和 C++ V2 runtime。
- [ ] M019 首轮纯 V2 证明 `POST /api/cses/posts/create` body `{viewers,message,mentions,temporaryId,type,simpleMessage,channelId,userId,teamId,userSnapshot,id,props,topicId,revoke,createAt,replyId?,replyRootId?,replyFirstLevelId?}`；`replied={id,reply_id,reply_root_id,reply_first_level_id}` 只在 Helix 组装回复三锚，`req_id` 只透传 `Cses-Track-Id`，两者均不作为 HTTP body 字段、正常发送主推 `post`；`update_channel` 不是普通发送主链路单独推送，仅在读标记/频道变更/撤回最后一条/加急等副链出现；错误本地 `im:post:send-failed`、`PersistAtomic -> PersistOk` 与移动 UI 同一 correlation。
- [ ] M020 物理删除 producer+consumer V1 支持并在新 candidate 二次验收。

全局进度真源为 [12-全量切换Tasks.md](../12-全量切换Tasks.md)。
