# G08 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

创建定时消息并在 owner attach/hydrate 后恢复 owner-only 状态；observer 在真正发送前零草稿泄漏。

## 影响文件

- Helix：schedule outbound/WS handler、channel summary composer、`tests/g08_schedule_create_interaction_test.rs`
- cses-client：schedule dialog/overlay/runtime 与 tests
- 数据：PG `post_schedule/channel/post`；SQLite `channel_schedule`

## 实施顺序

RED 覆盖过去时间、重复、拒绝、PersistErr、observer leak；实现 owner-scoped persist/V2 compose；客户端保持表单到权威终局；双用户证据。

## 完成闸门

owner 状态可恢复，observer 零草稿，V1/raw 零命中。
