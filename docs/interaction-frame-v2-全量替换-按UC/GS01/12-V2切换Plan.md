# GS01 V2-only 切换 Plan

**Spec**: [11-V2切换Spec.md](11-V2切换Spec.md) | **聚合计划**: [`018/plan.md`](../../../specs/018-interaction-frame-v2-cutover/plan.md)

## 目标

把 hello/increment 恢复链收敛为 `HostObservation -> Tick/Effect -> channel/member/cursor PersistAtomic -> PersistOk -> dialog/unread V2 snapshot -> FrameApplied`。

## 影响文件

- Helix：`crates/helix-im/src/sync/{session,increment,flow}.rs`、`surface_composer.rs`、`tests/gs01_04_v2_sync_recovery_test.rs`
- cses-client：`surface-runtime/`、`gs01-04-sync-recovery-dom.test.ts`
- 数据：PG `channel/channel_member/channel_event/post`；SQLite `channel/channel_member/channel_event_cursor`

## 实施顺序

1. RED：补 empty/equal/duplicate/乱序/PersistErr 与 Frame V2/ack 断言。
2. Helix：只在恢复 write-set matching PersistOk 后 compose dialog/unread。
3. Client：V2-only apply，生产回传 FrameApplied，验证会话行/未读 DOM。
4. Evidence：记录 PG 水位、SQLite cursor、Frame revision 的独立 delta。

## 完成闸门

GS01 focused tests PASS；`increment_channel_end` 不被当完成；V1/raw 零命中；进入 25-Gate candidate。
