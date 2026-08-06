# MessageV3 Search Gate 最小合同

状态：`IMPLEMENTING / REAL_EVIDENCE_NOT_RUN`

本文是 Search 的独立合同，不修改 Spec047 已冻结的 30 Gate manifest。Gate 沿用 Phase4
handoff 预留编号：`MV3-G14a` 消息搜索、`MV3-G14b` 用户/频道搜索、`MV3-G14c`
全局三段搜索。

## 分层边界

- 四个命令复用现有 `/api/cses/search/{post,user,channel,do}` 路由。
- Helix 负责公开参数校验、snake_case 到 camelCase、HTTP correlation 和统一错误回灌。
- Go/ES 负责身份与公司范围、可见性、固定时间倒序、游标分页、全局消息按群聚合。
- 成功和失败继续使用 `im:read:result{req_id,body|error}`；不新增事件、ABI、driver
  分支、业务 reducer、本地缓存或 SQLite 表。
- `req_id` 仅存在于 Helix correlation，不进入 Go wire body；cursor 是服务端 opaque 值，
  Helix 和 UI 均不得解析或重写。

## 公共命令

公共参数使用 snake_case，Go wire 使用 camelCase。四个命令都要求 1..=128 字节的非空
`req_id`，并拒绝客户端 `user_id`、`team_id`、`company_id`、`sort`。cursor 为
1..=4096 字节，`page_size` 为 1..=50；排序固定由服务端决定。

### `MV3-G14a / im_search_post`

```json
{
  "req_id": "search-post-1",
  "scope": "channel",
  "keyword": "可选",
  "channel_id": "channel scope 必填",
  "sender_ids": ["u1"],
  "content_types": ["FILE"],
  "start_at": 0,
  "end_at": 0,
  "cursor": "opaque",
  "page_size": 20
}
```

`scope` 只允许 `channel|global`。keyword 可省略或为空，但此时至少提供
`sender_ids/content_types/start_at/end_at` 之一；时间同时存在时 `start_at <= end_at`。
channel scope 必须携带非空 `channel_id`。

### `MV3-G14b / im_search_user` 与 `im_search_channel`

```json
{
  "req_id": "search-directory-1",
  "keyword": "必填非空",
  "cursor": "opaque",
  "page_size": 20
}
```

用户搜索范围由 Go session 的公司身份决定，包含同公司但未建立好友或私聊的用户；频道搜索
只返回当前用户可见的群聊和私聊。

### `MV3-G14c / im_search_do`

请求与目录搜索使用相同的 keyword/cursor/page_size 合同。Go 返回权威三段 JSON：

```json
{
  "users": { "items": [], "nextCursor": null, "hasMore": false },
  "channels": { "items": [], "nextCursor": null, "hasMore": false },
  "posts": { "groups": [], "nextCursor": null, "hasMore": false }
}
```

`posts.groups` 必须由 Go 按频道聚合；Helix 只原样回灌。查看某一群的更多消息继续调用
`im_search_post(scope=channel, channel_id=..., cursor=...)`。

## 验收边界

1. deterministic：四条命令 URL/body、边界拒绝、`im_search_do` read flag。
2. integration：`ImModule::handle` 产生一条 HTTP，PortReply 后恰好一条同 req_id
   `im:read:result`，三段 body 字节语义不被 Helix 改写。
3. runtime：PC、Flutter、Web 都经通用 command/event bridge；平台不得增加业务 reducer。
4. real/performance：真实 Go/ES、权限、分页稳定性与 p50/p95/p99 另行采集；本地单测或
   HTTP 200 不得记为真实 Search Gate PASS。
