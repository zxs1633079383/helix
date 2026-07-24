# G11 V2 切换 Spec

**目标**：请求定位 `postId` 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
目标窗口提交后执行 RevealMessage，目标不存在或失败时不移动当前滚动位置。

**输入**：`ActionBinding(locate)`；**Command**：`im_get_posts`；**Effect**：`http_read`、`storage_upsert`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
