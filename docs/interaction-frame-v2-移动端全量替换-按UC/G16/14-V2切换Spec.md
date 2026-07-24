# G16 V2 切换 Spec

**目标**：关闭、退出或保存设置 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
关闭后输入区禁用、频道从列表移除或标记关闭，其他频道保持可用。

**输入**：`ActionBinding(close/leave/settings)`；**Command**：`im_channel_close` / `im_channel_leave` / `im_channel_change_info`；**Effect**：`http_write`、`storage_upsert`、`projection`、`transport_broadcast`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.channel-summary`、`message.left-dialog-list`、`message.composer`、`message.member-roster`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
