# csesapi 后端 HTTP 接口清点 — Bot / Agent / Webhook 域

> 来源文件: `server/channels/csesapi/{bot.go, bot_manage.go, bot_agent.go, agent.go, webhook_dispatcher.go, cross_repo_demo.go}`
> Base prefix: `/api/cses`（`model.APIURLSuffixCSES`），各 sub-router PathPrefix 见 `csesapi/api.go` L167-186。
> 响应包络: 绝大多数 handler 用 `dto.NewCommonRes()`（`SetSuccess/SetFailure/SetMessage/SetData`），webhook_dispatcher 与 cross_repo_demo 用裸 `map[string]interface{}`。
> 字段大小写: 请求/响应 struct **统一 camelCase**（json tag verbatim 摘录于下）。
> ⚠️ 索引偏旧，字段以本文 verbatim 摘录的活体源码为准。

## 接口清单总表

| # | Method + Path | Handler | 鉴权 | App 方法 | 主要 DB 表 | WS/MQ 事件 |
|---|---|---|---|---|---|---|
| 1 | POST `/api/cses/posts/createBot` | createBot | session | CreateBotMessage | (无落库，grpc 出站到 Gateway) | — |
| 2 | POST `/api/cses/posts/botCallback` | botCallback | session(放行) | HandleBotCallback | post（CreateCsesPost） | `posted` + channel_event |
| 3 | GET `/api/cses/bot-manage/list` | csesListVisibleBots | session | CsesListVisibleBots | bot / cses_bot_scope / cses_bot_visible_user | — |
| 4 | POST `/api/cses/bot-manage/create` | csesCreateBot | session | CsesCreateBot | bot / cses_bot_token / cses_bot_scope | — |
| 5 | POST `/api/cses/bot-manage/token/generate` | csesGenerateToken | session | CsesGenerateBotToken | cses_bot_token | — |
| 6 | GET `/api/cses/bot-manage/token/list` | csesListTokens | session | CsesListBotTokens | cses_bot_token | — |
| 7 | POST `/api/cses/bot-manage/token/revoke` | csesRevokeToken | session | CsesRevokeBotToken | cses_bot_token | — |
| 8 | POST `/api/cses/bot-manage/scope/set` | csesSetScope | session | CsesSetBotScope | cses_bot_scope | — |
| 9 | POST `/api/cses/bot-manage/scope/visible-user/add` | csesAddVisibleUser | session | CsesAddBotVisibleUser | cses_bot_visible_user | — |
| 10 | POST `/api/cses/bot-manage/scope/visible-user/remove` | csesRemoveVisibleUser | session | CsesRemoveBotVisibleUser | cses_bot_visible_user | — |
| 11 | GET `/api/cses/bot-manage/scope/visible-user/list` | csesListVisibleUsers | session | CsesListBotVisibleUsers | cses_bot_visible_user | — |
| 12 | POST `/api/cses/bot-agent/config` | botAgentCreateConfig | session | SaveBotAgentConfig + (async) OnBotAgentConfigCreated | bot_agent_config / channel(member) | webhook `bot_added`(Pulsar HTTP→agent) |
| 13 | GET `/api/cses/bot-agent/config/{botUserId}` | botAgentGetConfig | session | GetBotAgentConfigByBotUserID | bot_agent_config | — |
| 14 | GET `/api/cses/bot-agent/configs/enabled` | botAgentGetAllEnabledConfigs | session | GetAllEnabledBotAgentConfigs | bot_agent_config | — |
| 15 | DELETE `/api/cses/bot-agent/config/{botUserId}` | botAgentDeleteConfig | session | DeleteBotAgentConfig | bot_agent_config / bot_agent_channel | — |
| 16 | POST `/api/cses/bot-agent/config/{botUserId}/regenerate-token` | botAgentRegenerateToken | session | RegenerateBotAgentToken | bot_agent_config | — |
| 17 | POST `/api/cses/bot-agent/config/{botUserId}/test-webhook` | botAgentTestWebhook | session | TestBotAgentWebhook | bot_agent_config（读 webhook url） | — (HTTP 出站测试) |
| 18 | GET `/api/cses/bot-agent/channel/{channelId}/bots` | botAgentGetChannelBots | session | GetChannelBotAgents | bot_agent_channel / bot_agent_config | — |
| 19 | GET `/api/cses/bot-agent/channel/{channelId}/available-bots` | botAgentGetAvailableBots | session | GetAvailableBotAgentsForChannel | bot_agent_config / bot_agent_channel | — |
| 20 | POST `/api/cses/bot-agent/channel/{channelId}/bot/{botUserId}` | botAgentAddToChannel | session | AddBotAgentToChannel | bot_agent_channel / channel / channelmember | — |
| 21 | DELETE `/api/cses/bot-agent/channel/{channelId}/bot/{botUserId}` | botAgentRemoveFromChannel | session | RemoveBotAgentFromChannel | bot_agent_channel | — |
| 22 | GET `/api/cses/bot-agent/info` | botAgentGetSelfInfo | **Bot Token (mp_)** | — (从 ctx 取 config) | — | — |
| 23 | POST `/api/cses/bot-agent/messages/direct` | botAgentSendDirectMessage | **Bot Token** | SendBotAgentDirectMessage | channel / channelmember / post | `posted` + channel_event |
| 24 | POST `/api/cses/bot-agent/messages/channel` | botAgentSendChannelMessage | **Bot Token** | SendBotAgentChannelMessage | channel / post | `posted` + channel_event |
| 25 | GET `/api/cses/bot-agent/teams/{team_id}/members` | botAgentGetTeamMembers | **Bot Token** | GetTeamMembersForBotAgent | channelmember / user | — |
| 26 | GET `/api/cses/bot-agent/teams/{team_id}/users/{user_id}` | botAgentGetUser | **Bot Token** | GetUserForBotAgent | user | — |
| 27 | GET `/api/cses/bot-agent/teams/{team_id}/channel` | botAgentGetTeamChannel | **Bot Token** | GetDefaultChannelForBotAgent | channel | — |
| 28 | POST `/api/cses/agents/create` | createAgent | session | CreateAgent | bot | — |
| 29 | POST `/api/cses/agents/join-channel` | joinAgentChannel | session | JoinAgentToChannel | cses_channel / channelmember | — |
| 30 | POST `/api/cses/agents/start-session` | startAgentSession | session | StartAgentSession | cses_channel / agent_session | — |
| 31 | POST `/api/cses/agents/send` | sendAgentMessage | session | SendAgentMessage | agent_event | — |
| 32 | POST `/api/cses/agents/callback` | agentCallback | session(放行) | HandleAgentCallback | agent_event / post（CreateCsesPost） | `posted` + channel_event |
| 33 | GET `/api/cses/agents/session/{sessionId}/timeline` | loadAgentTimeline | session | LoadAgentTimeline | agent_event | — |
| 34 | GET `/api/cses/agents/session/{sessionId}/cost` | loadAgentCost | session | LoadAgentCost | agent_event | — |
| 35 | POST `/api/cses/webhook/config` | setWebhookConfig | session | SetWebhookConfig | (内存/配置，无 ent 表) | — |
| 36 | GET `/api/cses/webhook/config` | getWebhookConfig | session | GetWebhookConfig | (内存/配置) | — |
| 37 | POST `/api/cses/webhook/test` | testWebhook | session | TestWebhook | — | — (HTTP 出站到 OpenClaw) |
| 38 | POST `/api/cses/channels/csesCrossRepoDemo` | csesCrossRepoDemo | session | app.ReadMember/ReadDeptPositions/ReadOrganizes (grpc→cses) | — (跨仓 grpc demo) | — |
| 39 | POST `/api/cses/channels/crossRepoCounterPoll` | crossRepoCounterPoll | session | (本地 demo，故意报错) | — | — |
| 40 | POST `/api/cses/channels/triggerMmToCsesCounterAck` | triggerMmToCsesCounterAck | session | app.GrpcInvoke (CsesGrpc /Demo/...) | — | — |

