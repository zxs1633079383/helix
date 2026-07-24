# GS03 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | `pong.gaps` |
| Helix 输入 | `Inbound` + `Timer` |
| Command | `im_sync_channels` |
| QuickJS binding | `im.sync.channels` |
| Surface | `message.left-dialog-list`、`message.unread-summary`、`message.timeline` |
| 完整 VM | InteractionRuntimeVm + 受影响 Surface VM |
| 移动 UI 结果 | 补偿期间显示同步状态但不清空旧视图；合法补偿提交后只刷新受影响 Surface |
| 共享 PC/Helix 真源 | [GS03](../../interaction-frame-v2-全量替换-按UC/GS03/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
