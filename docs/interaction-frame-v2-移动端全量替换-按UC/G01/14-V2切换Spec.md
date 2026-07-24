# G01 V2 切换 Spec

**目标**：输入文本并提交 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
同一消息行从 sending 收敛为 sent 或 failed，禁止 JS 再造第二条临时消息。

**输入**：`ActionBinding(message.create)`；**Command**：`im_send_message`；**Effect**：`storage_upsert`、`http_write`、`timer`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.composer`、`message.timeline`、`message.left-dialog-list`、`message.unread-summary`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
