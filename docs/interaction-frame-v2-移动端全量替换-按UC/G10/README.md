# G10 时间线窗口：Mobile V2 Spec 输入

| Gate | 输入/调用面 | Mobile UI 终态 |
|---|---|---|
| G10 | attach / `HostObservation` / `im.posts.queryMessagesByChannel` | 首屏显示有界窗口；Partial/Miss 经 Helix 回源 |
| G10a | `ActionBinding(load_older)` / `im.posts.loadOlderContext` | 围绕锚点补齐 before context，整窗提交后保持滚动锚点 |

唯一 Surface/VM 为 `message.timeline` / `TimelineWindowVm`。滚动像素与锚点执行属于 Venus，
窗口选择、顺序、target、page 与失败时保留旧窗口属于 Helix；JS 不合并 SQLite rows。

共享领域、HTTP/WS/PG/SQLite 与负向合同见
[PC/Helix G10](../../interaction-frame-v2-全量替换-按UC/G10/README.md)。
