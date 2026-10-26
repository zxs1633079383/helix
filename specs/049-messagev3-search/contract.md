# MessageV3 Search Gate 最小合同

状态：`IMPLEMENTING / REAL_EVIDENCE_NOT_RUN`

本文是 Search 的独立合同，不修改 Phase 4 manifest 或其它 Gate 资产。Gate 沿用 Phase 4
预留编号：`MV3-G14a` 顶部全局搜索、`MV3-G14b` 会话内消息搜索、`MV3-G14c` 历史消息按会话
分组搜索。

## 真实链与职责

- 四个命令固定复用 Go 路由：`POST /api/cses/search/{do,user,channel,post}`。
- `im_search_do` 返回 Go 的全局三段结果；`im_search_user`、`im_search_channel` 和
  `im_search_post` 分别承担对应结果段的续页或展开。
- Helix 只做命令校验、snake_case → Go camelCase、HTTP correlation 和统一错误回灌；不维护
  全文索引、排序、权限、结果分组或本地搜索缓存。
- Go/ES 负责 actor/company session、可见性、固定排序、服务端 opaque cursor、筛选、分页和
  消息按会话聚合。客户端不得下发 `user_id`、`team_id`、`company_id` 或 `sort`。
- 成功/失败继续走 `im:read:result{req_id,body|error}`；不新增事件、ABI、driver 分支、业务
  reducer、本地缓存或 SQLite 表。
- `req_id` 只存在于 Helix correlation，绝不进入 Go request wire；cursor 由 Go 产生，Helix
  和 UI 只透传，不解析、不重写。

## Gate 命令分工

### `MV3-G14a`：全局搜索首屏与三段独立续页

1. 首屏调用 `im_search_do`（不带三段 cursor），Go 返回 `GlobalSearchResult` 的 users、
   channels、messageGroups 三个独立分页段。
2. users 段续页调用 `im_search_user`，仅携带该段 `nextCursor`。
3. channels 段续页调用 `im_search_channel`，仅携带该段 `nextCursor`。
4. messageGroups 段续页调用 `im_search_post`，沿同一搜索意图透传对应 cursor；展开单个群的
   更多命中时携带该群 `channel_id`，由 Go 返回该群消息页。

### `MV3-G14b`：会话内筛选搜索

`im_search_post` 必须携带非空 `channel_id`，并可携带 `keyword`、`sender_ids`、
`content_types`、`start_at`、`end_at` 过滤及 `cursor/page_size`。Go 返回该频道的
`SearchPage<SearchPostHit>`，不由 Helix 计算 `total`、`hasMore` 或排序。

### `MV3-G14c`：历史消息按会话分组

1. 首屏仍调用 `im_search_do`；本 Gate 只消费其中的 `messageGroups` 段，users/channels 段不
   计入本 Gate 的结果。
2. 某一 `MessageGroup` 的更多消息调用 `im_search_post`，携带该组 `channel_id`、原筛选意图
   和服务端返回的 opaque cursor；返回 `SearchPage<SearchPostHit>`。
3. `MessageGroup` 的分组、命中数和组内消息均以 Go response 为准，Angular/Helix 不得另行分组
   或重排。

## Go request wire（DTO 真源）

公共命令参数使用 snake_case；Go JSON 使用下表 camelCase 字段。`req_id` 是本地 transport
correlation，不在任何 Go body 中。

| 命令 | Go DTO body |
| --- | --- |
| `im_search_post` | `keyword?`、`channelId?`、`senderUserIds?`、`contentTypes?`、`startAt?`、`endAt?`、`cursor?`、`pageSize?` |
| `im_search_user` | `keyword`、`cursor?`、`pageSize?` |
| `im_search_channel` | `keyword`、`cursor?`、`pageSize?` |
| `im_search_do` | `keyword`、`userCursor?`、`channelCursor?`、`messageGroupCursor?`、`pageSize?` |

边界由 Go DTO/查询规范化共同约束：keyword 由 user/channel/do 要求非空；post 至少有
keyword、channelId、senderUserIds、contentTypes、startAt、endAt 之一；时间同时存在时
`startAt <= endAt`；cursor 长度为 1..=4096 字节；`pageSize` 为 1..=50；排序固定由 Go 决定。
Helix 还要求每条命令有 1..=128 字节的非空 `req_id`，并拒绝前端 authority/sort 字段。

## Go response wire（DTO 真源）

Go `data` payload 必须保持 DTO 字段，不得改成旧的 `posts.groups` 形状：

```json
{
  "users": { "items": [], "nextCursor": "", "hasMore": false },
  "channels": { "items": [], "nextCursor": "", "hasMore": false },
  "messageGroups": { "items": [], "nextCursor": "", "hasMore": false }
}
```

- `SearchPage<T>` 的权威字段为 `items`、`nextCursor`、`hasMore`。
- `GlobalSearchResult` 的权威字段为 `users`、`channels`、`messageGroups`，每段都是
  `SearchPage<T>`。
- `MessageGroup` 的权威字段为 `channel`、`posts`、`matchCount`；`posts` 是
  `SearchPostHit[]`，不是 `groups` 或 `items`。
- Go 的公共 response envelope 保持现有 `status/message/data`；Helix 对 `data` 之外的原始
  body 不做业务改写，成功回灌同一 `req_id`，失败只回灌结构化错误。

## 验收边界

1. deterministic：四条命令 URL/body、camelCase 字段、边界拒绝、`im_search_do` read flag 和
   三段 cursor 透传。
2. integration：`ImModule::handle` 每次命令产生一条对应 HTTP；PortReply 后恰好一条同
   `req_id` 的 `im:read:result`，Go DTO body 字节语义不被 Helix 改写。
3. runtime：PC、Flutter、Web 均经通用 command/event bridge；平台不得增加业务 reducer。
4. real/performance：真实 Go/ES、权限、筛选、分页稳定性与 p50/p95/p99 另行采集；本地单测、
   fixture 或 HTTP 200 不得记为真实 Search Gate PASS。
