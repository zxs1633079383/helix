# G08 Handoff

## 状态

`PARTIAL - WAITING_SHARED_CHAIN_AND_RUST_AGGREGATE`

## 已完成

- `getSchedule` owner/channel 双重校验已沉入 `owner_schedule_hydration`，非 owner 不返回正文。
- 取消后的 schedule projection 不再携带已退役 draft 正文或摘要。
- 新增 focused Rust 合同 `g08_schedule_create_interaction_test.rs`。
- 新增 schedule dialog Angular 测试；owner hydrate、提交 intent、失败保留 draft、unavailable fail-closed 已覆盖。

## 证据

- Angular focused batch: 2026-07-22，本组所在 3 suites 共 `16/16 PASS`，wall-clock `25.33s`。
- Rust 第一次 `cargo check -p helix-im` 在共享 `module/action.rs` 的临时 `EffectSink::into_vec` 编译错误处停止；该共享错误已由 root 修复，本 agent 按 Cargo 纪律不重复运行。

## 待 root 收敛

- `message.schedule` binding、ActionInvoked、HTTP accepted 非终态、`post_schedule_created` authority、PersistOk 后完整 Frame 属共享 action/correlation/persist 链。
- Rust aggregate/focused PASS 后再勾选 T002/T010；最终双账号/PG/SQLite/Frame/DOM/visual 只由 017 建证。
