# G09 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 确认取消 |
| Helix 输入 | `ActionBinding(schedule_cancel)` |
| Command | `im_cancel_schedule` |
| QuickJS binding | `im.posts.cancelSchedule` |
| Surface | `message.channel-summary`、`message.left-dialog-list` |
| 完整 VM | ChannelSummaryVm + DialogListVm |
| 移动 UI 结果 | 取消成功后定时入口消失，重复取消幂等，失败时保留可重试状态 |
| 共享 PC/Helix 真源 | [G09](../../interaction-frame-v2-全量替换-按UC/G09/README.md) |
