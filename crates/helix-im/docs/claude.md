# 频道同步分页协议维护说明

本页记录当前 Helix `channel_sync` 实现的维护入口。它描述的是代码已经落地的协议，不代表
Host、Angular、Go 或双用户真实链路已经验收。

## 生产调用链

`increment_channel_end` 聚合频道/member/cursor 写集 → `IncrementBatchPersist` →
`PersistOk` → `im:channel-sync-ready`。ready 只发布
`channelSyncSessionId`、`generation`、`defaultPageSize=20`，不携带身份或频道数组；
Persist 失败不发布 ready。

ready 后 Host 通过 `im_query_channel_sync_page` 请求本地 `channel` Scan。业务请求只允许
`channel_sync_session_id`、`next_cursor`、`page_size`（`req_id` 仅为 Host transport 关联），
页长固定 20；opaque cursor 绑定 session/generation/offset。回包使用统一
`im:read:result` waiter，body 为最多 20 条 render-ready absolute channel 行，并带
`hasMore`、`nextCursor`、`channelSyncSessionId`、`generation`。

`im_complete_channel_sync` 只接受当前 account/company 的 session，使用统一
`im:read:result` 回传 `completed=true`；重复 complete 可重复得到同一成功 body，错误
session、旧 generation 和迟到 page 都 fail-closed 并回传 waiter error（有 `req_id` 时）。

## 维护边界

- 业务身份来自 `ImConfig` RuntimeAuth，不能从 caller payload 读取 account/company。
- page 事件不恢复 `im:channel:increment`、`im:channels:loaded`，也不创建
  `InteractionFrame`/snapshot/patch。
- 修改协议字段时同步 `src/channel_sync.rs`、`src/event/channel_sync.rs`、query/corr 路由和本页。
- 单元测试只证明确定性协议与本地投影；它们不能替代真实 Tauri/WS、双用户或 DOM 证据。

## 当前验证

`cargo test -p helix-im channel_sync --lib` 覆盖固定页长、0/1/20/21 行、有界连续页、
cursor 绑定、Persist 成功/失败与 ready 一次性、complete scope/idempotency。