> 注：bot-agent 子路由通过 `agentRouter := BotAgent.PathPrefix("").Subrouter()` + `BotAgentTokenAuth` 中间件挂载（bot_agent.go L48-64）；中间件检测 `Authorization: Bearer mp_<token>` 命中则走 token 鉴权注入 ctx，否则放行给 session 鉴权（管理接口）。

---

## 1. POST /api/cses/posts/createBot

- **App 方法**: `App.CreateBotMessage(*entity.BotMessageRequest) (*entity.BotMessageResponse, error)` (`channels/app/gateway_bot.go`)
- **DB 表**: 无落库；grpc/HTTP 出站把消息推到外部 Gateway。userId 为空时回填 `c.AppContext.Session().UserId`。
- **触发 WS**: 无（出站到 Gateway，由 botCallback 回流时才推 WS）。
- **业务说明**: 把用户消息发往外部 AI Gateway（openclaw）。

请求结构 verbatim (`entity.BotMessageRequest`):
```go
type BotMessageRequest struct {
	SessionId        string `json:"sessionId"`
	UserId           string `json:"userId"`
	ChannelId        string `json:"channelId,omitempty"`
	ConversationType string `json:"conversationType,omitempty"` // "direct", "group", "channel"
	Text             string `json:"text"`
}
```
响应结构: `dto.CommonRes`（success/message，无 data）。内部 `BotMessageResponse{Success bool, Error string}` 仅用于 app 层判定。

