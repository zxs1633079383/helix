# 频道同步分页协议维护说明

本页记录当前 Helix `channel_sync` 实现的维护入口。它描述的是代码已经落地的协议，不代表
Host、Angular、Go 或双用户真实链路已经验收。

## 生产调用链

每个新的 global `increment_channel` 批次在 `increment_channel_end` 聚合频道/member/cursor 写集 →
`IncrementBatchPersist` → `PersistOk` → `im:channel-sync-ready`。同一批次的重复 end 会被忽略；
后续收到新 increment 会重新打开批次并再次走同一屏障。每个成功持久化的非空批次只发布一次 ready，
ready 只发布
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
频道页的群设置读取版本仍必须在本地读侧保留：Go 当前以 `channel.update_at` 同时作为
`settingVersion` 与 `permissionRevision`。因此 `query/render_ready/channel.rs` 对旧表行优先读取显式
`setting_version`/`permission_revision`，缺失时回退到持久化的 `updated_at`/`updateAt`。MessageV3
频道与成员写入不接收客户端版本条件；这些读取字段仍用于投影、乱序保护和后续独立竞态方案，
不得因此删除。

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
cursor 绑定、Persist 成功/失败与 ready 一次性、跨批次重新 ready、complete scope/idempotency；
`cargo test -p helix-im --test p4_ws_events_test` 覆盖在线角色变更的读快照→原子写→读回 roster 链。

## Phase 3 频道治理命令维护

`im_channel_change_orient` 使用 `id/orient`，允许空导向但限制 30 个字符；
`im_channel_change_permission` 使用无版本条件的 partial patch，权限值仅允许
`BOSS/CREATOR/MANAGER/MEMBER`，Helix 不根据频道类型重算 `BOSS` 能力。成员变更与昵称命令
的 canonical 输入分别固定为五键和三键，空数组/空昵称仍可表达清空语义。

增量读取在没有 `req_id` 时保留内部 hydration 上下文；显式 `req_id` 时复用统一
`OutboundReadReply`/`im:read:result` 纯读回灌路径，不增加 Storage、WS 或第二结果事件。
`channelIncrement` 专用事件生产者尚未在本仓发现，不能据此宣称 G09d canonical 事件完成。

## 2026-08-06 频道设置回包可诊断边界

- `port_reply/dispatch_projection.rs::handle_outbound_channel_settings_reply` 只在完整的 HTTP 2xx/base64
  envelope、body `status=SUCCESS`、object `data`、`channelId`、`purpose`、`header` 和非负整数
  `settingVersion` 均通过后才持久化并发布 `channelUpdate`；任何缺失或类型错误都 fail-closed。
- `response_data_with_reason` 与 `decode_settings_projection` 返回有限的脱敏 reason，并在
  `cses.im.permission` target 记录 `stage/channelId/correlation/operationId/hasBody/hasProjection/hasCapability/
  settingVersion/reason`；默认日志级别不改变业务状态，也不打印正文、成员或认证信息。
- 持久化成功后才允许事件发布；persist 或 emit 失败不会伪造成功投影。当前 `dispatch_projection` 单测只证明
  HTTP envelope/authority decode 的确定性边界，未证明真实 Go/WS、Tauri bus、Angular 消费或双用户链路。

## Phase 2 时间线单活动窗口修复

`TimelineState` 以 `active_window_by_channel` 维护每个频道唯一活动窗口。latest/locate
`replace` 先校验 rows，再淘汰同频道旧 slots、attachments 和 navigation token；旧异步 page
回包不能通过 `entry(...).or_insert` 复活已淘汰窗口。`paging_context_for_anchor` 与
`unique_attached_window_for_channel` 只查询活动窗口，缺失或过期 anchor 以结构化 read error
结束，不再抛出裸 `im_load_older_context requires...` engine-step parse error。

Timeline HTTP、Persist、SQLite read-back 及最终 Emit 均在继续前重检 active window；窗口已替换时
只记录脱敏 stale 诊断并失败安全。transport reconnect 仍按既有规则保留 viewer-local attachment，
身份/generation reset 由 `ImState::reset_recent_query_coverage` 清除全部时间线状态。

`timeline_state` 单测覆盖同频道换窗、旧 merge、当前 older、同 token 导航、跨频道隔离和 reset；
真实群聊切换、上拉与 SQLite read-back 仍需用户在人工验收阶段确认。

## 纯离线 sync/notify 字段消失诊断

诊断默认关闭，运行时由 Host 环境变量注入 `ImConfig.offline_sync_diagnostics`，不改变同步
业务语义。三项开关分别是：`HELIX_OFFLINE_SYNC_TRACE`（sync/notify、StorageOp、提交后
readback 的脱敏字段 presence/length/hash）、`HELIX_OFFLINE_SYNC_WAL`（SQLite WAL/SHM
只读元数据）和 `HELIX_OFFLINE_SYNC_TARGETS`（目标过滤，支持 `qem,bzj,sjx` 或精确
`temporary_id`）。目标别名映射固定在 `sync/observability.rs`；日志不输出正文、cookie、token
或用户快照。

Host 侧在 SQLite atomic write 的 `before_write`、`after_commit`、`after_readback` 三个点
采集 WAL header、frame 数、commit frame、page number 尾部和 DB/WAL/SHM 文件大小/mtime；
readback 同时记录 `msg_id`、`temporary_id`、`channel_id`、`event_seq`、`operation_id` 及目标
字段摘要；即使后续补丁只改正文，也会固定回读 `expedite_map`、`reply_messages`、
`reply_count`、`reply_id`、`reply_root_id`、`reply_first_level_id`、`replied_message`。
该采样只读文件字节，不执行 checkpoint、VACUUM、REINDEX 或 journal_mode 变更。

当前验证仅覆盖 Rust 编译和确定性单测；真实登录、离线同步、SQLite/PG/WAL 读回分析仍须在
用户明确报告“离线同步已完成”后进行，不能把本地测试当作真实链路结论。
