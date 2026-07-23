# G09 V2-only 切换 Spec

**Gate**: G09 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

定时任务所有者确认取消后，owner-only 定时状态从频道摘要和会话行移除；其他用户不接收该草稿或取消详情。

## 合同与影响面

| 维度 | G09 合同 |
|---|---|
| 输入/命令 | `ActionBinding(schedule_cancel)` → `im_cancel_schedule` |
| Effect/Persist | http_write、storage_upsert、projection；`channel_schedule` |
| Go | `/posts/cancelSchedule`；WS `post_schedule_canceled` |
| PG | `post_schedule`、`channel`、`channel_member` |
| SQLite | `channel_schedule` |
| Surface/VM | channel-summary、left-dialog-list |
| Angular | message-ui-schedule-dialog、message-ui-confirm-dialog |

## 专属证据清单

- [ ] 只有 owner 有有效 cancel binding，跨 actor/actionRef 被拒绝。
- [ ] PG cancel 状态、SQLite 删除/terminal 状态和 V2 Frame 一致。
- [ ] 重复取消幂等，不产生第二次可见 mutation。
- [ ] 取消失败保留 owner banner 与可重试操作；observer 无数据泄漏。
- [ ] confirm dialog、pending、成功关闭和失败提示 DOM 正确。

## 验收标准

取消结果 owner-only、幂等且可恢复；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
