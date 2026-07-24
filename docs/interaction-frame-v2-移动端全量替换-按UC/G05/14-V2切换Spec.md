# G05 V2 切换 Spec

**目标**：选择目标并提交转发 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
每个目标频道独立呈现转发终态，源频道与失败目标不被假成功覆盖。

**输入**：`ActionBinding(forward)`；**Command**：`im_relay_messages`；**Effect**：`storage_read`、`http_write`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.timeline`、`message.left-dialog-list`、`message.composer`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
