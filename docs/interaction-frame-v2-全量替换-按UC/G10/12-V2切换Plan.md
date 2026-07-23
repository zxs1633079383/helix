# G10/G10a V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

首屏/向前补窗使用 local-first 有界窗口；Complete 零 HTTP，缺口才回源，V2 commit 后保持滚动锚点。

## 影响文件

- Helix：query render-ready/timeline projector、`tests/g10_timeline_window_interaction_test.rs`
- cses-client：timeline adapter/runtime-view、scroll-anchor-store 与 tests
- 数据：PG `post/channel`；SQLite `message`

## 实施顺序

RED 覆盖 complete/incomplete/end/并发/stale reply/PersistErr；实现 bounded query/persist/V2 window；客户端 commit 后 anchor reconcile；窗口 identity/delta 证据。

## 完成闸门

Complete 零出网、无重复/跳顶，V1/raw 零命中。
