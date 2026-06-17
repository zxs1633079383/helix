# 执行级能力缺口矩阵（cses-im-server 替代 mattermost csesapi）

> 生成时间：2026-06-16 · 合并自 4 个 `partials/gap-*.md`
> 三态判据：
> - **covered（✅ 已迁移）**：路由 + app + DB + WS 全链路就位，wire 契约 verbatim 对齐
> - **partial（🟡 部分迁移）**：路由+入参已注册，但 WS 表达/字段形态/消费侧编排/空桩 有差异
> - **missing（❌ 缺失）**：未注册路由（无 live handler），或有意不迁
>
> 配套：索引导读 [00-README-index.md](00-README-index.md)；cutover 关键路径 `cses-im-server/docs/2026-06-15-cutover-checklist-顶替mattermost.md`。

---

## 1. 三态总矩阵（按域）

| 域 | 源接口数 | ✅ covered | 🟡 partial | ❌ missing | 缺失中「有意不迁/uncertain」 | gap 文档 |
|----|---------|-----------|-----------|-----------|------------------------------|----------|
| posts | 35 | 27 | 7 | 1 | 1（createMock 压测旁路） | [gap-posts.md](partials/9--gap-posts.md) |
| channel | 34 | 21 | 13 | 0 | 0 | [gap-channel.md](partials/10--gap-channel-sync.md) |
| user-misc | 15 | 7 | 4 | 4 | 2（groups / loadTarget uncertain；createUser/status 是真阻塞） | [gap-user-misc.md](partials/11--gap-user-misc.md) |
| bot-agent | 40（含 3 demo） | 31 | 6 | 2 | 5（3 demo 排除 + 2 timeline/cost uncertain） | [gap-bot-agent.md](partials/12--gap-bot-agent.md) |
| **合计** | **124** | **86** | **30** | **7** | **8** | — |

> 口径说明：
> - bot-agent 40 接口含 3 个 cross-repo demo（#38-40，GitNexus trace 评分合成，**有意排除**，不计真实业务缺口）。计真实业务接口 37。
> - **「真阻塞缺失」仅 2 个**（user-misc #2 users/status/ids、#3 createUser）；其余 5 个缺失是 uncertain（源本就空骨架/全注释）或有意不迁（createMock）。
> - **30 个 partial 高度同根**：channel 域 12 个 + bot-agent 域 2 个 + posts 域部分，**根因都是 P0-2「CreateCsesPost 系统消息消费侧编排不完整」**——非接口缺失，是消费落库闭环未完工。

---

## 2. 「替代 mattermost 的阻塞能力」优先级总清单

> 按「替代 mattermost 后客户端可感知的功能后果」排序聚合，跨域去重。

### P0 — 阻塞 cutover（必须关闭才能替代 mattermost）

#### P0-1 · CreateCsesPost 系统消息消费侧编排不完整（SendPosts 编排）— 全域共性根因
- **影响范围**：channel 域 12 路由（#7,8,9,13,16,17,19,20,21,22,32,33：建群欢迎语 / 改群名·介绍·描述·权限系统提示 / 加管理员 / 置顶 / 入退群系统消息）+ bot-agent 域 2 路由（#23/#24 Bot/Agent 发消息）+ posts 域系统消息表达路由。
- **具体缺**：`total_msg_count(_all)` 未读计数 / `posts_update` 批帧 / reply-chain echo / `notification`·push fanout。单节点降级到 12 行 `ConsumePostSenderMessage`，缺完整 `SendPosts` 编排。
- **后果**：接收方**未读数 / 艾特数 / push 推送**维度与 mattermost 不一致；建群/改群信息/加管理员/置顶/入退群的系统消息可能不落库或计数不准。
- **来源**：gap-channel P0-1、gap-bot-agent P0-1、gap-posts（系统消息路由共担）。
- **动作**：实现 cutover-checklist **P0-2** SendPosts 完整编排，一处修则全域 14+ 路由随之修好。

