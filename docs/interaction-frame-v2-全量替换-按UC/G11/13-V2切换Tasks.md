# G11 V2-only Tasks

- [ ] T060 [P] [US1] 在 `crates/helix-im/tests/g11_locate_interaction_test.rs` 与 cses-client timeline/anchor tests 增加 local/remote/not-found/stale/PersistErr RED 用例
- [ ] T061 [US1] 在 `crates/helix-im/src/query/render_ready/locate.rs`、timeline projector 与 SurfaceComposer 实现 context V2 Frame 和 post-commit reveal directive
- [ ] T062 [US1] 在 cses-client runtime/scroll-anchor 与 evidence collector 验证 PG/SQLite target、FrameApplied 顺序和目标 DOM focus

**Done**: commit-before-reveal 且失败不扰动，进入最终 candidate。
