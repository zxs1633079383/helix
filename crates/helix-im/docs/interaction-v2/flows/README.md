# GS01-GS04 / G01 运行、证据与人工介入手册

> 状态：运行手册 / Review Draft。它服务于 `InteractionFrame v2` 实现后的验证、故障定位和证据封存；不是“已经实现”的声明，也不是用日志代替回放、测试或真 Go 验收的捷径。

流程图：

- [GS01-GS04 离线同步](./01-GS01-GS04-离线同步流程图.mmd)
- [G01 在线发送消息](./02-G01-在线发送消息流程图.mmd)

上位合同：[同步合同](../01-GS01-GS04-同步到Frame完整时序与数据.md)、[发送合同](../02-G01-文本发送到Frame完整时序与数据.md)、[Frame Cache 合同](../03-FrameCache原子替换与三端渲染合同.md)。本手册不更改其中的领域语义。

## 1. 什么时候执行

| 场景 | 执行范围 | 能否宣称 Gate 完成 |
|---|---|---|
| 仅文档或流程图变更 | Mermaid/链接/术语自审。 | 否。 |
| 修改 GS01-GS04、G01、Frame Cache、action dispatch、persist barrier、HTTP/WS adapter 或生成 wire | 对应单 Gate 回放、Frame fixture、PC/Mobile apply；必要时真 Go。 | 仅该 Gate 的代码/合同层可候选完成。 |
| Helix、PC、Mobile 任何一端 pin/schema/decoder 变更 | 至少重跑该 Gate 的同一 golden Frame 和跨端 apply。 | 否，除非重新取得所有要求的证据。 |
| 线上/预发发现重复行、未读错、发送卡 Sending、sync gap、actionRef stale | 按本手册采集一次有 correlation 的运行证据，再定位到流程图的第一个偏离点。 | 否；先标 `FAIL` 或 `BLOCKED`。 |
| 尚未落地 V2 producer 或 consumer | 只可进行旧链现状观察与差异取证。 | 否。V2 必须标 `planned`，不可把 raw emit 成功当作 Frame v2 成功。 |

每次运行先建一张 run card，至少固定：`runId`、Helix/PC/Mobile/Go 的 commit 或 pin、actor A/B、team/tenant、测试 channel、session epoch、`requestId`、`transactionId`、`temporaryId`、server message id、channel event sequence。所有日志和查询只围绕这些键收集。

## 2. 要产出的交互物与证据包

### 2.1 GS01-GS04

| 层次 | 必须看见的产物 |
|---|---|
| 输入 | `Tick::Connected` / hello、sync notify、pong gap 或 unknown channel HostObservation。 |
| Effect | local cursor/coverage scan；`LoadIncrement`、`SyncNotify` 或 channel hydration HTTP；必要的 recovery/reset plan。 |
| 提交 | 一个可标识的 `PersistAtomic` 写集；`PersistOk` 前不推进 cursor、不发最终业务 Frame。 |
| 编排 | `SurfaceComposer` 依据 attach index 选择 dialog/unread/timeline/thread/summary/composer；无 attach 的 slot 不发 mutation。 |
| Frame | `InteractionFrame v2` 有 schema、transaction、runtime、typed mutation、revision/epoch；GS04 的 `summary + composer` 是同一 bundle。 |
| 三端消费 | Cache 整帧校验、原子 commit、`FrameApplied`，随后才有 directive/`DirectiveApplied`。 |

### 2.2 G01

| 层次 | 必须看见的产物 |
|---|---|
| 上行 | `message.composer` 的 `message.create` ActionBinding 和一个带 requestId 的 v2 invocation。 |
| 受理 | `InteractionActionReceipt` accepted/rejected；rejected 不可产生本地消息行。 |
| Sending | `PersistAtomic(temporary Message, PendingSend)` 成功后，产生 `delivery=Sending` 的 Frame；清编辑器 directive 必须在 Frame apply 后执行。 |
| 远端确认 | HTTP 2xx 只表示 transport 已受理；权威 WS echo 或 sync 事实才允许 canonical settle。 |
| Sent/Failed | canonical PersistOk 后同一逻辑行由 temporaryId settle 为 server id、`Sent`；失败则同一行变 `Failed`，只在允许时签发 row-scoped retry binding。 |
| 双用户 | 接收者有自己独立的 Persist/Frame：dialog、unread 和已 attach timeline/thread；发送者的 `Sending` 不得泄漏。 |

