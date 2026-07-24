# G15a V2 切换 Spec

**目标**：提交话题或群聊创建表单 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
新群聊在首批四个 Surface 原子出现，成员与 viewer 隔离由权威事实决定。

**输入**：`ActionBinding(make_topic/create_channel)`；**Command**：`im_make_topic` / `im_create_channel`；**Effect**：`http_write`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.topic-list`、`message.channel-summary`、`message.left-dialog-list`、`message.member-roster`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
