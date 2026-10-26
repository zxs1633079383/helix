# G04 撤回：Mobile V2 Spec 输入

| Gate | 输入/调用面 | Mobile UI 终态 |
|---|---|---|
| G04 actor | `ActionBinding(revoke)` -> `im.posts.revoke` / `im_revoke` | 时间线、线程和会话摘要显示权威撤回态，原操作按钮失效 |
| G04a observer | type-3 `Inbound`，不发主动撤回 HTTP | 同一撤回事实与 cursor 提交后更新三个 Surface |

Surface 为 `message.timeline`、`message.thread-detail`、`message.left-dialog-list`；完整 VM 为
`TimelineWindowVm + ThreadDetailVm + DialogListVm`。Mobile 不得从 raw type-3 事件直接改行，
也不得让 observer 复用 actor action。

共享领域、HTTP/WS/PG/SQLite 与负向合同见
[PC/Helix G04](../../interaction-frame-v2-全量替换-按UC/G04/README.md)。
