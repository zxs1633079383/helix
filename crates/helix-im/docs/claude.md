# 频道同步分页协议维护说明

本页记录当前 Helix `channel_sync` 实现的维护入口。它描述的是代码已经落地的协议，不代表
Host、Angular、Go 或双用户真实链路已经验收。

## 生产调用链

`increment_channel_end` 聚合频道/member/cursor 写集 → `IncrementBatchPersist` →
`PersistOk` → `im:channel-sync-ready`。ready 只发布
`channelSyncSessionId`、`generation`、`defaultPageSize=20`，不携带身份或频道数组；
Persist 失败不发布 ready。

ready 后 Host 通过 `im_query_channel_sync_page` 请求本地 `channel` Scan，随后 Helix 以同一
correlation 再做一次租户内 `channel_member` Scan；两次读取在 Helix 内合并，避免逐频道 N+1。
只有成员快照读回成功才发布最终页。业务请求只允许
`channel_sync_session_id`、`next_cursor`、`page_size`（`req_id` 仅为 Host transport 关联），
页长固定 20；opaque cursor 绑定 session/generation/offset。回包使用统一
`im:read:result` waiter，body 为最多 20 条 render-ready absolute channel 行（含
`lastEventSeq` 等 camelCase 游标字段），并带 `hasMore`、`nextCursor`、
`channelSyncSessionId`、`generation`。

频道页的 `members/adminUsers/boss/owner/memberCount` 来自 `channel_member` 行的角色分桶，
并按 `team_id` 与 RuntimeAuth company 过滤；缺失快照时保留已有非空字段，不用空数组覆盖旧投影。

在线 `channel_member_role_updated` 不再把 `{userIds, role}` delta 透传给 Angular。Helix 先读回
当前频道成员，确认目标成员已存在后以 `channel_id,user_id` 复合键原子 upsert 角色（撤销归一为
`MEMBER`），PersistOk 后再次读回完整 roster，再发布既有 `im:channel:member-updated` 包装的
绝对 `channel`。任何缺行、租户缺失、持久化失败或读回畸形都 fail-closed，旧 roster 保留。

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

`cargo test -p helix-im --lib` 覆盖固定页长、0/1/20/21 行、有界连续页、成员租户过滤与角色分桶、
cursor 绑定、Persist 成功/失败与 ready 一次性、complete scope/idempotency；
`cargo test -p helix-im --test p4_ws_events_test` 覆盖在线角色变更的读快照→原子写→读回 roster 链。

## Phase 3 频道治理命令维护

`im_channel_change_orient` 使用 `id/orient/expectedVersion`，允许空导向但限制 30 个字符；
`im_channel_change_permission` 使用非负 `expectedVersion` 的 partial patch，权限值仅允许
`BOSS/CREATOR/MANAGER/MEMBER`，Helix 不根据频道类型重算 `BOSS` 能力。成员变更与昵称命令
的 canonical 输入分别固定为五键和三键，空数组/空昵称仍可表达清空语义。

增量读取在没有 `req_id` 时保留内部 hydration 上下文；显式 `req_id` 时复用统一
`OutboundReadReply`/`im:read:result` 纯读回灌路径，不增加 Storage、WS 或第二结果事件。
`channelIncrement` 专用事件生产者尚未在本仓发现，不能据此宣称 G09d canonical 事件完成。
