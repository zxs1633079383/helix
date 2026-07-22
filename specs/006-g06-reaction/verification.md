# Verification Procedure: G06 Emoji Reaction

## Focused Admission Runs

- **V-001**: 运行本 Gate Rust action/replay/contract test；必须证明 ActionBinding、Tick/Effect、authority normalize、PersistOk barrier、完整 Frame 和重复/乱序幂等。
- **V-002**: 运行本 Gate Angular codec/component/DOM test；必须证明只回传 actionRef/input、whole-frame apply、成功/失败/forbidden/empty 状态和稳定 ARIA/data-*。
- **V-003**: 运行 Spec Kit R1-R3 与 speckit-analyze；Constitution 冲突或未覆盖 requirement 数量必须为 0。
- **V-004**: 仅在 017 最终 candidate 中运行双账号 Native、远端 PG、本地 SQLite、Frame/DOM 与 Index-Dual 双尺寸证据；本 Gate 禁止单独 Native build。

## Evidence Rules

- 原始输出写入本 feature 的 `evidence/`，用 `loop_engine.py record-evidence` 登记哈希、命令、起止时间和结果。
- focused PASS 只允许标记 `READY_FOR_FINAL_E2E`，不得标记 runtime Green。
- 所有密码、cookie 与 DSN 必须脱敏；数据库验证只读。
- 同一 validator/失败签名最多自动修复两次，第三次必须进入 NeedsHuman。

