# G06 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 点击 emoji 或快捷回复 |
| Helix 输入 | `ActionBinding(react)` |
| Command | `im_quick_reply` |
| QuickJS binding | `im.posts.quickReply` |
| Surface | `message.timeline`、`message.thread-detail` |
| 完整 VM | TimelineWindowVm + ThreadDetailVm |
| 移动 UI 结果 | emoji 聚合数量、当前 actor 选中态和线程详情一致；拒绝时恢复旧聚合 |
| 共享 PC/Helix 真源 | [G06](../../interaction-frame-v2-全量替换-按UC/G06/README.md) |
