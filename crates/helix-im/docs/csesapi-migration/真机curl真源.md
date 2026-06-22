# 真机 curl 真源 —— outbound 命令请求体/头的 ground truth

> 来源：用户 2026-06-22 从**现网 cses-client 实际抓的 curl**（api-im-pre.jinqidongli.com，可换 `localhost:8065`）。
> 用途：helix-im 各 outbound 命令（`crates/helix-im/src/commands.rs` + `handle_send_message`）的请求体/头**必须逐字对齐这里**，
> 否则 Go 收下但不处理/不推回声（实测 send 无回声的真因 = 体不对，**非 Pulsar**）。
> 这是比 `full-map/partials/{1,2}-*.md`（读 Go 源码反推）**更高优先级**的真源（真抓包 > 源码推断）。

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
**🔴 helix 现状不符（send 无回声真因）**：`handle_send_message` 只发 `{channel_id, temporary_id, message}`（snake_case + 缺字段）。真源要 `channelId`/`temporaryId`（camelCase）+ `viewers`/`type:"TEXT"`/`simpleMessage`/`userId`/`teamId`/`userSnapshot`/`mentions`/`props`/`topicId`/`revoke`/`id`。**字段名 + 必填集合都要改。**

---

## 2. posts/makeTopic — 消息转话题 → WS 推 **`channel_created` + `post_update`**

```
POST /api/cses/posts/makeTopic    （头带 Cses-Track-Id）
```
**body verbatim**：
```json
{"rootId":"e9wnenyb1brbpfxssfuyfhcw5h","teamId":"6111fb0a202d425d221c53db","postId":"1j5q6c87fbg55fr3bo3kni7c9o","displayName":"1","type":"T","users":[{"id":"6008f761453b4e60700f01e2","role":"MEMBER","teamId":"6111fb0a202d425d221c53db"},{"id":"676cc4ccfbbc501161d5cd65","role":"CREATOR","teamId":"6111fb0a202d425d221c53db"}],"picturetype":"USER","picture":{"userIds":["6008f761453b4e60700f01e2","676cc4ccfbbc501161d5cd65"]},"forceCreate":true}
```
**🔴 helix 现状**：`im_make_topic` 仅校验 `postId` 透传 raw body。真源需 `rootId`/`teamId`/`postId`/`displayName`/`type:"T"`/`users[]`(含 CREATOR/MEMBER role)/`picturetype`/`picture`/`forceCreate`。

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

## 对账小结（helix-im 待改清单）

| 命令 | endpoint | helix 现状 | 动作 |
|---|---|---|---|
| send | posts/create | 🔴 snake_case + 缺 ~10 必填字段 + 回声 action 是 `post` 非 `posted` | **改 handle_send_message body + 回声/对账 action** |
| makeTopic | posts/makeTopic | 🔴 仅校验 postId | 补 rootId/teamId/displayName/type/users/picture/forceCreate |
| create | channel/create | 🟡 raw 透传 | 给 repl 真源模板 |
| member change | channel/member/change | ❓ 缺（只有 member/leave）| 核实是否新增 im_channel_member_change(join/leave) |
| revoke | posts/revoke | ✅ 一致 | — |
| close | channel/close | ✅ 一致 | — |

> 通用头待补：`appVersion` / `osVersion` / `device=Desktop` / `deviceId` 形态核对（host-cli main.rs）。
