# G05 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 选择目标并提交转发 |
| Helix 输入 | `ActionBinding(forward)` |
| Command | `im_relay_messages` |
| QuickJS binding | `im.posts.relayMessages` |
| Surface | `message.timeline`、`message.left-dialog-list`、`message.composer` |
| 完整 VM | 源/目标 TimelineWindowVm + DialogListVm |
| 移动 UI 结果 | 每个目标频道独立呈现转发终态，源频道与失败目标不被假成功覆盖 |
| 共享 PC/Helix 真源 | [G05](../../interaction-frame-v2-全量替换-按UC/G05/README.md) |
