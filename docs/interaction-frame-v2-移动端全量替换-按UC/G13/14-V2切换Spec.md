# G13 V2 切换 Spec

**目标**：频道可见或点击已读 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
会话未读数、频道已读边界和时间线读态同帧变化，重复可见观察幂等。

**输入**：`ActionBinding(channel_read)` / `HostObservation`；**Command**：`im_channels_view`；**Effect**：`http_write`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.left-dialog-list`、`message.unread-summary`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
