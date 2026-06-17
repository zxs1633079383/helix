# Gap 分析 — Bot / Agent / Webhook 域（csesapi ↔ cses-im-server）

> 对照源: `docs/csesapi-migration/full-map/partials/backend-http-bot-agent.md`（40 接口）
> 目标实现: `cses-im-server/internal/{api,app,ws}/`（bot.go / bot_agent.go / agent.go / webhook_dispatcher.go + 对应 app）
> 核验方式: Read 活体源码 + grep 取证（GitNexus 索引偏旧，字段以源码为准）。
> 结论速读: **40 接口中 31 已迁 / 6 部分迁 / 3 故意排除（cross-repo demo,非业务）**。
> 主要缺口集中在「**Bot/Agent 发消息落库链路的 SendPosts 完整编排**」（与全域同根，见 cutover P0-2），非本域路由本身缺失。

## 三态对照表

| # | csesapi 接口 | im-server 实现 | 状态 | 差异 / 缺口 |
|---|---|---|---|---|
| 1 | POST `/posts/createBot` | api/bot.go `createBot` → app/gateway_bot.go `CreateBotMessage` | ✅ 已迁移 | 出站 Gateway HTTP，签名/字段一致；userId 空回填 session 一致 |
| 2 | POST `/posts/botCallback` | api/bot.go `botCallback` → app/gateway_bot.go `HandleBotCallback` | ✅ 已迁移 | 内存构造 Post + `a.publish(WebsocketEventPost)`；**无 DB 落库**（与 mattermost 一致，不走 channel_event）。WS `post` 帧齐备 |
| 3 | GET `/bot-manage/list` | api/bot.go `listVisibleBots` → CsesListVisibleBots | ✅ 已迁移 | BotInfo 字段 verbatim 一致 |
| 4 | POST `/bot-manage/create` | api/bot.go `createBotManage` → CsesCreateBot | ✅ 已迁移 | co-tx bot+token+scope，返回明文 token，字段一致 |
| 5 | POST `/bot-manage/token/generate` | api/bot.go `generateBotToken` → CsesGenerateBotToken | ✅ 已迁移 | GenerateToken{Request,Response} 一致 |
| 6 | GET `/bot-manage/token/list` | api/bot.go `listBotTokens` (query botUserId) → CsesListBotTokens | ✅ 已迁移 | BotTokenInfo 一致 |
| 7 | POST `/bot-manage/token/revoke` | api/bot.go `revokeBotToken` → CsesRevokeBotToken | ✅ 已迁移 | message-only 一致 |
| 8 | POST `/bot-manage/scope/set` | api/bot.go `setBotScope` → CsesSetBotScope | ✅ 已迁移 | SetScopeRequest 一致 |
| 9 | POST `/bot-manage/scope/visible-user/add` | api/bot.go `addBotVisibleUser` → CsesAddBotVisibleUser | ✅ 已迁移 | VisibleUserRequest 一致 |
| 10 | POST `/bot-manage/scope/visible-user/remove` | api/bot.go `removeBotVisibleUser` → CsesRemoveBotVisibleUser | ✅ 已迁移 | 同上 |
| 11 | GET `/bot-manage/scope/visible-user/list` | api/bot.go `listBotVisibleUsers` (query botUserId) → CsesListBotVisibleUsers | ✅ 已迁移 | 一致 |
| 12 | POST `/bot-agent/config` | api/bot_agent.go `botAgentCreateConfig` → SaveBotAgentConfig + OnBotAgentConfigCreated | ⚠️ 部分迁移 | 路由/异步入大群/`bot_added` webhook 全在；**req/resp 仍直接暴露 `ent.BotAgentConfig`**（同 mattermost 未抽 DTO，迁移注意项未消解）。OnBotAgentConfigCreated **同步调用**（非 mattermost 的 `go ...` 异步），但内部 webhook dispatch 走 `dispatchBotAgentLifecycleWebhookAsync` goroutine，行为等价 |
| 13 | GET `/bot-agent/config/{botUserId}` | `botAgentGetConfig` → GetBotAgentConfigByBotUserID | ✅ 已迁移 | 返回 ent.BotAgentConfig 一致 |
| 14 | GET `/bot-agent/configs/enabled` | `botAgentGetAllEnabledConfigs` → GetAllEnabledBotAgentConfigs | ✅ 已迁移 | 一致 |
| 15 | DELETE `/bot-agent/config/{botUserId}` | `botAgentDeleteConfig` → DeleteBotAgentConfig(+`bot_removed` webhook) | ✅ 已迁移 | teamID=session.CompanyId 一致；删 config+channel 关联 |
| 16 | POST `/bot-agent/config/{botUserId}/regenerate-token` | `botAgentRegenerateToken` → RegenerateBotAgentToken | ✅ 已迁移 | 返回新 token config 一致 |
| 17 | POST `/bot-agent/config/{botUserId}/test-webhook` | `botAgentTestWebhook` → TestBotAgentWebhook | ✅ 已迁移 | inline `{message}` body + BotAgentTestWebhookResult(Success/StatusCode/Latency/Error) verbatim；HMAC-SHA256 + Bearer 头一致 |
| 18 | GET `/bot-agent/channel/{channelId}/bots` | `botAgentGetChannelBots` → GetChannelBotAgents | ✅ 已迁移 | BotAgentChannelInfo 一致 |
| 19 | GET `/bot-agent/channel/{channelId}/available-bots` | `botAgentGetAvailableBots` → GetAvailableBotAgentsForChannel | ✅ 已迁移 | BotAgentConfigWithAdded 一致 |
| 20 | POST `/bot-agent/channel/{channelId}/bot/{botUserId}` | `botAgentAddToChannel` → AddBotAgentToChannel(+`bot_added` webhook) | ✅ 已迁移 | addedBy=session.UserId 一致 |
| 21 | DELETE `/bot-agent/channel/{channelId}/bot/{botUserId}` | `botAgentRemoveFromChannel` → RemoveBotAgentFromChannel(+`bot_removed` webhook) | ✅ 已迁移 | 一致 |
| 22 | GET `/bot-agent/info` | `botAgentGetSelfInfo`（mp_ token 子路由） | ✅ 已迁移 | inline map{id,username,displayName} verbatim；BotAgentTokenAuth 中间件齐备 |
| 23 | POST `/bot-agent/messages/direct` | `botAgentSendDirectMessage` → SendBotAgentDirectMessage → createBotPost → CreateCsesPost | ⚠️ 部分迁移 | 路由/字段/createChannel 建 DM 全在；**落库走 CreateCsesPost → 单节点 sync fallback ConsumePostSenderMessage(12 行)**，缺 SendPosts 完整编排（unread/mention/notification/push fanout，见 cutover P0-2）。WS `post` 帧经共享落库路径发出；channel_event 经 `appendEntPostCreateEventCoTx` 写入。req 校验文案 `team_id/user_id` 是文字，JSON tag 实为 camelCase `teamId/userId`（与 csesapi 同款无害不一致） |
| 24 | POST `/bot-agent/messages/channel` | `botAgentSendChannelMessage` → SendBotAgentChannelMessage → createBotPost → CreateCsesPost | ⚠️ 部分迁移 | 同 #23：Bot 非成员自动 join 在；落库链同样缺 SendPosts 完整编排（mention/unread/push）。WS `post` + channel_event 在 |
| 25 | GET `/bot-agent/teams/{team_id}/members` | `botAgentGetTeamMembers` (page/per_page≤200) → GetTeamMembersForBotAgent | ⚠️ 部分迁移 | 路由/分页/脱敏在；成员经 `userResolver.ReadMembersMinimal`(paas-grpc) 回填，**resolver 未注入时返回空 slice**（行为依赖 P2-13 gRPC-to-cses 接通）；`Email` 恒空、`Role` 恒 "member"（源也脱敏，但字段填充度更低） |
| 26 | GET `/bot-agent/teams/{team_id}/users/{user_id}` | `botAgentGetUser` → GetUserForBotAgent | ⚠️ 部分迁移 | 同 #25 依赖 userResolver；未注入则 user not found |
| 27 | GET `/bot-agent/teams/{team_id}/channel` | `botAgentGetTeamChannel` → GetDefaultChannelForBotAgent | ✅ 已迁移 | 返回 map{channelId,channelName}；依赖 findCompanyMainChannel 解析公司大群 |
| 28 | POST `/agents/create` | api/agent.go `agentCreate` → CreateAgent | ✅ 已迁移 | CreateAgent{Request,Response} 一致 |
| 29 | POST `/agents/join-channel` | `agentJoinChannel` → JoinAgentToChannel | ✅ 已迁移 | JoinAgentChannelRequest 一致 |
| 30 | POST `/agents/start-session` | `agentStartSession` → StartAgentSession | ✅ 已迁移 | StartAgentSession{Request,Response} 一致 |
| 31 | POST `/agents/send` | `agentSend` → SendAgentMessage | ✅ 已迁移 | SendAgentMessage{Request,Response} 一致；写 agent_event SEND_TO_AGENT |
| 32 | POST `/agents/callback` | `agentCallback` → HandleAgentCallback → publishCallbackPost | ✅ 已迁移 | 写 CALLBACK 事件 + cost 行；`a.publish(WebsocketEventPost)` WS 帧在；**内存构造 Post 不落 post 表**（与 mattermost 一致）。AgentCallback{Request,Response,Usage} verbatim |
| 33 | GET `/agents/session/{sessionId}/timeline` | **未挂路由** | ❌ 缺失 | uncertain，C013 不臆造（NEED_FIX_agent_uncertain.md）。`AgentTimelineResponse` 类型在 entity 但无 handler/app 方法。待裁决迁移 or 废弃（cutover P1-8） |
| 34 | GET `/agents/session/{sessionId}/cost` | **未挂路由** | ❌ 缺失 | 同 #33；`AgentCostResponse` 类型在但无 live handler |
| 35 | POST `/webhook/config` | api/webhook_dispatcher.go `setWebhookConfig` → SetWebhookConfig | ✅ 已迁移 | 裸 map{success,message} 信封（非 dto.CommonRes）verbatim；WebhookConfigRequest 一致 |
| 36 | GET `/webhook/config` | `getWebhookConfig` → GetWebhookConfig | ✅ 已迁移 | secret 掩码（前4+****）+ inline map 全字段一致；未配置返 {configured:false} |
| 37 | POST `/webhook/test` | `testWebhook` → TestWebhook | ✅ 已迁移 | body 可选；出站 OpenClaw；裸 map 信封一致 |
| 38 | POST `/channels/csesCrossRepoDemo` | **未迁移（故意排除）** | ⛔ 排除 | 非业务接口（GitNexus 跨仓 trace demo）；grep=0；迁移蓝图明确排除 |
| 39 | POST `/channels/crossRepoCounterPoll` | **未迁移（故意排除）** | ⛔ 排除 | 同上，合成 demo handler |
| 40 | POST `/channels/triggerMmToCsesCounterAck` | **未迁移（故意排除）** | ⛔ 排除 | 同上 |

