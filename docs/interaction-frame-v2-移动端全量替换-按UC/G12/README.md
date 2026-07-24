# G12 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 打开线程、分支或提交回复 |
| Helix 输入 | `ActionBinding(open_reply_tree/reply)` |
| Command | `im_get_replies` / `im_get_reply_branch` |
| QuickJS binding | `im.posts.getReplies`、`im.posts.getReplyBranch` |
| Surface | `message.thread-detail`、`message.timeline` |
| 完整 VM | ThreadDetailVm + TimelineWindowVm |
| 移动 UI 结果 | 线程 root/replies 图完整出现；回复发送终态与主时间线一致，不由 JS 拼 reply tree |
| 共享 PC/Helix 真源 | [G12](../../interaction-frame-v2-全量替换-按UC/G12/README.md) |
