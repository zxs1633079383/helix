# G12 V2 切换 Spec

**目标**：打开线程、分支或提交回复 只通过 InteractionFrameV2 驱动 Venus QuickJS UI，最终效果为：
线程 root/replies 图完整出现；回复发送终态与主时间线一致，不由 JS 拼 reply tree。

**输入**：`ActionBinding(open_reply_tree/reply)`；**Command**：`im_get_replies` / `im_get_reply_branch`；**Effect**：`http_read`、`projection`；
**Persist**：`PersistAtomic -> PersistOk`；**Surface**：`message.thread-detail`、`message.timeline`。

**完成条件**：producer+consumer V1 同时切断，首次纯 V2 和删除后二次证据均闭合；实际页面
`SOURCE_MISSING` 必须在实施前解除。
