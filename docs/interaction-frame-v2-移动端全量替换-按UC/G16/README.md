# G16 频道治理：Mobile V2 Spec 输入

| Gate | 输入/调用面 | Mobile UI 终态 |
|---|---|---|
| G16 | `ActionBinding(close)` / `im.channels.close` | 输入区禁用，频道从列表移除或标记关闭，其他频道不受影响 |
| G16b | `ActionBinding(leave)` / `im.channels.leave` | 成员表、频道可见性和输入权限一致；observer 看到成员变化 |
| G16c | `ActionBinding(settings)` / `im.channels.changeInfo` | 频道头、会话行和成员权限按 `expectedVersion` 原子更新 |

Surface 为 `message.channel-summary`、`message.left-dialog-list`、`message.composer`、
`message.member-roster`；完整 VM 为
`ChannelSummaryVm + DialogListVm + ComposerVm + MemberRosterVm`。确认框和表单属于 Venus；
terminal tombstone、权限和 expectedVersion 冲突属于 Helix/authority。

共享领域、HTTP/WS/PG/SQLite 与负向合同见
[PC/Helix G16](../../interaction-frame-v2-全量替换-按UC/G16/README.md)。Go 当前
`3f43d28` 已修正关闭会话已读回执假成功，正式 Spec 不得复用旧假成功口径。
