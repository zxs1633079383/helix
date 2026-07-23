# G01 V2-only 切换 Spec

**Gate**: G01 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

发送者提交文本后，同一消息行从 temporary identity 的发送中状态收敛到服务端 canonical identity；观察者只在权威 `post` 到达并持久化后看到消息。

## 合同与影响面

| 维度 | G01 合同 |
|---|---|
| 输入/命令 | `ActionBinding(message.create)` → `im_send_message` |
| Effect/Persist | storage_upsert、http_write、timer、projection；`message`、`pending_send` |
| Go | `/posts/create`；WS `post`，本地失败 `im:post:send-failed` |
| PG | `post`、`channel`、`channel_member`、`channel_event` |
| SQLite | `message`、`pending_send` |
| Surface/VM | composer、timeline、left-dialog-list、unread-summary；四个完整 VM |
| Angular | message-ui-composer、message-surface-runtime-view、message-ui-surface-dialog-list |

## 专属证据清单

- [ ] temporaryId 在 PG `post.temporary_id`、SQLite pending/message 与 Frame 中关联到唯一 server id。
- [ ] HTTP SUCCESS 不结束 pending；matching WS/sync authority + PersistOk 才发布 sent Frame。
- [ ] actor 行原地收敛且不重复；observer 得到独立 viewer-correct V2 Frame。
- [ ] timeout、reject、late echo、duplicate echo 和 retry 均有确定终态。
- [ ] composer 清空/恢复、消息 delivery-kind、会话预览和未读摘要同一 causation 正确。

## 验收标准

一条提交只产生一条 canonical 消息，双用户 UI、PG、SQLite 一致；V1/raw fallback 为零，纯 V2 两轮回归通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
