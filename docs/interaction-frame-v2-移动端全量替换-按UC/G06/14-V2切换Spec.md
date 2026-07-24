# G06 V2 切换 Spec

**目标**：点击 emoji 或快捷回复 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
emoji 聚合数量、当前 actor 选中态和线程详情一致；拒绝时恢复旧聚合。

**输入**：`ActionBinding(react)`；**Command**：`im_quick_reply`；**Effect**：`http_write`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.timeline`、`message.thread-detail`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
