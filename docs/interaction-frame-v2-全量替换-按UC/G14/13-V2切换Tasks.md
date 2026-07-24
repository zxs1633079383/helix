# G14/G14a V2-only Tasks

- [X] T069 [P] [US1] 在 `crates/helix-im/tests/g14_receipt_template_interaction_test.rs` 与 cses-client `g14-receipt-template-interaction.test.ts` 增加重复/跨 post/viewer/语义混写/PersistErr RED
- [X] T070 [US1] 在 Helix post-read outbound/WS、receipt/timeline/thread projectors 与 SurfaceComposer 实现分离 receipt/template V2 状态
- [X] T071 [US1] 在 cses-client read-receipt/template-content/runtime 与 evidence collector 验证 PG/SQLite receipt、author/reader DOM 和术语不混写

**Done**: 两种语义独立且幂等，进入最终 candidate。
