# 历史真机 curl wire 参考 —— outbound 命令请求体/头

> 来源：用户 2026-06-22 从**现网 cses-client 实际抓的 curl**（api-im-pre.jinqidongli.com，可换 `localhost:8065`）。
> 这里冻结的是当日 outbound wire 的 body/headers，不是当前 Helix 实现状态快照；当前字段是否已对齐，
> 必须以 `crates/helix-im/src/` 和对应测试复核。它的 wire 事实优先于
> `full-map/partials/{1,2}-*.md`（读 Go 源码反推），但不能用历史抓包替代当前代码证据。

---

## 0. 通用请求头（所有 outbound 都带）

| header | 示例值 | 说明 |
|---|---|---|
| `Content-Type` | `application/json` | |
| `cookieId` | `676cc4ccfbbc501161d5cd65` | = userId，身份头（host 注入全局头）|
| `companyId` | `6111fb0a202d425d221c53db` | 公司，缺则 paas-grpc 业务失败 |
| `connectionId` | `bpijf5yx7bfgdmexzyrw8hg7my` | im 收 hello 盖章（**握手后才有**，故首命令须等握手）|
| `deviceId` | `c56d5310-539c-49b7-8033-b1624388294b` | ⚠️ **带连字符 UUID**（与 host-cli 写死的 `ED380414…`32hex 无连字符**形态不同**，待核现网真用哪种）|
| `device` | `Desktop` | ⚠️ **不是 `IOS`**（host-cli 默认 IOS，桌面端真值是 Desktop）|
| `appVersion` | `1.2.56` | ⚠️ host-cli 当前**不发**此头 |
| `osVersion` | `macos-14.5.0` | host-cli 当前不发 |
| `Cses-Track-Id` | `6a38b67289e244c1441cc1a1` | makeTopic/create 等带，透传到 WS envelope.tracing.csesTrackId |
| `uber-trace-id` / `traceparent` | jaeger/otel 链路头 | 可选（链路追踪）|

> **host-cli 待补头**：`appVersion` / `osVersion`；`device` 默认改 `Desktop`；`deviceId` 形态待核（连字符 UUID vs 32hex）。

---

## 1. posts/create — 发普通消息 → WS 推 **`post`**（不是 `posted`！）

> ⚠️ **WS 回声 action = `post`**（用户明确：「普通发消息是 post 推送，不是 posted」）。helix-im registry 的 `posted` handler 不是这条。

```
POST /api/cses/posts/create
```
**body verbatim**（注意全 camelCase + 大量必填字段）：
```json
{"viewers":["all"],"message":"1","mentions":[],"temporaryId":"6a38b61dd2ce3c849a1cc1a0","type":"TEXT","simpleMessage":"1","channelId":"e9wnenyb1brbpfxssfuyfhcw5h","userId":"676cc4ccfbbc501161d5cd65","teamId":"6111fb0a202d425d221c53db","userSnapshot":{"orgName":"后端开发","deptName":"技术部","userName":"张立超","userId":"676cc4ccfbbc501161d5cd65","teamId":"6111fb0a202d425d221c53db"},"id":"","props":{},"topicId":"","revoke":false}
```
**✅ 当前 Helix builder 已对齐主要字段**：`outbound/send_build.rs` 的 `handle_send_message` 现在生成
`channelId`/`temporaryId`（camelCase）以及 `viewers`、`type`、`simpleMessage`、`userId`、`teamId`、
`userSnapshot`、`mentions`、`props`、`topicId`、`revoke`、`id` 和 `createAt`；
`tests/p1_send_build_test.rs` 覆盖这些字段。上面的 curl 仍是外部 wire 参考，不能再作为“只发 snake_case”
的当前 Helix 结论。

---

## 2. posts/makeTopic — 消息转话题 → WS 推 **`channel_created` + `post_update`**

