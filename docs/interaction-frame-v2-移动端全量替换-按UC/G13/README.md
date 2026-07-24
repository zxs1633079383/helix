# G13 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 频道可见或点击已读 |
| Helix 输入 | `ActionBinding(channel_read)` / `HostObservation` |
| Command | `im_channels_view` |
| QuickJS binding | `im.posts.readChannel` |
| Surface | `message.left-dialog-list`、`message.unread-summary`、`message.timeline` |
| 完整 VM | DialogListVm + UnreadSummaryVm + TimelineWindowVm |
| 移动 UI 结果 | 会话未读数、频道已读边界和时间线读态同帧变化，重复可见观察幂等 |
| 共享 PC/Helix 真源 | [G13](../../interaction-frame-v2-全量替换-按UC/G13/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
