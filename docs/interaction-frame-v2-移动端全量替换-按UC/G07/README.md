# G07 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 发起加急或点击确认 |
| Helix 输入 | `ActionBinding(urgent/urgent_confirm)` |
| Command | `im_urgent_post` / `im_urgent_confirm` |
| QuickJS binding | `im.posts.urgentPost`、`im.posts.urgentConfirm` |
| Surface | `message.timeline`、`message.participant-receipts`、`message.unread-summary` |
| 完整 VM | TimelineWindowVm + ParticipantReceiptsVm + UnreadSummaryVm |
| 移动 UI 结果 | 加急标识、确认人数、目标列表与按钮权限一致，actor/observer 视图按权限分离 |
| 共享 PC/Helix 真源 | [G07](../../interaction-frame-v2-全量替换-按UC/G07/README.md) |
