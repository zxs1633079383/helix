# G14 单条回执：Mobile V2 Spec 输入

| Gate | 输入/调用面 | Mobile UI 终态 |
|---|---|---|
| G14 | visible `HostObservation` / `ActionBinding(post_read)` / `im.posts.markRead` | 读者列表、消息读态和线程详情在权威提交后一致 |
| G14a | `ActionBinding(template_received)` / `im.posts.templateReceived` | 模板已收到仅在权威提交后出现，重复确认幂等，失败保留按钮 |

Surface 为 `message.participant-receipts`、`message.timeline`、`message.thread-detail`；完整 VM 为
`ParticipantReceiptsVm + TimelineWindowVm + ThreadDetailVm`。HostObservation 只描述可见事实，
不得直接伪造已读/已收到成功。

共享领域、HTTP/WS/PG/SQLite 与负向合同见
[PC/Helix G14](../../interaction-frame-v2-全量替换-按UC/G14/README.md)。
