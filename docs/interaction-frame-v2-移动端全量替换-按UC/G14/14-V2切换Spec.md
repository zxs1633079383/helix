# G14 V2 切换 Spec

**目标**：单条可见、打开回执或点击已收到 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
读者列表、当前消息读态和线程详情一致；HostObservation 不直接伪造已读成功。

**输入**：`ActionBinding(post_read/template_received)` / `HostObservation`；**Command**：`im_post_read` / `im_template_received`；**Effect**：`http_write`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.participant-receipts`、`message.timeline`、`message.thread-detail`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