---

## 2. POST /api/cses/posts/botCallback

- **App 方法**: `App.HandleBotCallback(*entity.BotCallbackRequest) (*entity.BotCallbackResponse, error)`
- **DB 表**: `post`（经 `CreateCsesPost` 落库）。
- **触发 WS**: `posted`（`model.WebsocketEventPost`，gateway_bot.go L175-177 `a.Publish`）+ channel_event（CreateCsesPost 共享路径写）。
- **业务说明**: 接收 Gateway/AI 回调，把 AI 回复落成一条 post 并推 WS 给目标 channel/user。`To` 字段 `"user:userId"` 或 `"channel:channelId"`。

请求结构 verbatim (`entity.BotCallbackRequest`):
```go
type BotCallbackRoutingInfo struct {
	ChannelID string `json:"channelId,omitempty"`
	UserID    string `json:"userId,omitempty"`
	SessionID string `json:"sessionId,omitempty"`
}
type BotCallbackRequest struct {
	MessageID string                 `json:"messageId"`
	Timestamp int64                  `json:"timestamp"`
	To        string                 `json:"to"`
	Text      string                 `json:"text"`
	MediaUrl  string                 `json:"mediaUrl,omitempty"`
	ReplyToId string                 `json:"replyToId,omitempty"`
	ThreadId  string                 `json:"threadId,omitempty"`
	Routing   BotCallbackRoutingInfo `json:"routing"`
}
```
响应: `dto.CommonRes`（resp.Message 回填）。

---

## 3-11. /api/cses/bot-manage/* （Bot 管理 + Token + Scope + Visible-User）

App 方法均在 `channels/app/cses_bot_manage.go`，事务统一走 `Store().ExecEntTxDefault[WithResult]`。

DB 表: `bot`（CSES fork 表名单数，见 table-naming-trap）、`cses_bot_token`、`cses_bot_scope`、`cses_bot_visible_user`。
触发 WS: 无（纯管理 CRUD）。

### 3. GET /bot-manage/list — csesListVisibleBots
- App: `CsesListVisibleBots(userId, companyId string) ([]*entity.BotInfo, error)`；从 session 取 userId/companyId。
- 响应 data: `[]entity.BotInfo`:
```go
type BotInfo struct {
	BotId       string `json:"botId"`
	BotUserId   string `json:"botUserId"`
	Name        string `json:"name"`
	Description string `json:"description"`
	Type        string `json:"type"`
	Active      bool   `json:"active"`
	Source      string `json:"source"` // "owned" | "company" | "individual"
}
```

### 4. POST /bot-manage/create — csesCreateBot
- App: `CsesCreateBot(*entity.CreateBotRequest, ownerUserId string) (*entity.CreateBotResponse, error)`
```go
type CreateBotRequest struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	TeamId      string `json:"teamId,omitempty"`
}
type CreateBotResponse struct {
	BotId     string `json:"botId"`
	BotUserId string `json:"botUserId"`
	Name      string `json:"name"`
	Token     string `json:"token"`
}
```

### 5. POST /bot-manage/token/generate — csesGenerateToken
- App: `CsesGenerateBotToken(*entity.GenerateTokenRequest, createBy string) (*entity.GenerateTokenResponse, error)`
```go
type GenerateTokenRequest struct {
	BotUserId   string `json:"botUserId"`
	Description string `json:"description"`
	ExpireAt    *int64 `json:"expireAt,omitempty"`
}
type GenerateTokenResponse struct {
	TokenId string `json:"tokenId"`
	Token   string `json:"token"`
}
```

