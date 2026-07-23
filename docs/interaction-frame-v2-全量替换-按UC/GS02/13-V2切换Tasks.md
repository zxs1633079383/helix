# GS02 V2-only Tasks

- [ ] T024 [P] [US1] 在 `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs` 和 cses-client 恢复 DOM 测试增加 full/mixed/no_change/满批/重复/PersistErr 的 GS02 RED 用例
- [ ] T025 [US1] 在 `crates/helix-im/src/sync/{explicit,flow}.rs` 与 `src/channel/gate/{flow,commit}.rs` 实现连续 batch、message+cursor PersistAtomic 和 matching V2 compose
- [ ] T026 [US1] 在 cses-client `tests/e2e/desktop/harness/evidence-collector.ts` 登记 GS02 fromSeq/nextSeq、PG `channel_event/post`、SQLite `channel_event_cursor/message`、FrameApplied 与 DOM facet

**Done**: focused readiness 通过且无 cursor/Frame 提前推进；最终依赖两轮全量 E2E。