#### P0-A · WS 事件命名重构双端核对（posts 撤回/加急/确认 4 条）
- **影响**：posts 域 revoke / urgentPost / urgentConfirm / urgentCancel。源端笼统 `post_update`，im-server 细分为 `posts_update` / `post_urgent` / `post_confirm_urgent`（event_type.go L25-26 自陈有意升级）。
- **后果（若客户端仍按旧名监听）**：撤回不生效 / 加急不弹 / 确认回执丢，**静默失效**。
- **判据**：是「协议升级」还是「drift」取决于客户端 Rust handler 当前监听的 action 名 —— **不在后端可单方定论**。
- **来源**：gap-posts WS 专章 P0。
- **动作**：双端日志联调（im-dual-endpoint-log-debug skill）实发一条 revoke/urgent，比对 `client-usecases.md` router.rs action→handler 映射表，确认客户端已切细分名 or 须后端兼容旧名。

#### P0-B · `post/read` 是否同发 `update_channel` 清 badge
- **影响**：posts 域 #23 `/post/read`。源是双事件（`post_read` + `update_channel`）；im-server `publishRead` 须确认未漏 `update_channel`。
- **后果**：已读后未读 badge 不清。
- **来源**：gap-posts P0-2。
- **动作**：grep `cses_post_mark_read.go publishRead`（静态）+ 活体核对（遵循 decisions/2026-05-29-static-vs-live）。

### P1 — 高阻塞（功能缺位，需裁决迁移 or 外部供给）

#### P1-1 · createUser（POST /users，user-misc #3）— 真阻塞缺失
- 用户开通全流程。原生 mattermost 注册深耦合（token/invite/admin/signup 四分支 + 审计 + guest license + `new_user`/`user_updated` 广播）未迁。
- **后果**：im-server 单仓无法完成开户；`new_user`/`user_updated` WS 无触发路径。
- **来源**：gap-user-misc P1-1（NEED_FIX + cutover P1-4）。
- **动作**：裁决「迁移 or 由 ucenter/cses 侧供给」。

#### P1-2 · users/status/ids（POST /users/status/ids，user-misc #2）— 真阻塞缺失
- 批量在线状态查询，依赖 PlatformService statusCache（mattermost 深耦合基础设施）。
- **后果**：前端在线状态 / 红点展示阻塞。
- **来源**：gap-user-misc P1-2（cutover P1-4）。

#### P1-3 · BotAgentWebhookEvent 群聊消息 Pulsar fanout 未接（bot-agent）
- 群内 @bot 触发外部 agent 的实时分发链未通；依赖 P0 消费侧 + bot-agent webhook sender。
- **后果**：「群聊里召唤 AI bot」场景不可用。
- **来源**：gap-bot-agent P1。

#### P1-4 · channel 系统消息 WS 表达手段差异（postPinned / permission / displayName·orient·purpose）
- #21/#22 postPinned 从 `post_pin` 改走系统消息 `post`；#9/#13/#16/#17 从 `update_channel` 改走系统消息 `post`。
- **后果**：若客户端仍监听 `post_pin`/`update_channel` 帧渲染群头部/置顶变更，群信息变更**不实时反映**。
- **来源**：gap-channel P1（#2/#3）。
- **动作**：live 验证系统消息能否等效驱动 UI，否则保留原 `post_pin`/`update_channel` 帧。

### P2 — 中阻塞（已注册空桩 / 依赖外部接通 / 契约形态偏离）

#### P2-1 · search 四件套（user-misc #11-#14）
- 路由+信封已通，但 `app.Search*` 全空桩（ES/store 未迁）。
- **两类缺口**：(a) 功能空桩，恒返回空；(b) **data 形态偏离源**——源裸 `*entity.Posts`/`*entity.UserEntities`/`*model.ChannelList`，im-server 统一成 `map[string]any{key:[]}` 套 key，且 doSearch 的 searchAll 缺源的 `channelDetails` 第四 key。
- **动作**：接 ES+store 时 verbatim 对齐源 data 结构（含 channelDetails），否则前端 key off 解析失败。
- **来源**：gap-user-misc P2。