### 2.3 最小证据包

每个 run 输出以下逻辑文件或等价的可追溯记录；没有 correlation 的截图、散乱日志或单个 HTTP 200 不算证据。

```text
run-card.md                 runId、pins、测试账户、预期与判定
helix-hop.jsonl             Tick -> Effect -> PortReply/PersistOk -> Frame 的 corr_key 链
go-server.jsonl             对应 HTTP/WS/event-seq/idempotency 记录
pc-frame-cache.jsonl        decode -> validate -> atomic apply -> FrameApplied/directive
mobile-frame-cache.jsonl    与 PC 同一 fixture 或同一 wire 的 apply 记录（适用时）
sqlite-read.md              本地只读事实快照与查询条件
postgres-read.md            远端只读事实快照与查询条件
verdict.md                  PASS / FAIL / BLOCKED / INCONCLUSIVE 与缺口
```

日志不得包含 token、authorization header、全量消息正文、文件路径或无关用户数据；正文只保留用于关联的受控摘要/ID。

## 3. 读写权限与数据边界

### 3.1 当前可用 MCP 的真实能力

当前工具名带 `ro`，因此是**硬只读**，不是“默认可写”。

| 平面 | 可读 | 可写 | 用途与限制 |
|---|---|---|---|
| 本地 Helix SQLite | `mcp__helix_local_sqlite_ro__list_tenants`、`mcp__helix_local_sqlite_ro__list_tables`、`mcp__helix_local_sqlite_ro__query`。query 只接受单条 bounded `SELECT` / `WITH ... SELECT`，最多 500 行。 | 不可。 | 观察 `channel`、`channel_bitmap`、`channel_member`、`message`、`message_mention`、`message_viewer`、`tenant_meta` 等本地已提交事实。 |
| CSES pre PostgreSQL | `mcp__cses_pre_postgres_ro__list_tables`、`mcp__cses_pre_postgres_ro__query`。同样只接受 bounded `SELECT` / `WITH ... SELECT`，最多 500 行。 | 不可。 | 观察 `channel`、`channel_event`、`channel_event_head`、`channel_member`、`post`、`post_idempotency`、`delivery_outbox` 等服务端权威事实。 |
| Go server、Helix、PC/Mobile 日志 | 允许按 run card correlation 读取、过滤、封存。 | 不可修改、清空或轮转。 | 建立跨端 hop 链，而不是搜索“看起来像成功”的孤立文本。 |
| 真实业务状态 | 仅通过被验证的 `ActionBinding -> Tick -> Effect -> Persist` 产品链改变。 | 允许该产品链自身的正常写入。 | 不允许 SQL、MCP、脚本或客户端 store 直接伪造 message、cursor、receipt、Frame Cache 状态。 |
| fixture / 隔离测试数据库 | 仅在另一个明确授权的测试任务中写入。 | 条件允许。 | 必须独立 DSN/tenant、可清理、记录写集；不与本手册的只读观察混用。 |

直接写 SQLite 或 PG 会绕过 `PersistAtomic`、cursor 不变量、审计和跨端 Frame，故本手册把它判为无效证据；远端 PG 的 DDL/DML、批量导出、生产数据修复一律需要人工明确批准。

### 3.2 查询纪律

1. 先 `list_tables`，再按当前 schema 写 `SELECT`；不从旧文档猜字段。
2. 每条查询必须带 tenant/user/channel/message 或时间窗口，设置 `LIMIT <= 500`。
3. 保存查询文本、执行时刻、row count 和被遮蔽后的结果摘要；不要保存敏感正文。
4. 本地 SQLite 只说明“Persist 后已提交的本地事实”；PG 只说明“服务端当时可见的权威事实”；两者都不能单独证明客户端 Cache 已原子 apply。

## 4. 日志采集与无效噪音过滤

### 4.1 先锁定 correlation，再读取日志

