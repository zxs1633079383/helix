# G07 Handoff

## 状态

`IN_PROGRESS`。三命令 canonical wire、输入零信任与 Angular empty/submitting/failed/forbidden 状态已覆盖；三类 binding 和四 viewer terminal Frame 待共享 runtime 收口。

## 已完成

- urgentPost/confirm/cancel 接受严格字段集；targetIds 有界、非空、无重复，actor/channel/userId 禁止由 renderer 注入。
- Urgent panel 暴露 `data-urgent-state` 与 `aria-busy`，空目标、无权限、提交态不会误触发完成。
- 新增 G07 Rust/Angular focused tests。

## 真实验证

- `2026-07-22T12:01:49Z..12:01:56Z`：G05/G07 focused Jest，7s，2 suites/2 tests PASS。
- 既有 UC05/UC08 与 production visuals 在相邻回归中 16/16 PASS。
- `2026-07-22T12:05:32Z..12:05:59Z`：`cargo check -p helix-im --tests`，27s，PASS；未单独运行 Native build。

## 待完成

- shared `UrgentPost/UrgentConfirm/UrgentCancel` ActionDispatch 与 `canUrgentCancel` authority capability。
- 发起者、目标、非目标、无权限四 viewer 的 PersistOk 后完整 Frame/causation。
- Rust focused test PASS、017 双账号/双库/视觉。