```
POST /api/cses/posts/makeTopic    （头带 Cses-Track-Id）
```
**body verbatim**：
```json
{"rootId":"e9wnenyb1brbpfxssfuyfhcw5h","teamId":"6111fb0a202d425d221c53db","postId":"1j5q6c87fbg55fr3bo3kni7c9o","displayName":"1","type":"T","users":[{"id":"6008f761453b4e60700f01e2","role":"MEMBER","teamId":"6111fb0a202d425d221c53db"},{"id":"676cc4ccfbbc501161d5cd65","role":"CREATOR","teamId":"6111fb0a202d425d221c53db"}],"picturetype":"USER","picture":{"userIds":["6008f761453b4e60700f01e2","676cc4ccfbbc501161d5cd65"]},"forceCreate":true}
```
**🟡 当前 Helix 实现仍是部分映射**：`MakeTopicCommand` 已不再只校验 `postId`，而是要求
`root_id`、`user_ids`、`req_id`（可选 `display_name`），输出 `rootId`、`userIds` 和可选 `displayName`。
抓包中的 `teamId`、`type`、`users`、`picturetype`、`picture`、`forceCreate` 尚未由该 builder 表达；
这属于代码待核/待补缺口，不能把历史 curl 写成当前已完成。

---

## 3. posts/revoke — 撤回消息 → WS 推 **`post_update`**

```
POST /api/cses/posts/revoke
```
**body verbatim**：
```json
{"postId":"hwi11t1xi7y49ruxfx9sfs51iy"}
```
**✅ helix 现状符合**：`im_revoke` 发 `{"postId": ...}` 一致。

---

## 4. channel/create — 创建群聊 → WS 推 **`channel_created` + `channel_member_update` + `post`**

```
POST /api/cses/channel/create    （头带 Cses-Track-Id）
```
**body verbatim**：
```json
{"teamId":"6111fb0a202d425d221c53db","displayName":"123123123","orient":"","type":"P","users":[{"id":"6a27eaf08e095b5eb67f76a7","teamId":"6111fb0a202d425d221c53db","role":"MEMBER"},{"id":"676cc4ccfbbc501161d5cd65","teamId":"6111fb0a202d425d221c53db","role":"CREATOR"},{"id":"64b668ba5965132667458686","teamId":"6111fb0a202d425d221c53db","role":"MEMBER"}],"picturetype":"USER","picture":{"userIds":["6a27eaf08e095b5eb67f76a7","676cc4ccfbbc501161d5cd65","64b668ba5965132667458686"]},"forceCreate":true}
```
**🟡 helix 现状**：`im_create_channel` 透传 raw body（repl 收 rawJSON）。真源模板：`teamId`/`displayName`/`orient`/`type:"P"`/`users[]`(自己 role=CREATOR + 他人 MEMBER)/`picturetype`/`picture.userIds`/`forceCreate`。

---

## 5. channel/member/change — 加/退成员 → WS 推 **`channel_member_update`**

> ⚠️ **新 endpoint（之前漏了）**：成员增减走 `channel/member/change`，**不是** `channel/member/leave`。
> `joinUsers` 非空 = 加人；`leaveUsers` 非空 = 退人；**两者可同时非 nil**。

```
POST /api/cses/channel/member/change
```
**body verbatim（join 示例）**：
```json
{"channelId":"f65kcp58qfb7fm3w8y433wkumy","joinUsers":[{"id":"6800c44dbd6a2f6be5534c95","teamId":"6111fb0a202d425d221c53db","role":"MEMBER"}]}
```
> leave 同结构，用 `leaveUsers:[...]`。
> 注：`channel/member/leave`（full-map #33）是「自己退群」，与本 admin 增减成员是**两条**——待核 helix `im_channel_leave` 该映射哪条（自己退 vs change.leaveUsers）。

---

## 6. channel/close — 关闭频道 → WS 推 **`channel_close`**

```
POST /api/cses/channel/close
```
**body verbatim**：
```json
{"channelId":"f65kcp58qfb7fm3w8y433wkumy"}
```
**✅ helix 现状符合**：`im_channel_close` 发 `{"channelId": ...}` 一致。

---

## 对账小结（抓包与当前 Helix 实现）

