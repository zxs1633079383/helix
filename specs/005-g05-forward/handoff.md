# G05 Handoff

## 状态

`IN_PROGRESS`。canonical forward assembly、per-target partial acceptance 和 Angular individual/merged/partial retry 已覆盖；opaque ActionBinding 与 terminal delivery Frame 仍由共享 runtime owner 收口。

## 已完成

- `forward.rs` 对 renderer 输入做 exact-key、上限、非空和去重校验，保持确定性顺序。
- 新增 G05 Rust focused test，覆盖 live binding 目标和 partial 不冒充 all-success/delivered。
- Forward dialog 新增明确的 individual/merged radiogroup、失败目标定向 retry 与逐目标状态。

## 真实验证

- `2026-07-22T12:01:49Z..12:01:56Z`：G05/G07 focused Jest，7s，2 suites/2 tests PASS。
- `2026-07-22T12:02:48Z..12:03:05Z`：G05 新旧 focused Jest，17s，2 suites/2 tests PASS。
- `2026-07-22T12:05:32Z..12:05:59Z`：`cargo check -p helix-im --tests`，27s，PASS；未单独运行 Native build。

## 待完成

- shared `ActionDispatch::ForwardMessages`、source ids registry ownership、ActionInvocation input codec。
- accepted target 的 WS post delivery authority、PersistOk 后完整 Frame 与 causation。
- Rust focused test PASS、017 双账号/双库/视觉。
