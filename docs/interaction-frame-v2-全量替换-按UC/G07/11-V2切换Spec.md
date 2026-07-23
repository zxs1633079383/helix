# G07 V2-only 切换 Spec

**Gate**: G07 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

**执行**: [Plan](12-V2切换Plan.md) → [Tasks](13-V2切换Tasks.md)

## 目标与用户场景

发起者加急消息，被加急用户确认；权威收件事实驱动加急标识、确认人数和按钮权限。

## 合同与影响面

| 维度 | G07 合同 |
|---|---|
| 输入/命令 | `ActionBinding(urgent/urgent_confirm)` → 两个 IM command |
| Effect/Persist | http_write、projection；`message`、`message_receipt` |
| Go | `/posts/urgentPost`、`/posts/urgentConfirm`；`post_update`、`update_channel` |
| PG | `post`、`channel`、`channel_member`、`channel_event` |
| SQLite | `message`、`message_receipt` |
| Surface/VM | timeline、participant-receipts、unread-summary |
| Angular | message-ui-urgent、message-ui-read-receipt、surface runtime view |

## 专属证据清单

- [ ] 发起与确认使用不同 binding/correlation，权限不能互换。
- [ ] PG targetIds/确认事实与 SQLite receipt 按 actor/observer 对账。
- [ ] 未确认、已确认、重复确认、非目标确认和拒绝均有确定 VM。
- [ ] 确认人数、目标列表和 unread/urgent 标记在相关 Surface 一致。
- [ ] 双用户按钮权限、状态和截图正确。

## 验收标准

加急和确认状态按 viewer 正确、幂等且可追踪；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
