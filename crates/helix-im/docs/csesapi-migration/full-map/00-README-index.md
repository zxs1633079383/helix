# csesapi → cses-im-server 全量迁移图谱（full-map）索引

> 生成时间：2026-06-16 · 分支 `feat/im-sync-v2-backend`
> 目的：把「mattermost/server/channels/csesapi 这套 RESTful 后端 + WS 推送」全量映射到迁移目标后端 `cses-im-server`，并以**客户端真实用例**为消费视角，给出**执行级缺口矩阵**，回答「cses-im-server 现在能不能替代 mattermost」。
>
> 三库基准（活体源码 2026-06-16，GitNexus 索引偏旧，字段以源码为准）：
> - 迁移源后端：`mattermost/server/channels/csesapi/`（base prefix `/api/cses`）+ `app/cses_*.go`
> - 迁移目标后端：`cses-im-server/internal/{api,app,ws,store,entity,model}/`
> - 客户端：`cses-client/src/pages/message-v3/`（TS）+ `src-tauri/src/features/im/`（Rust）

---

## 0. 三份产出物结构

本图谱由三类 partial 文档构成（全部落盘在 `partials/`），本 README 做索引 + 导读，缺口矩阵汇总见 `30-capability-gap-matrix.md`。

| 类别 | 视角 | 回答什么 |
|------|------|----------|
| **A. backend-\*** | 迁移源 csesapi 后端 HTTP/WS 全清单 | 「源端有哪些接口/事件，wire 契约字段是什么」 |
| **B. client-\*** | 客户端真实用例/UI/Rust IPC | 「客户端实际怎么调、监听什么、字段契约现状」 |
| **C. gap-\*** | 源 vs 目标三态对照 | 「cses-im-server 复刻了没有，差在哪」 |

---

## 1. A. 后端源端全清单（backend-\*）

| 文件 | 域 | 计数 | 一句话导读 |
|------|----|------|-----------|
| [partials/1--backend-http-posts.md](partials/1--backend-http-posts.md) | posts | 35 HTTP | posts/post/approval/bookmark 四子路由，全 POST，mutation 走 channel_event co-tx AppendEvent；命名陷阱 3 处（snake_case `channel_id`、`timeStamp` 大写 S、`postIds` 单数但实为单 id）；`createMock` 压测旁路有意不迁。 |
| [partials/2--backend-http-channel-sync.md](partials/2--backend-http-channel-sync.md) | channel | 34 HTTP | channel.go 33 + sync.go 1，全 POST + `dto.CommonRes` 信封（status 为字符串 SUCCESS/FAILURE）；sync wire 硬契约 `fromSeq`/`eventSeq`（非 seq），too_long>5000 / events≤500 / cursor≤500；casing quirk 3 处（`ChannelId` PascalCase / `direct_key` snake_case / change/notify payload key 是 `id`）。 |
| [partials/3--backend-http-user-misc.md](partials/3--backend-http-user-misc.md) | user-misc（用户/团队/分组/模块/通知/搜索/健康/个人守卫） | 15 HTTP | 本域无自有 channel_event 驱动事件，WS 全来自下游复用；3 端点不走信封（createUser 裸 201 / health 裸 {status} / getUserStatusesByIds 空数组裸 null）；8 处已知待修 bug；createUser 深耦合原生注册全链是最高迁移风险。 |
| [partials/4--backend-http-bot-agent.md](partials/4--backend-http-bot-agent.md) | bot-agent（Bot/Agent/Webhook/CrossRepoDemo） | 40 HTTP | bot/bot_manage/bot_agent/agent/webhook_dispatcher/cross_repo_demo 六文件；bot-agent 子路由用 `Bearer mp_<token>` BotAgentTokenAuth；本域无专属 WS 事件常量，全复用标准 `post`；#38-40 cross_repo_demo 是 GitNexus trace 评分合成 handler，有意排除。 |
| [partials/5--backend-ws-events.md](partials/5--backend-ws-events.md) | WS 推送事件 | 19 事件 | **校正了任务前提两处错误**：(1) WS 事件常量在 `server/public/model/websocket_message.go:21-144`，**不在** `cons/fields.go`；(2) csesapi handler 不直接 Publish，经 app 层调 `a.Publish`/`a.PublishSlice`。移植铁律：`event_seq` 注入 data 顶层（pts gate 直读）、`update_channel`/`increment_channel` 的 `lastPost` 必须 JSON string 非 map、19 个 cses 事件全部无 BroadcastHook。 |