### 6. GET /bot-manage/token/list — csesListTokens
- Query: `?botUserId=` (required)。App: `CsesListBotTokens(botUserId) ([]*entity.BotTokenInfo, error)`
```go
type BotTokenInfo struct {
	TokenId     string `json:"tokenId"`
	BotUserId   string `json:"botUserId"`
	Description string `json:"description"`
	ExpireAt    *int64 `json:"expireAt,omitempty"`
	IsActive    bool   `json:"isActive"`
	CreateAt    int64  `json:"createAt"`
}
```

### 7. POST /bot-manage/token/revoke — csesRevokeToken
- App: `CsesRevokeBotToken(tokenId string) error`。请求: `RevokeTokenRequest{ TokenId string \`json:"tokenId"\` }`。响应: message-only。

### 8. POST /bot-manage/scope/set — csesSetScope
- App: `CsesSetBotScope(*entity.SetScopeRequest) error`
```go
type SetScopeRequest struct {
	BotUserId string `json:"botUserId"`
	ScopeType string `json:"scopeType"`
	TeamId    string `json:"teamId,omitempty"`
}
```

### 9 & 10. POST /bot-manage/scope/visible-user/{add,remove} — csesAddVisibleUser / csesRemoveVisibleUser
- App: `CsesAddBotVisibleUser` / `CsesRemoveBotVisibleUser(*entity.VisibleUserRequest) error`
```go
type VisibleUserRequest struct {
	BotUserId string `json:"botUserId"`
	UserId    string `json:"userId"`
}
```

### 11. GET /bot-manage/scope/visible-user/list — csesListVisibleUsers
- Query: `?botUserId=` (required)。App: `CsesListBotVisibleUsers(botUserId)`；返回可见用户列表。

---

## 12-21. /api/cses/bot-agent/* （管理接口，session 鉴权）

App 方法在 `channels/app/bot_agent.go`；DB 表 `bot_agent_config`、`bot_agent_channel`（+ channel/channelmember 用于加群）。

### 12. POST /bot-agent/config — botAgentCreateConfig
- 请求/响应 body 直接是 **ent 结构** `ent.BotAgentConfig`（解码 `json.NewDecoder.Decode(&config)`，非独立 DTO）。校验 `config.BotUserId` 非空。
- App: `SaveBotAgentConfig(*ent.BotAgentConfig) (*ent.BotAgentConfig, error)`；成功后 **异步** `go App.OnBotAgentConfigCreated(saved, teamID)`（teamID=session.CompanyId）。
- **DB 表**: `bot_agent_config`（写）；OnBotAgentConfigCreated 把 bot 加入公司大群（channel/channelmember）。
- **触发 WS/MQ**: OnBotAgentConfigCreated 触发 `bot_added` webhook（HTTP 出站到 agent，非 WS 广播；见 `entity.BotAgentEventBotAdded`）。
- ⚠️ 迁移注意: req/resp 直接暴露 ent 生成类型，迁移到 cses-im-server 需替换为独立 model DTO。

### 13. GET /bot-agent/config/{botUserId} — botAgentGetConfig
- App: `GetBotAgentConfigByBotUserID(botUserID) (*ent.BotAgentConfig, error)`。响应 data: ent.BotAgentConfig。

### 14. GET /bot-agent/configs/enabled — botAgentGetAllEnabledConfigs
- App: `GetAllEnabledBotAgentConfigs() ([]*ent.BotAgentConfig, error)`。

### 15. DELETE /bot-agent/config/{botUserId} — botAgentDeleteConfig
- teamID=session.CompanyId。App: `DeleteBotAgentConfig(botUserID, teamID) error`。表 `bot_agent_config` + `bot_agent_channel`。

### 16. POST /bot-agent/config/{botUserId}/regenerate-token — botAgentRegenerateToken
- App: `RegenerateBotAgentToken(botUserID) (*ent.BotAgentConfig, error)`。返回新 token 的 config。

### 17. POST /bot-agent/config/{botUserId}/test-webhook — botAgentTestWebhook
- 请求 body: inline 匿名 struct `{ Message string \`json:"message"\` }`。
- App: `TestBotAgentWebhook(botUserID, message string) (*entity.BotAgentTestWebhookResult, error)`
```go
type BotAgentTestWebhookResult struct {
	Success    bool   `json:"success"`
	StatusCode int    `json:"statusCode,omitempty"`
	Latency    int64  `json:"latency"`
	Error      string `json:"error,omitempty"`
}
```

