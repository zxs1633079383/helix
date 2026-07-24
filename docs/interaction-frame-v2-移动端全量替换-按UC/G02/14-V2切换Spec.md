# G02 V2 切换 Spec

**目标**：选择文档内容并提交 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
文档消息显示文档卡片、发送状态和失败后的输入恢复，不由 JS 解读 document props。

**输入**：`ActionBinding(message.create)`；**Command**：`im_send_message`；**Effect**：`upload`、`storage_upsert`、`http_write`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.composer`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
