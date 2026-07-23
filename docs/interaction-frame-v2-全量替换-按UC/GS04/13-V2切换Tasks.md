# GS04 V2-only Tasks

- [ ] T030 [P] [US1] 在 `crates/helix-im/tests/uc45_unknown_channel_post_test.rs`、`gs01_04_v2_sync_recovery_test.rs` 与 cses-client 恢复 DOM 测试增加 unknown/forbidden/PersistErr/pre-commit no-send RED 用例
- [ ] T031 [US1] 在 `crates/helix-im/src/channel/gate/flow.rs`、`src/sync/flow.rs`、`src/surface_composer.rs` 实现 hydrate write-set 和 post-commit summary/composer/timeline V2 bundle
- [ ] T032 [US1] 在 cses-client `message-surface.facade.ts`、composer/header/runtime-view 与 evidence collector 验证 PG/SQLite identity、binding 权限、FrameApplied 和三 Surface DOM

**Done**: PersistOk 前零 send capability，focused PASS 后进入最终 candidate。
