# G04 V2 切换 Spec

**目标**：撤回操作或 type-3 同步事实 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
发送者撤回后时间线、线程和会话摘要同步显示撤回态，原操作按钮失效。

**输入**：`ActionBinding(revoke)` / `Inbound`；**Command**：`im_revoke`；**Effect**：`http_write`、`storage_upsert`、`projection`、`sync_request`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.timeline`、`message.thread-detail`、`message.left-dialog-list`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
