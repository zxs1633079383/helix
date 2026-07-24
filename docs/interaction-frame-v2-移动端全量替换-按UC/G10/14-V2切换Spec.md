# G10 V2 切换 Spec

**目标**：attach 频道或上拉 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
首屏显示有界时间线；上拉追加旧消息且保持滚动锚点，Partial/Miss 显式回源。

**输入**：`HostObservation` / `ActionBinding(load_older)`；**Command**：`im_query_messages_by_channel` / `im_load_older_context`；**Effect**：`local_scan`、`http_read`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
