# GS04 V2 切换 Spec

**目标**：选择频道或未知 `channelId` 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
未知频道加载成功后频道头、输入区和时间线同时可用；失败时保留原频道。

**输入**：`HostObservation`；**Command**：`im_channel_load_increment_by_channel_id`；**Effect**：`http_read`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.channel-summary`、`message.composer`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
