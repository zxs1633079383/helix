# G10/G10a V2-only Tasks

- [ ] T057 [P] [US1] 在 `crates/helix-im/tests/g10_timeline_window_interaction_test.rs` 与 cses-client timeline/anchor tests 增加 complete/incomplete/end/并发/stale/PersistErr RED 用例
- [ ] T058 [US1] 在 Helix query render-ready、timeline projector 与 SurfaceComposer 实现 bounded local-first/backfill V2 window 和 revision guard
- [ ] T059 [US1] 在 cses-client timeline adapter/runtime-view/scroll-anchor-store 与 evidence collector 验证零出网、PG/SQLite window、DOM 去重和 anchor

**Done**: 有界窗口与滚动稳定 focused PASS，进入最终 candidate。
