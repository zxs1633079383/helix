# G06 Handoff

## 状态

`IN_PROGRESS`。Go wire 和 Angular authority states 已覆盖；live opaque binding、权威绝对 reaction group 持久化及 terminal Frame 待共享 runtime 收口。

## 已完成

- canonical wire 固定为 `{postId,emoji}`，`req_id` 仅用于 Helix correlation，`desiredSelected` 与 renderer authority 字段 fail closed。
- Quick reply picker 新增 ready/failed/submitting/forbidden 语义状态，提交中和无权限时拒绝重复 selection。
- 新增 G06 Rust/Angular focused tests。

## 真实验证

- `2026-07-22T12:01:25Z..12:01:39Z`：G04/G06 focused Jest，14s，2 suites/2 tests PASS。
- `2026-07-22T12:05:32Z..12:05:59Z`：`cargo check -p helix-im --tests`，27s，PASS；未单独运行 Native build。

## 待完成

- shared `ActionDispatch::ToggleReaction` 与 timeline `react` binding live mapping。
- `post_update` canonical groups 经 PersistOk 后刷新完整绝对计数 Frame，禁止端侧 count delta。
- Rust focused test PASS、017 双账号/双库/视觉。
