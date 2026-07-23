# G11 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

定位目标消息时先提交包含目标的 V2 window，随后执行 reveal/focus directive；失败不移动当前视图。

## 影响文件

- Helix：`query/render_ready/locate.rs`、timeline projector、`tests/g11_locate_interaction_test.rs`
- cses-client：timeline/runtime/scroll-anchor 与 tests
- 数据：PG `post`；SQLite `message`

## 实施顺序

RED 覆盖 local hit/remote context/not-found/forbidden/stale/PersistErr；实现 context persist→Frame→directive；客户端 post-commit reveal；目标 identity/DOM 证据。

## 完成闸门

directive 不先于 commit，失败零滚动。
