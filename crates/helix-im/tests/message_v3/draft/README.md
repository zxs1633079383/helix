# MessageV3 Phase 1 Draft Gate 测试域

本目录覆盖 `MV3-G02e`。草稿以 `(accountId, channelId)` 隔离并采用 last-write-wins。
`im_save_draft` 与 `im_query_draft` 返回结构化 Command Result，不发布消息领域事件。

测试必须覆盖保存、查询、账号隔离、覆盖写、持久化失败保留旧值和重启读回。空 Event 数组不能
代替具体查询结果，Mock 也不能代替真实 Storage read-back。
