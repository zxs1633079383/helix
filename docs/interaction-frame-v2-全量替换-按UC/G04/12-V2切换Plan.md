# G04/G04a V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

在线 revoke 与离线 type-3 事实归一为相同 MessageRecalled write-set 和 timeline/thread/dialog V2 结果。

## 影响文件

- Helix：`tests/g04_revoke_interaction_test.rs`、revoke action/WS normalize、projector/surface composer
- cses-client：action-menu、message content、thread drawer 与 G04 tests
- 数据：PG `post/channel/channel_member/channel_event`；SQLite `message/channel_event_cursor`

## 实施顺序

RED 覆盖 online 1 HTTP/offline 0 HTTP、权限、重复/乱序/PersistErr；实现共同 authority normalize/persist/compose；客户端只按完整撤回 VM 渲染并失效旧 binding；登记双库/双用户证据。

## 完成闸门

在线/离线撤回同态，无残留旧动作，V1/raw 零命中。
