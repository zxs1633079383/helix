# GS01 V2-only 切换 Spec

**Gate**: GS01 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

重新上线用户建立连接并收到 `hello` 后，Helix 通过 HostObservation 完成全量增量恢复；只有已持久化频道事实才能形成会话列表和未读摘要 V2 Frame。同账号窗口及频道成员按各自 viewer 权限得到一致结果。

## 合同与影响面

| 维度 | GS01 合同 |
|---|---|
| 输入/命令 | `HostObservation` → `im_query_dialog_list` |
| Effect/Persist | `local_scan`、`projection`；PersistAtomic：`channel`、`channel_member`、`channel_event_cursor` |
| Go | `/channels/load/increment`、`/channel/load/incrementByChannelId`；`hello`、`increment_channel.data/end` |
| PG | `channel`、`channel_member`、`channel_event`、`post` |
| SQLite | `channel`、`channel_member`、`channel_event_cursor` |
| Surface/VM | `message.left-dialog-list`、`message.unread-summary`；DialogListVm + UnreadSummaryVm |
| Angular | message.component、message-surface-runtime-view、message-ui-surface-dialog-list |

## 专属证据清单

- [ ] `increment_channel_end` 只标记传输批结束；必须继续证明最后一个 PersistOk、cursor 与 V2 Frame。
- [ ] PG 水位、SQLite `channel_event_cursor` 和 Frame revision 分别记录且不混为同一字段。
- [ ] actor 与同账号其他窗口的会话行、未读数和 viewer 可见性正确。
- [ ] 空增量、重复快照、乱序水位和 PersistErr 保留上一版 UI，不发布伪成功。
- [ ] DOM 会话行、未读摘要、ARIA 状态及 444/678 截图与同一恢复运行关联。

## 验收标准

恢复完成后会话列表与未读摘要可操作、无重复/空白；所有业务 Frame 为 V2，V1/raw fallback 为零；首轮纯 V2和删除 V1 后两轮均通过公共验收合同。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