### 18. GET /bot-agent/channel/{channelId}/bots — botAgentGetChannelBots
- App: `GetChannelBotAgents(channelID) ([]*entity.BotAgentChannelInfo, error)`
```go
type BotAgentChannelInfo struct {
	BotUserID   string `json:"botUserId"`
	AgentName   string `json:"agentName"`
	Description string `json:"description"`
	AddedBy     string `json:"addedBy"`
	AddedAt     int64  `json:"addedAt"`
}
```

### 19. GET /bot-agent/channel/{channelId}/available-bots — botAgentGetAvailableBots
- App: `GetAvailableBotAgentsForChannel(channelID) ([]*entity.BotAgentConfigWithAdded, error)`
```go
type BotAgentConfigWithAdded struct {
	ID                  string   `json:"id"`
	BotUserID           string   `json:"botUserId"`
	AgentName           string   `json:"agentName"`
	Description         string   `json:"description"`
	AllowedChannelTypes []string `json:"allowedChannelTypes"`
	Added               bool     `json:"added"`
}
```

### 20. POST /bot-agent/channel/{channelId}/bot/{botUserId} — botAgentAddToChannel
- addedBy=session.UserId。App: `AddBotAgentToChannel(channelID, botUserID, addedBy) error`。表 `bot_agent_channel` + channel/channelmember。响应 message "added"。

### 21. DELETE /bot-agent/channel/{channelId}/bot/{botUserId} — botAgentRemoveFromChannel
- App: `RemoveBotAgentFromChannel(channelID, botUserID) error`。表 `bot_agent_channel`。响应 message "removed"。

---

## 22-27. /api/cses/bot-agent/* （Agent 调用接口，Bot Token 鉴权）

鉴权: `Authorization: Bearer mp_<token>` → `BotAgentTokenAuth` 中间件 → `GetBotAgentConfigByToken` → ctx 注入 `*ent.BotAgentConfig`（key=`BotAgentContextKey`）。config.Enabled=false 返 401。

### 22. GET /bot-agent/info — botAgentGetSelfInfo
- 无 App 调用；直接从 ctx 取 config。响应 data（inline map）:
```go
map[string]interface{}{
	"id":          agentConfig.BotUserId,
	"username":    agentConfig.AgentName,
	"displayName": agentConfig.AgentName,
}
```

### 23. POST /bot-agent/messages/direct — botAgentSendDirectMessage
- 校验 `team_id`/`user_id`/`message` 非空。App: `SendBotAgentDirectMessage(botUserId, teamID, userID, channelID, message, createChannel, props, userSnapshot) (*entity.BotAgentSendMessageResponse, error)` (`bot_agent_message.go`)
- **DB 表**: channel / channelmember（必要时建 DM channel）/ post（`CreateCsesPost`）。
- **触发 WS**: `posted` + channel_event（CreateCsesPost 共享路径，bot_agent_message.go L119/L175）。

请求结构 verbatim (`entity.BotAgentSendMessageRequest`，direct 与 channel 共用):
```go
type BotAgentSendMessageRequest struct {
	TeamID        string            `json:"teamId"`
	UserID        string            `json:"userId,omitempty"`
	ChannelID     string            `json:"channelId,omitempty"`
	Message       string            `json:"message"`
	CreateChannel bool              `json:"createChannel,omitempty"`
	Props         map[string]any    `json:"props,omitempty"`
	UserSnapshot  map[string]string `json:"userSnapshot,omitempty"`
}
type BotAgentSendMessageResponse struct {
	PostID   string `json:"postId"`
	CreateAt int64  `json:"createAt"`
}
```

### 24. POST /bot-agent/messages/channel — botAgentSendChannelMessage
- 校验 `team_id`/`channel_id`/`message` 非空。App: `SendBotAgentChannelMessage(botUserId, teamID, channelID, message, props, userSnapshot)`。
- DB 表: channel / post。触发 WS: `posted` + channel_event。请求/响应同 #23。

### 25. GET /bot-agent/teams/{team_id}/members — botAgentGetTeamMembers
- Query `?page=&per_page=`（per_page 默认/上限 200）。App: `GetTeamMembersForBotAgent(teamID, page, perPage) ([]*entity.BotAgentMemberInfo, error)`
```go
type BotAgentMemberInfo struct {
	UserID      string `json:"userId"`
	DisplayName string `json:"displayName"`
	Nickname    string `json:"nickname"`
	Email       string `json:"email"`
	Role        string `json:"role"`
}
```

