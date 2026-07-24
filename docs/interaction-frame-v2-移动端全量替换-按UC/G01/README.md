# G01 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 输入文本并提交 |
| Helix 输入 | `ActionBinding(message.create)` |
| Command | `im_send_message` |
| QuickJS binding | `im.send` |
| Surface | `message.composer`、`message.timeline`、`message.left-dialog-list`、`message.unread-summary` |
| 完整 VM | ComposerVm + TimelineWindowVm + DialogListVm + UnreadSummaryVm |
| 移动 UI 结果 | 同一消息行从 sending 收敛为 sent 或 failed，禁止 JS 再造第二条临时消息 |
| 共享 PC/Helix 真源 | [G01](../../interaction-frame-v2-全量替换-按UC/G01/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
