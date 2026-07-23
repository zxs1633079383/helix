# G12 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

root/replies/branch 三锚归一为无环、去重的 reply graph，thread-detail 与 timeline 共享 canonical message identity。

## 影响文件

- Helix：`query/render_ready/replies.rs`、thread/timeline projectors、`tests/g12_reply_thread_interaction_test.rs`
- cses-client：thread/reply drawer、adapters 与 tests
- 数据：PG `post`；SQLite `message/reply_tree`

## 实施顺序

RED 覆盖分页/分支/缺 root/跨频道/循环/重复/PersistErr；实现 graph validate/persist/V2 compose；客户端机械渲染；三锚和 DOM 证据。

## 完成闸门

无环无重复，主线/抽屉 identity 一致。
