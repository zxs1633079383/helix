# G09 V2 切换 Spec

**目标**：确认取消 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
取消成功后定时入口消失，重复取消幂等，失败时保留可重试状态。

**输入**：`ActionBinding(schedule_cancel)`；**Command**：`im_cancel_schedule`；**Effect**：`http_write`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.channel-summary`、`message.left-dialog-list`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