**后端 HTTP 接口总数**：35 + 34 + 15 + 40 = **124**（含 3 个 cross-repo demo + 1 个 createMock 有意不迁）。

---

## 2. B. 客户端真实用例（client-\*）

| 文件 | 视角 | 计数 | 一句话导读 |
|------|------|------|-----------|
| [partials/6--client-usecases.md](partials/6--client-usecases.md) | message-v3 service 层 HTTP+WS 用户用例 | 42 用例 | 双 HTTP 实例：`imHttp`(IM_HTTP) 自动拼 `/api/cses` 前缀 + ImUnwrapStrategy 解包；`api`(AppHttpService) 仅搜索/投票/平均分用。WS 四段式：服务端 action → Rust `router.rs::route` → `emit_im("im:*")` → Angular `getXxx$` → `dispatch(MessageEventDispatch.*)`，含全映射表。 |
| [partials/7--client-ui-rendering.md](partials/7--client-ui-rendering.md) | message-v3 组件/UI 渲染依赖 | 10 用例集 | readBits 字符串位图按 index 对应 visibleMembers，sender 位服务端必须预置 `'1'`；`unReadPostId` wire 名大写 R；Rust Message 表 `team_id` 已 zombie；`messageType.SYSTEM` 值是 `'SYSTEN'`（拼写不一致，迁移不可纠正）；self-message 本地 SQLite id 列空串只有 temporaryId；离线增量 cursor 字段 `lastEventSeq`。末附「迁移服务端字段契约清单 10 项」。 |
| [partials/8--client-tauri-im.md](partials/8--client-tauri-im.md) | src-tauri Rust IM 核心 | 14 用例集 | sync/notify 响应外层信封 `{status:SUCCESS,data:{entries,serverTime}}` + entry/ImEvent camelCase 但 `createAt`（非 createdAt）+ `SyncEntryKind` 唯一 snake_case 枚举（no_change/events/snapshot/too_long）+ `eventType` 数字（1new/2edit/3delete/6read）；allHash FNV-1a 64 两端 byte-exact 锚定向量；WS 帧 `event_seq` 三级 fallback；gate 双路径（WS 严格+1 vs sync 跳稀疏空洞）。**发现源码注释不一致 1 处**：domain/sync.rs:227 hint 残留旧「外层 snake_case」文案，以 struct rename_all=camelCase + 单测为准。 |

---

## 3. C. 缺口三态对照（gap-\*）

| 文件 | 域 | covered / partial / missing | 一句话导读 |
|------|----|------------------------------|-----------|
| [partials/9--gap-posts.md](partials/9--gap-posts.md) | posts | 27 / 7 / 1 | 34/35 全链路已迁，仅 createMock 有意不迁。真正风险**不在接口缺失，在 WS 事件命名重构**：源把 revoke/urgent/confirm 笼统标 `post_update`，im-server 细分为 `posts_update`/`post_urgent`/`post_confirm_urgent`（有意升级，须与客户端监听名双端核对）。 |
| [partials/10--gap-channel-sync.md](partials/10--gap-channel-sync.md) | channel | 21 / 13 / 0 | 路由面 **34/34 全注册**，wire 契约 verbatim 对齐，无缺失路由。13 个「部分迁移」同一根因：靠 `CreateCsesPost` 落系统消息 `post` 帧表达副作用，而消费侧编排（P0-2）未完整。完成度最高的域。 |
| [partials/11--gap-user-misc.md](partials/11--gap-user-misc.md) | user-misc | 7 / 4 / 4 | 已迁 7（含修 3 个源 bug）；search 四件套路由+信封通但 app 层全空桩 + data 形态偏离源；真阻塞缺失是 #2 users/status/ids（在线状态）+ #3 createUser（开户），#7 groups / #10 loadTarget 是 uncertain 非阻塞。 |
| [partials/12--gap-bot-agent.md](partials/12--gap-bot-agent.md) | bot-agent | 31 / 6 / 2 | 路由面 ~95% 1:1 移植，无 wire 漂移。真阻塞=P0-2 SendPosts 编排（全域共性）+ BotAgentWebhookEvent Pulsar fanout（本域特有）；#33/#34 timeline/cost uncertain 未挂路由；#38-40 demo 故意排除。 |

