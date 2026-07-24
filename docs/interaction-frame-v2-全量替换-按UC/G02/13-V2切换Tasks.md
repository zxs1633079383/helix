# G02 V2-only Tasks

- [X] T036 [P] [US1] 在 `crates/helix-im/tests/interaction_frame_contract_test.rs`、`send_test.rs` 与 cses-client content/codec tests 增加 DOCUMENT typed/unknown/missing/failure RED 用例
- [X] T037 [US1] 在 `crates/helix-im/src/interaction_frame.rs` 与 send content normalize 路径实现 DOCUMENT typed VM，并复用 G01 pending/canonical V2 flow
- [X] T038 [US1] 在 cses-client `surface-contract.*`、`message-document-content.component.ts` 和 evidence collector 删除可达 raw props 判定并登记双库/双用户证据

**Done**: typed document focused PASS，无 raw props reducer，进入最终 candidate。
