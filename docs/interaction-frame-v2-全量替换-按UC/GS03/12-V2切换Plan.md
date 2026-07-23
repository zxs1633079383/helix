# GS03 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

只对 `pong.gaps/hashMismatch` 指定频道执行有界补偿，禁止普通 pong 触发全频道扫描或自激队列。

## 影响文件

- Helix：`src/send/pong_compensate.rs`、`src/sync/scheduler.rs`、`tests/p4_sync_scheduler_test.rs`
- cses-client：同步运行态组件/测试与 Native diagnostics
- 数据：PG `channel_event/post/channel`；SQLite `channel_event_cursor/message`

## 实施顺序

1. RED：普通 pong、单 gap、重复 gap、并发 gap、no_change、断网/PersistErr。
2. Helix：按 gap key 去重排队，限制 in-flight/重试，只在 PersistOk 后投影。
3. Client：同步状态不清空最后一致 Surface。
4. Evidence：记录 decision、queue depth、gap/head/cursor 和非 gap 零请求。

## 完成闸门

非 gap 频道零 sync request；队列有界；UI 不闪空；V1/raw 零命中。