#### P2-2 · Bot Agent team 成员/用户脱敏查询依赖 userResolver 注入（bot-agent #25/#26）
- 依赖 paas-grpc resolver；未注入返回空 slice / user not found。`Email` 恒空、`Role` 恒 member。
- **来源**：gap-bot-agent P2（依赖 P2-13 gRPC-to-cses 接通）。

#### P2-3 · createPosts / post/read/list wire key 大小写对齐（posts #2/#24）
- `createPosts` 请求体源无 tag=PascalCase（`Posts`/`ChannelIds`），im-server camelCase（`posts`/`channelIds`）；`PostReadDto` 响应无 tag=PascalCase（`PostId`/`ReadBits`）。
- **后果**：若前端按源 PascalCase 发/收，im-server decode 空 / 前端读不到已读 bitmap。
- **动作**：核对前端实发 key + im-server entity 同形。
- **来源**：gap-posts P1。

#### P2-4 · channel manager 变更缺 GrpcInvoke 对端透传（channel #19/#20）
- 源 add/remove manager 有 GrpcInvoke 到 paas；迁移侧仅靠 Pulsar 系统消息替代。
- **来源**：gap-channel P2（取决于 paas 是否仍在链路）。

#### P2-5 · consume-hop trace 透传缺失（channel P2-14）
- 系统消息经 Pulsar 消费边界不透传 traceparent，分布式 trace 断链（违反 Trace 父子约束）。不影响功能，影响可观测性。
- **来源**：gap-channel P2。

### P3 — 低阻塞 / 可不修（uncertain / 技术债 / 有意不迁）

| 项 | 域 | 性质 | 处置 |
|----|----|------|------|
| notification/loadTarget（#10） | user-misc | uncertain（源 store 全注释返 nil） | 裁决迁移 or 废弃（cutover P1-5），非阻塞 |
| groups/getGroups（#7） | user-misc | uncertain（源空骨架从未实现） | drop 即可，非阻塞 |
| agent session timeline/cost（#33/#34） | bot-agent | uncertain（类型在 entity 无 handler） | 裁决迁移 or 废弃（cutover P1-8），AI Lab 审计用 |
| bot-agent/config 暴露 ent.BotAgentConfig（#12） | bot-agent | 技术债（未抽 DTO，同 mattermost） | 非阻塞，schema 演进有风险 |
| createMock（posts #6） | posts | 有意不迁（压测旁路直发 Pulsar MockTopic） | 已拍板废弃 |
| cross-repo demo（#38-40） | bot-agent | 故意排除（GitNexus trace 合成 handler） | 不迁移 |

---

## 3. 替代 mattermost 的 Top 阻塞收敛（一句话版）

1. **P0-1 SendPosts 编排（cutover P0-2）** — 单点修复解锁 14+ 路由，是 cutover 第一关键路径。
2. **P0-A WS 事件命名双端核对** — posts 撤回/加急/确认细分事件 vs 客户端监听名，须日志联调实证。
3. **P1-1 / P1-2 createUser + users/status/ids** — 唯二「真阻塞缺失」，须裁决迁移 or 外部供给（ucenter/cses/paas）。
4. **P1-3 BotAgentWebhookEvent Pulsar fanout** — 群内召唤 AI bot 实时链未通。
5. **P0-B / P1-4 channel·post WS 表达手段差异** — 系统消息能否等效驱动客户端 UI（清 badge / 群头部 / 置顶），须 live 验证。

> 注：86 covered + 30 partial 中绝大多数「partial」会随 P0-1 单点修复转为 covered。真正需要**业务裁决**（非纯实现）的只有：createUser/status 供给方、4 个 uncertain 接口去留、search data 形态对齐窗口。