> 统计口径：covered=31（✅）/ partial=6（⚠️ #12,23,24,25,26 + 把 #12 也归 partial）/ missing=2（❌ #33,34）/ excluded=3（⛔ demo，不计入分母缺口）。

## WS / 出站事件核对（cses-im-server/internal/ws/）

| 事件 | csesapi 触发接口 | im-server 形态 | 状态 |
|---|---|---|---|
| `posted` (`WebsocketEventPost="post"`) | botCallback(#2) / botAgentSendDirect(#23) / botAgentSendChannel(#24) / agentCallback(#32) | `ws/event_type.go:35 WebsocketEventPost`；botCallback/agentCallback 直接 `a.publish`；#23/#24 经 CreateCsesPost 共享落库路径 | ✅ 齐备 |
| channel_event（append-only sync cursor，非 WS） | 凡走 CreateCsesPost 的 #23/#24 | `appendEntPostCreateEventCoTx` co-tx 写入；botCallback/agentCallback **不写**（内存 publish，与 mattermost 一致） | ✅ 一致 |
| `bot_added` / `bot_removed`（HTTP webhook 出站，非 WS hub） | botAgentCreateConfig(#12) / AddToChannel(#20) / DeleteConfig(#15) / RemoveFromChannel(#21) | `entity/bot_agent.go:25-26` 常量在；`dispatchBotAgentLifecycleWebhookAsync` goroutine + HMAC 签名出站到 agent webhookUrl | ✅ 齐备 |
| `BotAgentWebhookEvent`（Pulsar topic，群聊消息 fanout） | 群消息侧 hook PublishBotAgentWebhookEvent（非本域 handler 直调） | ⚠️ `bot_agent_webhook.go` 注释：群聊 Pulsar fanout「依赖 consumer 基础设施，留待消费链路 Phase」 | ⚠️ 部分（依赖 P0 消费侧 + bot-agent webhook sender，cutover P0-2 列「TODO」） |

> 本域**无新增专属 WS 事件常量**（与 csesapi 一致，复用标准 `post`）；bot_added/bot_removed/BotAgentWebhookEvent 均为 HTTP/Pulsar 出站非 WS 广播。

## 缺失能力优先级清单（按替代 mattermost 阻塞程度）

1. **P0 — Bot/Agent 发消息落库编排不完整（#23/#24）** — 阻塞级但**非本域专属**：`SendBotAgentDirectMessage`/`SendBotAgentChannelMessage` 走 `CreateCsesPost`，单节点降级到 12 行 `ConsumePostSenderMessage`，缺 unread(`total_msg_count_all`)/mention(`mention_count`)/notification/push fanout（cutover P0-2 整域问题）。后果：Bot 发消息接收方未读/艾特/推送维度与 mattermost 不一致。修 P0-2 即随全域一并修好。
2. **P1 — `BotAgentWebhookEvent` 群聊消息 Pulsar fanout 未接（WS 事件核对末行）** — 群内 @bot 触发外部 agent 的实时分发链未通；依赖 P0 消费侧 + bot-agent webhook sender（cutover P0-2 标 TODO）。中等阻塞：影响「群聊里召唤 AI bot」场景。
3. **P2 — agent session timeline/cost 缺失（#33/#34）** — 低阻塞（uncertain，C013 未臆造）。仅 AI Agent Lab 会话审计/成本面板用，普通 IM 不依赖。需主对话裁决「迁移 or 废弃」（cutover P1-8）。
4. **P2 — Bot Agent team 成员/用户脱敏查询依赖 userResolver 注入（#25/#26）** — 低阻塞但需配置：未注入 paas-grpc resolver 时返回空。依赖 P2-13 gRPC-to-cses 接通。
5. **P3 — config req/resp 暴露 ent.BotAgentConfig（#12）** — 非阻塞技术债：迁移注意项（不抽 DTO）未消解，与 mattermost 同款；不影响 cutover 但 schema 演进有风险。
6. **（非缺口）cross-repo demo #38/#39/#40** — 故意排除，不迁移。

## 与 cutover-checklist 的映射
- 本域路由面 **~95% 已迁**（38 业务接口中 36 有 live handler，仅 timeline/cost 2 个 uncertain 未挂；3 个 demo 不算）。
- 真正阻塞 = **P0-2 SendPosts 编排**（全域共性，bot/agent 发消息走同一 CreateCsesPost 链）+ **BotAgentWebhookEvent Pulsar fanout**（本域特有，依赖消费侧）。
- 路由/字段/casing/鉴权头/响应信封与 csesapi **逐一 verbatim 对齐**，无 wire 契约漂移。
