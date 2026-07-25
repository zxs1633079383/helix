# InteractionFrame V2 26 Gate Plan/Task 索引

> 机器真源：[plan-task-index.json](plan-task-index.json)。只打开当前 Gate 的 Plan/Tasks，避免把 26 个实现上下文一次性加载。

| 顺序 | Gate | 业务 | Plan | Tasks | Quickstart | 前置 |
|---:|---|---|---|---|---|---|
| 1 | G-01 | 发送文本基线 | [plan](../019-g01-wdio-five-phases/plan.md) | [tasks](../019-g01-wdio-five-phases/tasks.md) | [verification](../019-g01-wdio-five-phases/verification.md) | baseline reverify |
| 2 | G-02 | 发文档 | [plan](../025-ifv2-g-02-document-message/plan.md) | [tasks](../025-ifv2-g-02-document-message/tasks.md) | [quickstart](../025-ifv2-g-02-document-message/quickstart.md) | G-01 review-green |
| 3 | G-03a | 重发失败 | [plan](../026-ifv2-g-03a-retry-failed-send/plan.md) | [tasks](../026-ifv2-g-03a-retry-failed-send/tasks.md) | [quickstart](../026-ifv2-g-03a-retry-failed-send/quickstart.md) | G-02 review-green |
| 4 | G-04 | 撤回 | [plan](../027-ifv2-g-04-online-revoke/plan.md) | [tasks](../027-ifv2-g-04-online-revoke/tasks.md) | [quickstart](../027-ifv2-g-04-online-revoke/quickstart.md) | G-03a review-green |
| 5 | G-04a | 离线撤回重放 | [plan](../028-ifv2-g-04a-offline-revoke/plan.md) | [tasks](../028-ifv2-g-04a-offline-revoke/tasks.md) | [quickstart](../028-ifv2-g-04a-offline-revoke/quickstart.md) | G-04 review-green |
| 6 | G-05 | 转发/合并转发 | [plan](../029-ifv2-g-05-forward-message/plan.md) | [tasks](../029-ifv2-g-05-forward-message/tasks.md) | [quickstart](../029-ifv2-g-05-forward-message/quickstart.md) | G-04a review-green |
| 7 | G-06 | 快捷回复 emoji | [plan](../030-ifv2-g-06-quick-reaction/plan.md) | [tasks](../030-ifv2-g-06-quick-reaction/tasks.md) | [quickstart](../030-ifv2-g-06-quick-reaction/quickstart.md) | G-05 review-green |
| 8 | G-07 | 加急 + 确认 | [plan](../031-ifv2-g-07-urgent-message/plan.md) | [tasks](../031-ifv2-g-07-urgent-message/tasks.md) | [quickstart](../031-ifv2-g-07-urgent-message/quickstart.md) | G-06 review-green |
| 9 | G-08 | 定时消息 | [plan](../032-ifv2-g-08-schedule-create/plan.md) | [tasks](../032-ifv2-g-08-schedule-create/tasks.md) | [quickstart](../032-ifv2-g-08-schedule-create/quickstart.md) | G-07 review-green |
| 10 | G-09 | 取消定时 | [plan](../033-ifv2-g-09-schedule-cancel/plan.md) | [tasks](../033-ifv2-g-09-schedule-cancel/tasks.md) | [quickstart](../033-ifv2-g-09-schedule-cancel/quickstart.md) | G-08 review-green |
| 11 | G-10 | 切群首屏 | [plan](../034-ifv2-g-10-timeline-window/plan.md) | [tasks](../034-ifv2-g-10-timeline-window/tasks.md) | [quickstart](../034-ifv2-g-10-timeline-window/quickstart.md) | G-09 review-green |
| 12 | G-10a | 上拉更早历史 | [plan](../035-ifv2-g-10a-load-older/plan.md) | [tasks](../035-ifv2-g-10a-load-older/tasks.md) | [quickstart](../035-ifv2-g-10a-load-older/quickstart.md) | G-10 review-green |
| 13 | G-11 | 按 postId 定位 | [plan](../036-ifv2-g-11-locate-message/plan.md) | [tasks](../036-ifv2-g-11-locate-message/tasks.md) | [quickstart](../036-ifv2-g-11-locate-message/quickstart.md) | G-10a review-green |
| 14 | G-12 | 回复列表/分支 | [plan](../037-ifv2-g-12-reply-thread/plan.md) | [tasks](../037-ifv2-g-12-reply-thread/tasks.md) | [quickstart](../037-ifv2-g-12-reply-thread/quickstart.md) | G-11 review-green |
| 15 | G-13 | 会话已读 | [plan](../038-ifv2-g-13-channel-read/plan.md) | [tasks](../038-ifv2-g-13-channel-read/tasks.md) | [quickstart](../038-ifv2-g-13-channel-read/quickstart.md) | G-12 review-green |
| 16 | G-14 | 单条已读 | [plan](../039-ifv2-g-14-post-read/plan.md) | [tasks](../039-ifv2-g-14-post-read/tasks.md) | [quickstart](../039-ifv2-g-14-post-read/quickstart.md) | G-13 review-green |
| 17 | G-14a | 模板已收到 | [plan](../040-ifv2-g-14a-template-received/plan.md) | [tasks](../040-ifv2-g-14a-template-received/tasks.md) | [quickstart](../040-ifv2-g-14a-template-received/quickstart.md) | G-14 review-green |
| 18 | G-15a | 创建群聊 | [plan](../041-ifv2-g-15a-create-channel/plan.md) | [tasks](../041-ifv2-g-15a-create-channel/tasks.md) | [quickstart](../041-ifv2-g-15a-create-channel/quickstart.md) | G-14a review-green |
| 19 | G-15 | 创建话题 | [plan](../042-ifv2-g-15-create-topic/plan.md) | [tasks](../042-ifv2-g-15-create-topic/tasks.md) | [quickstart](../042-ifv2-g-15-create-topic/quickstart.md) | G-15a review-green |
| 20 | G-16 | 关闭/退出群 | [plan](../043-ifv2-g-16-channel-close-leave/plan.md) | [tasks](../043-ifv2-g-16-channel-close-leave/tasks.md) | [quickstart](../043-ifv2-g-16-channel-close-leave/quickstart.md) | G-15 review-green |
| 21 | G-16b | member leave 广播 | [plan](../044-ifv2-g-16b-member-leave-broadcast/plan.md) | [tasks](../044-ifv2-g-16b-member-leave-broadcast/tasks.md) | [quickstart](../044-ifv2-g-16b-member-leave-broadcast/quickstart.md) | G-16 review-green |
| 22 | G-16c | 群属性修改 | [plan](../045-ifv2-g-16c-channel-settings/plan.md) | [tasks](../045-ifv2-g-16c-channel-settings/tasks.md) | [quickstart](../045-ifv2-g-16c-channel-settings/quickstart.md) | G-16b review-green |
| 23 | G-S1 | hello 全量增量 | [plan](../021-ifv2-g-s1-cold-start-sync/plan.md) | [tasks](../021-ifv2-g-s1-cold-start-sync/tasks.md) | [quickstart](../021-ifv2-g-s1-cold-start-sync/quickstart.md) | G-16c review-green |
| 24 | G-S2 | sync notify | [plan](../022-ifv2-g-s2-offline-sync-notify/plan.md) | [tasks](../022-ifv2-g-s2-offline-sync-notify/tasks.md) | [quickstart](../022-ifv2-g-s2-offline-sync-notify/quickstart.md) | G-S1 review-green |
| 25 | G-S3 | 心跳 gap 补偿 | [plan](../023-ifv2-g-s3-pong-gap-recovery/plan.md) | [tasks](../023-ifv2-g-s3-pong-gap-recovery/tasks.md) | [quickstart](../023-ifv2-g-s3-pong-gap-recovery/quickstart.md) | G-S2 review-green |
| 26 | G-S4 | 陌生 channel 兜底 | [plan](../024-ifv2-g-s4-unknown-channel-hydration/plan.md) | [tasks](../024-ifv2-g-s4-unknown-channel-hydration/tasks.md) | [quickstart](../024-ifv2-g-s4-unknown-channel-hydration/quickstart.md) | G-S3 review-green |

## 每 Gate 固定产出

Helix 完整场景与 canonical Fixture/Golden；PC WDIO Mock/Real/assertion/report；Mobile mirror/GTest/production SDK-Bridge-SurfaceRuntime/QuickJS typed view；行为、代码与 Frame V2 Review。
