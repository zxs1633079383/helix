# G08 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 选择时间并提交 |
| Helix 输入 | `ActionBinding(schedule_create)` |
| Command | `im_create_schedule` |
| QuickJS binding | `im.posts.createSchedule` |
| Surface | `message.channel-summary`、`message.left-dialog-list`、`message.timeline` |
| 完整 VM | ChannelSummaryVm + DialogListVm + 可选 TimelineWindowVm |
| 移动 UI 结果 | 定时消息提交中、已创建和失败状态可恢复，observer 不显示 owner 草稿 |
| 共享 PC/Helix 真源 | [G08](../../interaction-frame-v2-全量替换-按UC/G08/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
