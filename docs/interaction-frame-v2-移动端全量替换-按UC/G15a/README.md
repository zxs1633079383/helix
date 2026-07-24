# G15a Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 提交话题或群聊创建表单 |
| Helix 输入 | `ActionBinding(make_topic/create_channel)` |
| Command | `im_make_topic` / `im_create_channel` |
| QuickJS binding | `im.channels.makeTopic`、`im.channels.create` |
| Surface | `message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster` |
| 完整 VM | TopicListVm + ChannelSummaryVm + DialogListVm + MemberRosterVm |
| 移动 UI 结果 | 新群聊在首批四个 Surface 原子出现，成员与 viewer 隔离由权威事实决定 |
| 共享 PC/Helix 真源 | [G15](../../interaction-frame-v2-全量替换-按UC/G15/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
