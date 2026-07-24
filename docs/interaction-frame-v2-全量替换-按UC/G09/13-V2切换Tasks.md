# G09 V2-only Tasks

- [X] T054 [P] [US1] 在 `crates/helix-im/tests/g09_schedule_cancel_interaction_test.rs` 与 cses-client schedule/confirm tests 增加跨 actor/重复/失败/PersistErr RED 用例
- [X] T055 [US1] 在 Helix cancel outbound/WS handler、storage effect 与 SurfaceComposer 实现 owner terminal V2 Frame 和幂等保护
- [X] T056 [US1] 在 cses-client schedule/confirm/runtime 与 evidence collector 验证双库 terminal、owner DOM、observer no-leak 和重复零 mutation

**Done**: cancel 幂等且 owner-only，进入最终 candidate。