优先键顺序：`runId`、`requestId`、`transactionId`、`sessionEpoch`、`channelId`、`temporaryId`、server message id、`event_seq`。ActionRef 只记安全哈希/短指纹，不记录完整 opaque 值。

| 站点 | 必须保留的有效 hop | 默认过滤掉 |
|---|---|---|
| Go server | HTTP route/outcome、idempotency、canonical post、channel event/event_seq、WS 投递结果。 | 其他用户、其他 channel、access log 静态资源、普通 heartbeat。 |
| Helix | inbound/command Tick、Effect 生成和 port reply、PersistAtomic/PersistOk、SurfaceComposer impact、Frame transaction。 | 无 correlation 的 poll、重复调度、历史 session、raw payload 全文。 |
| PC `cses-client` | invocation/receipt、v2 decode、slot validation、atomic cache commit、`FrameApplied`、directive result。 | Angular change-detection、DOM repaint、无关 route、旧 raw bus 的非本 run 日志。 |
| Mobile（适用时） | 与 PC 同一 wire 的 decode、slot validation、commit、ack/resync。 | Flutter layout/animation、无关 isolate、无关 socket keepalive。 |

旧 raw emit 仍存在的迁移阶段，只能作为**上游事实到达的辅助证据**。它不得被计为“业务 UI 已成功”，也不得代替 `InteractionFrame v2 -> Frame Cache -> FrameApplied` 这条证据链。

### 4.2 推荐的过滤顺序

```text
1. 用 runId/requestId 缩小窗口
2. 用 sessionEpoch + channelId 排除旧会话和其他频道
3. 用 temporaryId/serverId/event_seq 对齐发送或同步事实
4. 只保留每一跳首次状态变化和重复/乱序的异常样本
5. 以 transactionId 对齐 Helix Frame 与 PC/Mobile FrameApplied
```

禁止从全量 `tail`、全局关键字搜索或单端截图直接得出“成功”。出现多个 correlation 混入时，先把证据标 `INCONCLUSIVE`，重跑隔离 channel，而不是手工挑一段好看的日志。

## 5. GS01-GS04 验证器

**前提**：V2 producer、v2 decoder 和 Frame Cache 已实际接入目标端；否则本节只能形成差异报告，结论必须是 `BLOCKED: V2 path absent`。

| Gate | 通过条件 | 必须反证 |
|---|---|---|
| GS01 hello/初始增量 | hello 后完成 cursor scan、权威 increment、同批 `PersistAtomic`；`PersistOk` 后才有 dialog/unread snapshot 和 `runtime.initialSync=ready`。 | Persist 失败时 cursor 不前进、没有最终业务 Frame。 |
| GS02 离线补事件 | sync notify 的 affected cursor 拉回权威 facts；只更新实际 attach 且受影响的 slot。 | 不可由 notify/raw payload 直接拼 dialog/timeline Row。 |
| GS03 gap 补偿 | `runtime.recovery=syncing` 全局可见；已有 Cache 可读可操作；补偿 facts PersistOk 后才 `recovered` 和重组 Frame。 | 不可假装 cursor 追平、不可清掉其他频道有效 composer action。 |
| GS04 陌生频道 hydration | unknown channel 先有 Channel/Member/ViewerState 的提交；进入频道时 `summary + composer` 同一 Frame 原子 apply，随后才可拿到发送 actionRef。 | 不可插入孤儿 message、不可在 bundle 前发送、不可让客户端虚构频道行。 |

GS01-GS04 **同步成功**必须同时满足：

1. 所有输入事实都可由同一个 run card correlation 回溯；
2. SQLite 与 PG 的限定只读快照不矛盾，cursor 不越过缺失事实；
3. Helix 日志证明每个最终业务 Frame 在 `PersistOk` 后；
4. PC 与 Mobile（适用时）都证明同一 Frame 完整验证、原子 apply、`FrameApplied`；
5. UI 能看到全局同步状态，且 GS03 不冻结已有有效操作；GS04 bundle 前后 composer 行为符合合同；
6. 上表对应的至少一条反证已跑过且按预期失败。

## 6. G01 验证器

**前提**：发送者已通过 GS04 得到 `message.composer@Channel` 的 live `message.create` ActionBinding，且发送者/接收者使用隔离测试 channel。

