# G15 话题/群聊创建：Mobile V2 Spec 输入

| Gate | 输入/调用面 | Mobile UI 终态 |
|---|---|---|
| G15 | `ActionBinding(make_topic)` / `im.channels.makeTopic` | 新话题在 topic list、频道摘要、会话列表和成员表原子出现 |
| G15a | `ActionBinding(create_channel)` / `im.channels.create` | 新群聊首批四 Surface 原子出现，成员与 viewer 隔离由权威事实决定 |

Surface 为 `message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、
`message.member-roster`；完整 VM 为
`TopicListVm + ChannelSummaryVm + DialogListVm + MemberRosterVm`。表单与 overlay 属于 Venus，
频道/成员事实、权限和跨 Surface 原子性属于 Helix。

共享领域、HTTP/WS/PG/SQLite 与负向合同见
[PC/Helix G15](../../interaction-frame-v2-全量替换-按UC/G15/README.md)。
