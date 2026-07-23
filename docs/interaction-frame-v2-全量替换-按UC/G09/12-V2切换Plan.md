# G09 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

owner 取消定时消息后，以权威 terminal 状态移除 banner/会话标记；重复取消幂等。

## 影响文件

- Helix：cancel outbound/WS handler、summary composer、`tests/g09_schedule_cancel_interaction_test.rs`
- cses-client：schedule/confirm dialog 与 tests
- 数据：PG `post_schedule/channel/channel_member`；SQLite `channel_schedule`

## 实施顺序

RED 覆盖跨 actor、重复取消、失败/PersistErr；实现 terminal persist/V2 compose；客户端保留失败可重试态；owner/observer 证据。

## 完成闸门

重复取消零新增 mutation，无 viewer 泄漏。