| 阶段 | 成功判据 | 失败即为 |
|---|---|---|
| invocation | actionRef、actor、scope、epoch/revision、text codec 校验通过，receipt 为 accepted。 | 收到 accepted 但没有相应 pending correlation；或拒绝后仍出现业务行。 |
| Sending | temporary message 和 PendingSend `PersistOk` 后，发送者只出现一个 `Sending` 行；Frame apply 后才清可见编辑器。 | 本地手插 Row、Persist 前 Frame、clear directive 早于 FrameApplied。 |
| outbound | `Effect::Http(CreatePost)` 与 run card 的 request/temporary identity 对齐。 | 把 HTTP 2xx 当 `Sent`。 |
| settle | WS echo/权威 sync 的 canonical message 经过第二次 PersistOk，原逻辑行由 temporaryId settle 成 server id 且 `Sent`。 | 重复行、重复 settle、echo 早/晚导致错误状态。 |
| receiver | 接收者独立 Persist 后收到自己的 dialog/unread/attached timeline Frame。 | 把发送者的 Sending 或 Frame 复制给接收者。 |
| negative | actionRef stale、Persist fail、HTTP fail/timeout、duplicate echo 均触发合同规定的失败/幂等路径。 | raw emit 或 UI reducer 绕过 v2 Frame 令 UI 看似成功。 |

G01 **发送成功**必须同时满足：

1. 用户回车立即清可见编辑器，但 rejected 或 Persist fail 能恢复同一 request 的 recovery draft；
2. `Sending` 与 `Sent` 是两个由 PersistOk 驱动的 Frame，不以 HTTP 2xx 代替 echo/sync；
3. 一个逻辑消息最终只有一行，temporaryId/server id 只 settle 一次；
4. sender/receiver 的 Surface 变化都由各自 session 的 Frame Cache 原子 apply 得到；
5. Go、Helix、SQLite、PC（及 Mobile）的 correlation 可串成一条无断点 hop 链；
6. 至少执行 Persist fail 和 duplicate/late echo 两个反证剧本。

## 7. 统一验证器判定

| 判定 | 含义 | 后续动作 |
|---|---|---|
| `PASS` | 正向链和要求的负向反证都通过，全部必需证据面一致。 | 封存 evidence pack；仅将该 Gate 更新为候选完成，仍不外推其他 Gate。 |
| `FAIL` | 找到事实矛盾、顺序破坏、错误 Frame、跨端语义不一致或直接 raw reducer。 | 定位流程图的第一个偏离 hop，修复后从隔离 run 重跑。 |
| `BLOCKED` | V2 路径未接入、缺少授权环境/账户/日志/读权限、远端不可达。 | 明确 blocker，不以旧链成功替代。 |
| `INCONCLUSIVE` | correlation 混入、证据缺 hop、时间窗口不完整。 | 不猜结论；缩小 channel/session 后重跑。 |

验证器必须先失败后再证明自己能通过：例如注入 Persist failure 时不得有 business Frame，篡改 baseRevision 时 Cache 必须整帧拒绝。只跑正向 happy path 不足以宣称 GS 或 G01 成功。

## 8. 什么时候必须人工介入

1. 需要对远端 PG 或本地 SQLite 执行任何写、DDL、清理、修复、全表导出或超过 MCP bounded read 的操作；
2. 身份/租户/权限、生产数据、真实用户消息、token 或文件路径可能被暴露；
3. Go 的 canonical post/channel event 与 Helix Persist facts 矛盾，无法凭一次权威 resync 收敛；
4. Frame schema 未知、actionRef 校验失败、跨端 decoder 不一致，或 Cache 整帧拒绝后 snapshot resync 仍失败；
5. GS03 recovery 循环、cursor 持续倒退/跳跃、发送永远停在 Sending，或重复 echo 产生多行；
6. 需要改变 Gate 语义、Surface owner、Frame 原子边界、raw emit 下线策略或用户已拍板的 G03 deferred 范围。

人工介入时保留原始小窗口日志和只读查询，写清 `runId`、第一偏离 hop、已排除的噪音、是否可重现；不要先手工改库“救绿”。
