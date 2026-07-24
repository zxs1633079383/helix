# G04 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 撤回操作或 type-3 同步事实 |
| Helix 输入 | `ActionBinding(revoke)` / `Inbound` |
| Command | `im_revoke` |
| QuickJS binding | `im.posts.revoke` |
| Surface | `message.timeline`、`message.thread-detail`、`message.left-dialog-list` |
| 完整 VM | TimelineWindowVm + ThreadDetailVm + DialogListVm |
| 移动 UI 结果 | 发送者撤回后时间线、线程和会话摘要同步显示撤回态，原操作按钮失效 |
| 共享 PC/Helix 真源 | [G04](../../interaction-frame-v2-全量替换-按UC/G04/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
