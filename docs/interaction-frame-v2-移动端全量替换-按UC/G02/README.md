# G02 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 选择文档内容并提交 |
| Helix 输入 | `ActionBinding(message.create)` |
| Command | `im_send_message` |
| QuickJS binding | `im.send` |
| Surface | `message.composer`、`message.timeline` |
| 完整 VM | ComposerVm + TimelineWindowVm（文档内容） |
| 移动 UI 结果 | 文档消息显示文档卡片、发送状态和失败后的输入恢复，不由 JS 解读 document props |
| 共享 PC/Helix 真源 | [G02](../../interaction-frame-v2-全量替换-按UC/G02/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
