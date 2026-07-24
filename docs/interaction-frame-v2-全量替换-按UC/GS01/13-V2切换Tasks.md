# GS01 V2-only Tasks

- [X] T021 [P] [US1] 在 `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs` 与 cses-client `surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts` 增加 GS01 V2-only、PersistErr、重复/乱序和 FrameApplied 失败测试并先证明 RED
- [X] T022 [US1] 在 `crates/helix-im/src/sync/{session,increment,flow}.rs` 与 `crates/helix-im/src/surface_composer.rs` 实现 GS01 matching PersistOk 后 dialog/unread V2 snapshot，禁止 V1 producer
- [X] T023 [US1] 在 cses-client `surface-runtime/` 和 `tests/e2e/desktop/harness/frame-evidence.ts` 接通 GS01 V2 apply/ack/DOM，并登记 PG `channel/channel_member/channel_event/post` 与 SQLite `channel/channel_member/channel_event_cursor` 只读 delta

**Done**: 三项 focused validator 通过，只标 `READY_FOR_FINAL_E2E`；最终 PASS 仍依赖首轮和删除后二次 25-Gate 回归。
