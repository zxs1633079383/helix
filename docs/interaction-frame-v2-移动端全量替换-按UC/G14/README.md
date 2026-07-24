# G14 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 单条可见、打开回执或点击已收到 |
| Helix 输入 | `ActionBinding(post_read/template_received)` / `HostObservation` |
| Command | `im_post_read` / `im_template_received` |
| QuickJS binding | `im.posts.markRead`、`im.posts.templateReceived` |
| Surface | `message.participant-receipts`、`message.timeline`、`message.thread-detail` |
| 完整 VM | ParticipantReceiptsVm + TimelineWindowVm + ThreadDetailVm |
| 移动 UI 结果 | 读者列表、当前消息读态和线程详情一致；HostObservation 不直接伪造已读成功 |
| 共享 PC/Helix 真源 | [G14](../../interaction-frame-v2-全量替换-按UC/G14/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