### 26. GET /bot-agent/teams/{team_id}/users/{user_id} — botAgentGetUser
- App: `GetUserForBotAgent(teamID, userID)`。响应 data: 单个 `BotAgentMemberInfo`（脱敏用户信息）。

### 27. GET /bot-agent/teams/{team_id}/channel — botAgentGetTeamChannel
- App: `GetDefaultChannelForBotAgent(teamID)`。响应 data: 默认 channel 信息。

---

## 28-34. /api/cses/agents/* （AI Agent 会话编排，session 鉴权）

App 方法在 `channels/app/cses_agent.go`；DB 表 `agent_session`、`agent_event`、`bot`、`cses_channel`。

### 28. POST /agents/create — createAgent
- App: `CreateAgent(*entity.CreateAgentRequest, sessionUserId) (*entity.CreateAgentResponse, error)`。表 `bot`。
```go
type CreateAgentRequest struct {
	Name        string `json:"name"`
	Type        string `json:"type"`
	Description string `json:"description,omitempty"`
	TeamId      string `json:"teamId"`
	OwnerUserId string `json:"ownerUserId,omitempty"`
}
type CreateAgentResponse struct {
	ID          string `json:"id"`
	UserID      string `json:"userId"`
	Name        string `json:"name"`
	Type        string `json:"type"`
	Description string `json:"description,omitempty"`
	TeamID      string `json:"teamId"`
	OwnerUserID string `json:"ownerUserId,omitempty"`
}
```

### 29. POST /agents/join-channel — joinAgentChannel
- App: `JoinAgentToChannel(*entity.JoinAgentChannelRequest, sessionUserId) error`。表 cses_channel / channelmember。
```go
type JoinAgentChannelRequest struct {
	Type      string `json:"type"`
	ChannelID string `json:"channelId"`
	TeamID    string `json:"teamId"`
}
```

### 30. POST /agents/start-session — startAgentSession
- App: `StartAgentSession(*entity.StartAgentSessionRequest, sessionUserId) (*entity.StartAgentSessionResponse, error)`。表 `agent_session` + cses_channel。
```go
type StartAgentSessionRequest struct {
	SessionID        string   `json:"sessionId,omitempty"`
	TaskName         string   `json:"taskName,omitempty"`
	TeamID           string   `json:"teamId"`
	StarterUserID    string   `json:"starterUserId,omitempty"`
	ResultChannelID  string   `json:"resultChannelId"`
	BackstageChannel string   `json:"backstageChannelId,omitempty"`
	AgentTypes       []string `json:"agentTypes"`
}
type StartAgentSessionResponse struct {
	SessionID         string   `json:"sessionId"`
	TraceID           string   `json:"traceId"`
	TeamID            string   `json:"teamId"`
	ResultChannelID   string   `json:"resultChannelId"`
	BackstageChannel  string   `json:"backstageChannelId,omitempty"`
	BoundAgentTypes   []string `json:"agentTypes"`
	InitialSessionKey string   `json:"initialSessionKey"`
}
```

### 31. POST /agents/send — sendAgentMessage
- App: `SendAgentMessage(*entity.SendAgentMessageRequest, sessionUserId) (*entity.SendAgentMessageResponse, error)`。表 `agent_event`（EventType="SEND_TO_AGENT"）。
```go
type SendAgentMessageRequest struct {
	SessionID        string `json:"sessionId"`
	AgentType        string `json:"agentType"`
	Text             string `json:"text"`
	ConversationType string `json:"conversationType,omitempty"`
	ChannelID        string `json:"channelId,omitempty"`
	UserID           string `json:"userId,omitempty"`
}
type SendAgentMessageResponse struct {
	Accepted bool   `json:"accepted"`
	EventID  string `json:"eventId,omitempty"`
	Message  string `json:"message,omitempty"`
}
```

