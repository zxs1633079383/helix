# G13 V2-only 切换 Spec

**Gate**: G13 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

频道变为可见或用户点击已读时，Helix 更新频道已读边界和未读摘要；不允许 UI 自行清零后等待服务端补偿。

## 合同与影响面

| 维度 | G13 合同 |
|---|---|
| 输入/命令 | channel_read binding / HostObservation → `im_channels_view` |
| Effect/Persist | http_write、storage_upsert、projection；`channel`、`message_receipt` |
| Go | `/post/read`、`/channels/view`；`post_read`、`update_channel`、sync type=6 |
| PG | `post`、`channel_member`、`channel`、`channel_event` |
| SQLite | `channel`、`message_receipt` |
| Surface/VM | left-dialog-list、unread-summary、timeline |
| Angular | message-ui-surface-dialog-list、message-read-reporting |

## 专属证据清单

- [ ] read boundary 单调，旧/重复回执不能倒退。
- [ ] PG member/read authority、SQLite receipt 与三个 Surface 同一 causation。
- [ ] actor 未读数清零/递减准确，observer 不受错误 viewer 更新。
- [ ] 可见性 observation 有节流/合并，不形成逐消息请求风暴。
- [ ] 会话 badge、全局摘要和时间线边界同帧一致。

## 验收标准

已读边界单调且 viewer 隔离，未读显示一致；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
