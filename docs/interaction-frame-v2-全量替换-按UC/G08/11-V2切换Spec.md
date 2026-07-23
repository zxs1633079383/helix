# G08 V2-only 切换 Spec

**Gate**: G08 | **父规格**: `018-interaction-frame-v2-cutover` | **状态**: Draft

## 目标与用户场景

定时任务创建者选择时间并提交，owner-only 定时状态在权威创建并持久化后出现；普通频道成员在真正发送前不可看到草稿。

## 合同与影响面

| 维度 | G08 合同 |
|---|---|
| 输入/命令 | `ActionBinding(schedule_create)` → `im_create_schedule` |
| Effect/Persist | http_write、storage_upsert、projection；`channel_schedule` |
| Go | `/posts/createSchedule`；WS `post_schedule_created` |
| PG | `post_schedule`、`channel`、`post` |
| SQLite | `channel_schedule` |
| Surface/VM | channel-summary、left-dialog-list、可选 timeline |
| Angular | message-ui-schedule-dialog、message-ui-overlay-host |

## 专属证据清单

- [ ] schedule time、owner、channel 和摘要在 PG/SQLite/Frame 一致。
- [ ] owner attach/hydrate 可恢复 banner；observer 在发送前无草稿泄漏。
- [ ] accepted 不关闭为业务成功；权威事实 + PersistOk 后才更新。
- [ ] 过去时间、无权限、重复创建、超时和 PersistErr 保留表单与旧 Frame。
- [ ] 日期/时间输入、提交态、owner banner 和双尺寸截图正常。

## 验收标准

owner-only 定时状态正确建立且无隐私泄漏；纯 V2 两轮通过。

## 执行入口

工具与公共证据面见 [`../09-逐Gate-Spec公共验收合同.md`](../09-逐Gate-Spec公共验收合同.md)；字段细节见本目录 `06`-`10`。
