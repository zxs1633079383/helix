# G10a Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | attach 频道或上拉 |
| Helix 输入 | `HostObservation` / `ActionBinding(load_older)` |
| Command | `im_query_messages_by_channel` / `im_load_older_context` |
| QuickJS binding | `im.posts.queryMessagesByChannel`、`im.posts.loadOlderContext` |
| Surface | `message.timeline` |
| 完整 VM | TimelineWindowVm |
| 移动 UI 结果 | 围绕锚点补齐 before context，整窗提交后保持锚点位置，失败不破坏当前窗口 |
| 共享 PC/Helix 真源 | [G10](../../interaction-frame-v2-全量替换-按UC/G10/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
