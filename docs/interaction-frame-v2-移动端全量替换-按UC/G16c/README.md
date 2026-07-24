# G16c Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 关闭、退出或保存设置 |
| Helix 输入 | `ActionBinding(close/leave/settings)` |
| Command | `im_channel_close` / `im_channel_leave` / `im_channel_change_info` |
| QuickJS binding | `im.channels.close`、`im.channels.leave`、`im.channels.changeInfo` |
| Surface | `message.channel-summary`、`message.left-dialog-list`、`message.composer`、`message.member-roster` |
| 完整 VM | ChannelSummaryVm + DialogListVm + ComposerVm + MemberRosterVm |
| 移动 UI 结果 | 设置保存后频道头、会话行和成员相关权限按 expectedVersion 原子更新 |
| 共享 PC/Helix 真源 | [G16](../../interaction-frame-v2-全量替换-按UC/G16/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
