# G14/G14a V2-only 切换 Spec

**Gate**: G14、G14a | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

阅读用户上报单条已读，或对模板消息点击已收到；作者看到权威读者列表或模板收到状态，二者不能混为同一语义。

## 合同与影响面

| 维度 | G14 合同 |
|---|---|
| 输入/命令 | post_read/template_received binding 或 HostObservation |
| Effect/Persist | http_write、storage_upsert、projection；`message_receipt` |
| Go | `/post/read`；WS `post_read` |
| PG | `post`、`channel_member`、`channel_event` |
| SQLite | `message_receipt` |
| Surface/VM | participant-receipts、timeline、thread-detail |
| Angular | message-ui-read-receipt、message-template-content |

## 专属证据清单

- [ ] 单条 read 与 templateReceived 使用不同 action/状态字段。
- [ ] PG reader identity、SQLite receipt 与 author/reader 两视角 Frame 一致。
- [ ] 重复上报幂等，非 viewer、跨 post 和 stale binding 被拒绝。
- [ ] receipt drawer、模板按钮和 thread/timeline 状态同步。
- [ ] “已收到”不被文档或日志误写成业务确认/审批。

## 验收标准

读者列表与模板已收到语义独立、幂等、viewer-correct；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
