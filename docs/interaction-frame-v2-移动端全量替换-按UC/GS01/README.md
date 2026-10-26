# GS01 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 连接建立与 `hello` |
| Helix 输入 | `HostObservation` |
| Command | `im_query_dialog_list` |
| QuickJS binding | `im.sync.dialogList` |
| Surface | `message.left-dialog-list`、`message.unread-summary` |
| 完整 VM | DialogListVm + UnreadSummaryVm |
| 移动 UI 结果 | 连接完成后，会话列表与未读摘要一次出现；连接受理不能显示为空列表成功 |
| 共享 PC/Helix 真源 | [GS01](../../interaction-frame-v2-全量替换-按UC/GS01/README.md) |
