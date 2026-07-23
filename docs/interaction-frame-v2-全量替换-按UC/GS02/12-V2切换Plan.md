# GS02 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

把 sync notify 归一为连续有界 batch，并以 message+cursor 单次 PersistAtomic 解锁受影响 Surface V2 Frame。

## 影响文件

- Helix：`src/sync/{explicit,flow}.rs`、`src/channel/gate/{flow,commit}.rs`、`tests/gs01_04_v2_sync_recovery_test.rs`
- cses-client：`surface-runtime/`、恢复 DOM 测试与 Native evidence collector
- 数据：PG `channel_event/post/channel/channel_member`；SQLite `channel_event_cursor/message`

## 实施顺序

1. RED：full/mixed/no_change、满批续拉、重复 WS、PersistErr。
2. Helix：验证 event_seq 连续性，message/cursor 原子提交。
3. Client：保留最后一致缓存，matching V2 Frame 原子更新。
4. Evidence：fromSeq/nextSeq/PG head/SQLite cursor 同 correlation。

## 完成闸门

所有 batch 有界终止，cursor 不越过事实，V1/raw 零命中。
