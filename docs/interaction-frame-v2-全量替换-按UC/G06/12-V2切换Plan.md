# G06 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

reaction/快捷回复只从服务端权威聚合形成 timeline/thread typed VM，客户端不乐观累加业务计数。

## 影响文件

- Helix：quick-reply outbound/WS normalize、timeline/thread projectors、`tests/g06_reaction_interaction_test.rs`
- cses-client：quick-actions/action-menu 与 G06 tests
- 数据：PG `post/channel_event`；SQLite `message`

## 实施顺序

RED 覆盖并发、重复、切换/撤销、拒绝/PersistErr；实现权威聚合 persist/compose；客户端机械渲染选中态和数量；双用户对账。

## 完成闸门

计数和 viewer 选中态准确，零端侧 business reduce。
