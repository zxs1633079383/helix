# G04/G04a Handoff

## 状态

`IN_PROGRESS`。Angular focused contract 已通过；Helix revoke ActionBinding 与 offline replay 的 PersistOk barrier 仍等待共享 runtime owner 合入，不能声明 `READY_FOR_FINAL_E2E`。

## 已完成

- 新增 `g04_revoke_interaction_test.rs`，锁定 live revoke capability、offline 零 HTTP、单 PersistAtomic 与 ack 前零业务 Frame。
- 新增 message-v4 G04 菜单 DOM/ARIA 测试；既有撤回危险确认与 recalled system row 回归仍通过。

## 真实验证

- `2026-07-22T12:01:25Z..12:01:39Z`：G04/G06 focused Jest，14s，2 suites/2 tests PASS。
- `2026-07-22T12:02:04Z..12:02:28Z`：相邻 UI 回归首次 24s，21 PASS/1 FAIL；失败为新增 mode button 复用 `aria-pressed` 干扰旧 G05 selector，已修为 radiogroup/aria-checked。
- `2026-07-22T12:05:32Z..12:05:59Z`：`cargo check -p helix-im --tests`，27s，PASS；仅证明当前共享改动和 focused tests 可编译，不建立行为 Green。

## 待完成

- shared `ActionDispatch::RevokeMessage`、timeline binding producer、module action preparation。
- `posts_update` 权威事实合批 PersistAtomic，PersistOk 后完整 timeline Frame；PersistErr 零成功 Frame。
- Rust focused test PASS、017 双账号/PG/SQLite/Frame/DOM/visual。