| 命令 | endpoint | helix 现状 | 动作 |
|---|---|---|---|
| send | posts/create | ✅ 当前 builder 已生成 camelCase 与主要必填字段；回声 action 是 `post` 非 `posted` | 继续以源码/测试复核 wire 兼容 |
| makeTopic | posts/makeTopic | 🟡 builder 已生成 rootId/userIds/displayName，但仍少抓包中的 teamId/type/users/picture/forceCreate | 代码 owner 继续核对真实 wire |
| create | channel/create | 🟡 raw 透传 | 给 repl 真源模板 |
| member change | channel/member/change | ❓ 缺（只有 member/leave）| 核实是否新增 im_channel_member_change(join/leave) |
| revoke | posts/revoke | ✅ 一致 | — |
| close | channel/close | ✅ 一致 | — |
| createPosts | posts/createPosts | ✅ 一致（camelCase `{posts,channelIds}`）| — |

> 通用头待补：`appVersion` / `osVersion` / `device=Desktop` / `deviceId` 形态核对（host-cli main.rs）。

---

## 附录 A：createPosts（批量转发）—— **非真抓包，三方证据链定论**（F2 / gap-2，2026-06-23）

> 用户记忆中「曾有含 createPosts 的 curl 集合」——**本文件上方 6 curl 实抓集合不含 createPosts**（已逐条确认）。
> 全仓（cses-client / helix / mattermost）穷举搜索 `createPosts` + `curl `：**无任何 live-capture curl body** 留存。
> 命中的是测试计划/回归框架文档，只列端点路径不含 body；它们很可能就是用户记忆的
> 「集合」，但属于**用例清单**而非抓包 curl。
>
> 虽无真抓包，端点路径 + body casing 由**三方源码证据闭环确证**（强度等价抓包，因前端 HTTP body 逐字透传）：
>
> | 证据腿 | 出处 | 结论 |
> |---|---|---|
> | ① 前端调用（= 上线真实 wire） | `message.service.ts:803` `post$('/posts/createPosts', params)` 逐字透传 params；两 caller 都构造 camelCase：`chat-content-base.component.ts:967 {posts,channelIds}` + `messageContainerShared.service.ts:245 {posts:[post],channelIds:[channelId]}` | 端点 `/posts/createPosts`（**双段均复数**）；body **camelCase** `{posts,channelIds}` |
> | ② 后端路由 + handler | `csesapi/api.go:169` `BaseRoutes.Posts = APIRoot.PathPrefix("/posts").Subrouter()` + `posts.go:19` `Posts.Handle("/createPosts", createPosts)` → 全路径 `/posts/createPosts`；`posts.go:221 createPosts` 解构体 `{Posts *entity.Posts; ChannelIds []string}` **无 json tag** → Go case-insensitive，camelCase 实发可解 | handler 名 **createPosts（复数）**；端点 `/posts/createPosts`；PascalCase 仅 Go 字段名，从未上线 |
> | ③ 前端用例 / 接口全景 / 对接契约 | `00-接口全景.md:18` `sendRelayMessages POST /posts/createPosts`；`6--client-usecases.md:92` body `{channelIds,posts}`；`CSES_CLIENT_对接All-case实践.md:89` | 三处独立佐证同端点 |
>
> **定论**：端点全路径 `/posts/createPosts`（**post 后缀 s · createPosts 后缀 s · 双段复数**）；wire body **camelCase** `{"posts":[<Post>...],"channelIds":[<id>...]}`。
>
> **helix 现状 ✅ 一致**：`outbound/posts_relay.rs::CreatePostsCommand` build → endpoint `"posts/createPosts"` + camelCase `{"posts","channelIds"}`，`im_create_posts`（registry 88）。与三方证据零偏差，**无需改码**；`cp_create_posts_test.rs` 7 用例断言此 casing（含 Go 解构体 case-insensitive 折叠比对、PascalCase 不泄漏）。
>
> 历史 D3 挂起项（02/04/05 文档「createPosts PascalCase 待真抓包」）**就此收口**：PascalCase 是误读（源于 Go 无 tag 字段名 = PascalCase），真实上线 wire = 前端构造的 camelCase。
