# G12 V2-only 切换 Spec

**Gate**: G12 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

用户打开回复线程、分支或提交回复时，Helix 建立完整 root/replies 图；线程抽屉与主时间线共享 canonical message identity。

## 合同与影响面

| 维度 | G12 合同 |
|---|---|
| 输入/命令 | `ActionBinding(open_reply_tree/reply)` → get replies/branch |
| Effect/Persist | http_read、projection；`message`、`reply_tree` |
| Go | `/posts/getReplies`、`/posts/getReplyBranch`；无直接 WS |
| PG | `post` |
| SQLite | `message`、`reply_tree` |
| Surface/VM | thread-detail、timeline；ThreadDetailVm + TimelineWindowVm |
| Angular | message-ui-surface-thread-drawer、message-ui-reply-drawer |

## 专属证据清单

- [ ] replyId/rootId/firstLevelId 三锚在 PG、SQLite 与 Frame 图中一致。
- [ ] 分页/分支结果去重且顺序稳定，root 不重复。
- [ ] 回复提交复用 G01 canonical identity，不由 Angular拼装关系。
- [ ] 缺 root、跨频道、循环关系、重复页和 PersistErr fail-closed。
- [ ] 抽屉 loading/empty/ready/failed 与主时间线引用一致。

## 验收标准

回复图完整、无环、身份统一，主线与抽屉不漂移；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