### 32. POST /agents/callback — agentCallback
- App: `HandleAgentCallback(*entity.AgentCallbackRequest) (*entity.AgentCallbackResponse, error)`。表 `agent_event`（EventType="CALLBACK"）+ post（CreateCsesPost）。
- **触发 WS**: `posted`（cses_agent.go L406-408 `a.Publish`）+ channel_event。
```go
type AgentCallbackUsage struct {
	Model            string  `json:"model,omitempty"`
	PromptTokens     int64   `json:"promptTokens,omitempty"`
	CompletionTokens int64   `json:"completionTokens,omitempty"`
	TotalTokens      int64   `json:"totalTokens,omitempty"`
	Currency         string  `json:"currency,omitempty"`
	Amount           float64 `json:"amount,omitempty"`
}
type AgentCallbackRequest struct {
	MessageID    string                 `json:"messageId"`
	SessionID    string                 `json:"sessionId,omitempty"`
	TraceID      string                 `json:"traceId,omitempty"`
	SpanID       string                 `json:"spanId,omitempty"`
	ParentSpanID string                 `json:"parentSpanId,omitempty"`
	AgentType    string                 `json:"agentType,omitempty"`
	BotUserID    string                 `json:"botUserId,omitempty"`
	Stage        string                 `json:"stage,omitempty"`
	To           string                 `json:"to,omitempty"`
	Text         string                 `json:"text"`
	ReplyToID    string                 `json:"replyToId,omitempty"`
	ThreadID     string                 `json:"threadId,omitempty"`
	Commands     []string               `json:"commands,omitempty"`
	Usage        AgentCallbackUsage     `json:"usage"`
	Routing      BotCallbackRoutingInfo `json:"routing"`
}
type AgentCallbackResponse struct {
	Success bool   `json:"success"`
	Message string `json:"message,omitempty"`
	EventID string `json:"eventId,omitempty"`
}
```

### 33. GET /agents/session/{sessionId}/timeline — loadAgentTimeline
- App: `LoadAgentTimeline(sessionID) (*entity.AgentTimelineResponse, error)`。表 `agent_event`。
```go
type AgentEventView struct {
	ID         string                 `json:"id"`
	SessionID  string                 `json:"sessionId"`
	TraceID    string                 `json:"traceId,omitempty"`
	SpanID     string                 `json:"spanId,omitempty"`
	ParentSpan string                 `json:"parentSpanId,omitempty"`
	SourceType string                 `json:"sourceType,omitempty"`
	SourceID   string                 `json:"sourceId,omitempty"`
	EventType  string                 `json:"eventType,omitempty"`
	Stage      string                 `json:"stage,omitempty"`
	ChannelID  string                 `json:"channelId,omitempty"`
	PostID     string                 `json:"postId,omitempty"`
	OccurredAt int64                  `json:"occurredAt"`
	Payload    map[string]interface{} `json:"payload,omitempty"`
}
type AgentTimelineResponse struct {
	SessionID string           `json:"sessionId"`
	Events    []AgentEventView `json:"events"`
}
```

### 34. GET /agents/session/{sessionId}/cost — loadAgentCost
- App: `LoadAgentCost(sessionID) (*entity.AgentCostResponse, error)`。表 `agent_event`（聚合 usage）。
```go
type AgentCostResponse struct {
	SessionID       string  `json:"sessionId"`
	TotalPrompt     int64   `json:"totalPromptTokens"`
	TotalCompletion int64   `json:"totalCompletionTokens"`
	TotalTokens     int64   `json:"totalTokens"`
	TotalAmount     float64 `json:"totalAmount"`
	DefaultCurrency string  `json:"defaultCurrency"`
}
```

---

## 35-37. /api/cses/webhook/* （Webhook 分发配置）

注册在 `BaseRoutes.APIRoot`（`webhook_dispatcher.go`）。**响应不走 dto.CommonRes**，用裸 `map[string]interface{}{"success":..., "message":...}`。App 方法在 `channels/app/webhook_dispatcher.go`，配置存内存/配置态（无独立 ent 表）。

### 35. POST /webhook/config — setWebhookConfig
- App: `SetWebhookConfig(*entity.WebhookConfigRequest) error`
```go
type WebhookConfigRequest struct {
	TargetURL     string   `json:"targetUrl"`
	Secret        string   `json:"secret"`
	BotUserIds    []string `json:"botUserIds"`
	TriggerMode   string   `json:"triggerMode"`
	ChannelFilter []string `json:"channelFilter,omitempty"`
}
```
- 响应: `{"success": true, "message": "Webhook config updated"}`。

### 36. GET /webhook/config — getWebhookConfig
- App: `GetWebhookConfig() *entity.WebhookConfig`。secret 脱敏（前 4 字符 + `****`）。
- 响应 (configured 时，inline map):
```go
map[string]interface{}{
	"configured":    true,
	"enabled":       cfg.Enabled,
	"targetUrl":     cfg.TargetURL,
	"secret":        maskedSecret,
	"triggerMode":   cfg.TriggerMode,
	"botUserIds":    cfg.BotUserIds,
	"channelFilter": cfg.ChannelFilter,
}
```
- 未配置时: `{"configured": false}`。
- 关联 model `entity.WebhookConfig{ TargetURL, Secret, BotUserIds[], TriggerMode, ChannelFilter[], Enabled }`（全 camelCase）。

