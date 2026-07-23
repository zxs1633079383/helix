# G13 V2-only Tasks

- [ ] T066 [P] [US1] 在 `crates/helix-im/tests/g13_channel_read_interaction_test.rs` 与 cses-client `g13-channel-read-interaction.test.ts` 增加单调/重复/节流/viewer/PersistErr RED
- [ ] T067 [US1] 在 Helix read outbound/WS、dialog/unread/timeline projectors 与 SurfaceComposer 实现 read boundary PersistOk 后 V2 atomic update
- [ ] T068 [US1] 在 cses-client read-reporting/dialog/runtime 与 evidence collector 验证 PG/SQLite boundary、双用户未读 DOM 和零乐观清零

**Done**: viewer-isolated monotonic read focused PASS，进入最终 candidate。
