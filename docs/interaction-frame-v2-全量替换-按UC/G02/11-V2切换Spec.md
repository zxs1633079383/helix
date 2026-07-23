# G02 V2-only 切换 Spec

**Gate**: G02 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

发送者提交文档消息后，Helix 使用 typed document VM 表达发送中、成功与失败；Angular 不解析 raw props 决定业务状态。

## 合同与影响面

| 维度 | G02 合同 |
|---|---|
| 输入/命令 | `ActionBinding(message.create)` → `im_send_message` |
| Effect/Persist | upload、storage_upsert、http_write、projection；`message`、`pending_send` |
| Go | `/posts/create`，`type=DOCUMENT`；WS `post` |
| PG | `post`、`channel`、`channel_member`、`channel_event` |
| SQLite | `message`、`pending_send` |
| Surface/VM | composer、timeline；ComposerVm + typed document TimelineWindowVm |
| Angular | message-ui-composer、message-document-content |

## 专属证据清单

- [ ] 文档 URL/title/snapshot/version 从权威 props 归一为 typed VM，未知版本 fail-closed。
- [ ] temporaryId/serverId 与 G01 同样唯一收敛，失败恢复输入。
- [ ] PG props、SQLite typed message 与 Frame 内容语义一致，日志不输出文档正文或敏感 URL token。
- [ ] actor/observer 均显示正确文档控件，不能退化为 raw JSON 或普通文本。
- [ ] 缺字段、上传失败、late echo、重复 echo 和 PersistErr 均不制造 sent 状态。

## 验收标准

文档消息以 typed VM 正确渲染并唯一收敛；无 V1 consumer 或 raw props reducer，纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
