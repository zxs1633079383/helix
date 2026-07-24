# G11 Mobile InteractionFrame V2 替换

| 项目 | 合同 |
|---|---|
| 用户入口 | 请求定位 `postId` |
| Helix 输入 | `ActionBinding(locate)` |
| Command | `im_get_posts` |
| QuickJS binding | `im.posts.locate` |
| Surface | `message.timeline` |
| 完整 VM | TimelineWindowVm + RevealMessage directive |
| 移动 UI 结果 | 目标窗口提交后执行 RevealMessage，目标不存在或失败时不移动当前滚动位置 |
| 共享 PC/Helix 真源 | [G11](../../interaction-frame-v2-全量替换-按UC/G11/README.md) |

本 Gate 先冻结 V1，再切断 producer 与 consumer，首轮纯 V2 后物理删除 V1，最后二次验收。