**三态合计**：covered **86** / partial **30** / missing **7**（详见 `30-capability-gap-matrix.md`）。

> 缺口矩阵 + 「替代 mattermost 的阻塞能力」优先级总清单见：
> **[30-capability-gap-matrix.md](30-capability-gap-matrix.md)**

---

## 4. 待核对（cross-validation 待闭合项）

> 校验方法：把各 backend 报告的 `wsEventsSeen` 与 `backend-ws-events.md` 实际覆盖的 19 个事件交叉比对，并标注客户端监听契约需 live 核对项。

### 4.1 backend 报告但 WS 文档未收录的事件（需澄清）

| 事件 | 报告来源 | 是否在 backend-ws-events.md | 结论 / 待办 |
|------|----------|------------------------------|-------------|
| `new_user` | user-misc（createUser #3 链） | ❌ 否 | **非缺漏**：WS agent 只追实际可达的 csesapi 业务链；`createUser` 未迁（gap-user-misc missing #3），故触发路径不存在，WS 文档据实未收。常量在 im-server `event_type.go:65` 已定义。**待 createUser 裁决迁移后补 WS 链路验证。** |
| `user_updated` | user-misc（createUser #3 链） | ❌ 否 | 同上（常量 `event_type.go:64` 在，无触发路径）。 |
| `change_props`（channel change/props） | channel | ❌ 否（且非 WS 事件） | **确认非 WS 事件**：change/props 落库后**不广播**（gap-channel #15 + backend-http-channel 已标注「源仅落库无 Publish」），im-server 同样无广播，一致。`change_props` 不是 WS action 字符串，是 backend agent 列举的「该路由涉及的字段域」误入 wsEventsSeen。**无需补，仅澄清。** |
| `post`（agent/bot variant） | bot-agent | ✅ 是（标准 `post`） | 已覆盖：bot/agent 回调复用标准 `WebsocketEventPost="post"`，WS 文档第 19 节已含 post 的 bot 变体。 |

### 4.2 backend 报告的「出站事件」非 WS hub 广播（归类澄清）

| 事件 | 来源 | 性质 | 结论 |
|------|------|------|------|
| `bot_added` / `bot_removed` | bot-agent | HTTP webhook 出站（HMAC-SHA256 + Bearer） | 非 WS hub 广播，WS 文档据实不收，正确。 |
| `BotAgentWebhookEvent` | bot-agent | Pulsar topic fanout | 非 WS 广播；且 im-server 侧 Pulsar fanout 未接（gap-bot-agent P1）。属消费侧缺口，已入矩阵。 |

### 4.3 WS 事件命名重构（双端 live 核对，矩阵 P0）

`backend-ws-events.md` 列源端 19 事件，但 `cses-im-server` 在 posts 域做了**事件细分重构**（`posts_update`/`post_urgent`/`post_confirm_urgent` 取代笼统 `post_update`）。这是源端清单与目标端的**有意分歧**，是否构成「替代 mattermost 时撤回/加急静默失效」取决于客户端 Rust handler 当前监听的 action 名——**必须双端日志联调实证**（见矩阵 P0-A，im-dual-endpoint-log-debug skill）。`client-usecases.md` 的 router.rs action→handler 全映射表是核对锚点。

### 4.4 其他 partial 自陈的待核对点（不阻塞索引，登记备查）

- **client-tauri-im.md**：`domain/sync.rs:227` hint 文案残留旧「外层 snake_case」说法，与 struct `rename_all=camelCase` + 单测冲突——以 struct + 单测为准（文案 bug，非协议 bug）。
- **gap-user-misc.md** 注 A/B/C：notification LoadByUserId 的 `page.Page*page.Page` offset bug 是否已修（未深入 store 函数体）；search data 形态接真实实现时须 verbatim 对齐源裸数组类型 + 补 `channelDetails` 第四 key；`PERSONAL_USER_FORBIDDEN` magic string 是否 verbatim 保留（未 Read middleware 实体）。
- **gap-posts.md**：`App.ReadPost`（publishRead）是否在 `post_read` 之外**也发 `update_channel`** 清 badge（源双事件，须 grep 确认 im-server 未漏）。
