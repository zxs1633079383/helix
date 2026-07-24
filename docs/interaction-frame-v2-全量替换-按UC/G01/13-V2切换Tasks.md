# G01 V2-only Tasks

- [X] T033 [P] [US1] 在 `crates/helix-im/tests/send_test.rs`、`f1_send_http_reconcile_test.rs` 与 cses-client composer/runtime 测试增加 binding、HTTP 2xx、duplicate/late echo、PersistErr 的 G01 RED 用例
- [X] T034 [US1] 在 `crates/helix-im/src/module/action.rs`、`src/send/{flow,pending,reconcile}.rs`、`src/surface_composer.rs` 实现 temporary→canonical 状态机和 V2-only Frame
- [X] T035 [US1] 在 cses-client composer/projection/runtime 与 Native evidence collector 验证 clear-after-apply、双用户 DOM、PG `post.temporary_id/id` 和 SQLite `message/pending_send`

**Done**: 单行唯一收敛且双用户 focused PASS，进入最终 candidate。