### 37. POST /webhook/test — testWebhook
- 请求 body 可选: `WebhookTestRequest{ Message string \`json:"message,omitempty"\` }`。
- App: `TestWebhook(message string) error`（HTTP 出站到 OpenClaw）。响应: `{"success": true, "message": "Test webhook sent successfully"}`。

> 关联出站载荷 `entity.WebhookPayload`（CSES→OpenClaw，verbatim，全 camelCase）:
```go
type WebhookPayload struct {
	Event            string   `json:"event"`
	PostId           string   `json:"postId"`
	ChannelId        string   `json:"channelId"`
	UserId           string   `json:"userId"`
	Username         string   `json:"username"`
	Message          string   `json:"message"`
	ThreadId         string   `json:"threadId,omitempty"`
	ReplyToId        string   `json:"replyToId,omitempty"`
	MediaUrls        []string `json:"mediaUrls,omitempty"`
	TeamId           string   `json:"teamId"`
	Timestamp        int64    `json:"timestamp"`
	Mentions         []string `json:"mentions"`
	ConversationType string   `json:"conversationType"`
}
```

---

## 38-40. /api/cses/channels/{csesCrossRepoDemo,crossRepoCounterPoll,triggerMmToCsesCounterAck} （跨仓 trace demo）

> ⚠️ **非业务接口**：GitNexus 跨仓爆炸半径 trace demo（`cross_repo_demo.go` 顶部注释）。**迁移时应排除**——这是为 /observe trace 评分制造跨仓 span 的合成 handler，故意走非存在 ID/channel 报错。注册到 `BaseRoutes.Channels`。

### 38. POST /channels/csesCrossRepoDemo — csesCrossRepoDemo
- 链路: `executeCrossRepoDemoChain` → `app.ReadMember` / `app.ReadDeptPositions` / `app.ReadOrganizes`（grpc 调 cses CompanyController）。
- 响应 (inline map): `{"members":int, "deptPositions":int, "organizes":int, "note":string}`。错误返 502。

### 39. POST /channels/crossRepoCounterPoll — crossRepoCounterPoll
- 链路: `executeCounterPollChain` → `loadCounterMetrics`（故意 channel not found）。Query `?channelId=`。
- 响应 (inline map): `{"channelId", "metricCount", "windowMillis", "note"}`。故意返 502。

### 40. POST /channels/triggerMmToCsesCounterAck — triggerMmToCsesCounterAck
- 链路: `executeMmTriggerCounterAckChain` → `invokeCsesTriggerCounterPoll` → `app.GrpcInvoke(CsesGrpc, "/Demo/triggerCrossRepoCounterPoll")`。Query `?channelId=`。
- 响应 (inline map): `{"channelId", "csesPayloadB", "note"}`。

---

## 触发的 WS / MQ 事件汇总（供 WS 追踪 agent 交叉核对）

| 事件名 | 常量 | 触发接口 | 路径 |
|---|---|---|---|
| `posted` | `model.WebsocketEventPost` | botCallback(#2) / botAgentSendDirectMessage(#23) / botAgentSendChannelMessage(#24) / agentCallback(#32) | 直接 `a.Publish` 或经 `CreateCsesPost` 共享路径 |
| channel_event（非 WS，append-only event log）| — | 凡走 `CreateCsesPost` 的 #2/#23/#24/#32 | 共享 post 落库路径写 channel_event（离线 sync cursor） |
| `bot_added`（webhook，非 WS 广播）| `entity.BotAgentEventBotAdded` | botAgentCreateConfig(#12) 异步 OnBotAgentConfigCreated | HTTP 出站到 agent webhook url |
| `BotAgentWebhookEvent`（Pulsar topic，非 WS）| — | 群聊消息触发 `PublishBotAgentWebhookEvent`（消息侧 hook，非本域 handler 直接调） | Pulsar producer，consumer 异步分发 webhook |

> 说明: 本域 handler **没有**新增专属 WS 事件常量（`cons/fields.go` 中无 bot/agent 专用事件）；所有实时推送复用标准 `posted` 事件。bot_added/bot_removed 与 BotAgentWebhookEvent 是 **HTTP/Pulsar webhook** 出站（推给外部 agent），不是 mattermost WS hub 广播。
