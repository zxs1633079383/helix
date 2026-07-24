# G08 V2 切换 Spec

**目标**：选择时间并提交 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
定时消息提交中、已创建和失败状态可恢复，observer 不显示 owner 草稿。

**输入**：`ActionBinding(schedule_create)`；**Command**：`im_create_schedule`；**Effect**：`http_write`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.channel-summary`、`message.left-dialog-list`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
