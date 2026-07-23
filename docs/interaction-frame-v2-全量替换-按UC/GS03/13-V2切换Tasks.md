# GS03 V2-only Tasks

- [ ] T027 [P] [US1] 在 `crates/helix-im/src/send/pong_compensate_tests.rs`、`tests/p4_sync_scheduler_test.rs` 和 cses-client 同步状态测试增加普通 pong/重复 gap/并发/no_change/PersistErr RED 用例
- [ ] T028 [US1] 在 `crates/helix-im/src/send/pong_compensate.rs` 与 `src/sync/scheduler.rs` 实现 only-gap 去重、有界队列/重试和 PersistOk 后 V2 runtime/surface compose
- [ ] T029 [US1] 在 cses-client Native diagnostics/evidence collector 记录 GS03 gap decision、queue、PG head、SQLite cursor、V2 FrameApplied 和非 gap 零请求断言

**Done**: 无自激恢复或全频道扇出，focused PASS 后进入最终 candidate。
