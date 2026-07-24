# GS02 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | `sync notify` 入站 |
| Helix 输入 | `Inbound` |
| Command | `im_sync_channels` |
| QuickJS binding | `im.sync.channels` |
| Surface | `message.left-dialog-list`、`message.unread-summary`、`message.timeline` |
| 完整 VM | 恢复运行态与受影响完整 VM |
| 移动 UI 结果 | 恢复期间保留最后一致视图，提交后会话、未读和受影响时间线同帧更新 |
| 共享 PC/Helix 真源 | [GS02](../../interaction-frame-v2-全量替换-按UC/GS02/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
