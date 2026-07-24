# GS04 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 选择频道或未知 `channelId` |
| Helix 输入 | `HostObservation` |
| Command | `im_channel_load_increment_by_channel_id` |
| QuickJS binding | `im.channels.ensureLoaded` |
| Surface | `message.channel-summary`、`message.composer`、`message.timeline` |
| 完整 VM | ChannelSummaryVm + ComposerVm + TimelineWindowVm |
| 移动 UI 结果 | 未知频道加载成功后频道头、输入区和时间线同时可用；失败时保留原频道 |
| 共享 PC/Helix 真源 | [GS04](../../interaction-frame-v2-全量替换-按UC/GS04/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
