# G01 完整验收 Handoff

> 更新时间：2026-07-24。本文是 Helix G01 Frame producer 与 cses-client MessageV4
> WDIO consumer/真实链的跨仓交接入口。它描述完成定义、当前代码事实和下一步，不把
> Fixture、Mock、历史运行或命令受理升级成真实链 Green。

## 1. 目标与范围

G01 是“创建本次测试专用群聊后发送普通文本消息”的完整桌面验收：

```text
Actor 444 通过 MessageV4 创建群聊并选择 Observer 678
→ 服务端确认新 channelId 和成员权威事实
→ 444/678 左侧会话均出现新群
→ 444 在新群发送唯一 marker
→ Actor timeline: sending → sent
→ Observer timeline: sent
→ Actor/Observer DialogLeft: preview + lastActivityAt 更新
→ Command/Persist/HTTP/WS/Frame/FrameApplied/DOM 完成 correlation
```

本轮明确不包含 `GS01-GS04`。创建群聊是 G01 的 required prerequisite，不再作为
本次 WDIO 的独立 GS Gate，也不能通过复用既有共同频道跳过。

## 2. 架构边界

完整 G01 Green 是下列证据面的交集：

```text
Helix producer contract
∩ cses-client Fixture parity
∩ WDIO IPC Mock consumer
∩ 真实 Tauri invoke
∩ 444/678 双实例真实链
∩ FrameApplied + MessageV4 DOM
∩ 跨层 correlation
∩ 阶段化机器报告
```

各层职责：

| 层 | 证明内容 | 不允许声明 |
|---|---|---|
| Helix replay | Tick/PortReply/Inbound 到 Effect、Persist barrier、完整 V2 Frame | Tauri、Go、SQLite 执行器或 DOM 已通过 |
| IPC Mock | 完整 Helix Fixture 能经真实 `im:__bus__` 被 SurfaceRuntime apply 并渲染 | Rust/SQLite/HTTP/WS 已执行 |
| real invoke | `window.__TAURI__.core.invoke` 真正进入 Tauri/Helix | Observer、WS、PG 或完整 G01 已通过 |
| dual real | 两个隔离实例完成 Go/PG/SQLite/WS/Frame/DOM 事实闭环 | 未纳入本次 run 的失败分支已通过 |

`AcceptedCommand` 只表示 command queue admission。Fixture 不是 command 同步返回值；
`TauriMockBridge.replay()` 必须把完整 `InteractionFrameV2` 注入生产 bus ingress，
Angular 不得手写或补算 Helix 业务 ViewModel。

## 3. G01 Frame 模型

当前 canonical Fixture 真源：

```text
crates/helix-im/tests/fixtures/g01-wdio/
├── group-created.frame-v2.json
├── sending.frame-v2.json
├── sent.frame-v2.json
├── failed.frame-v2.json
├── ws-timeline-sent.frame-v2.json
├── ws-left-dialog-list.frame-v2.json
└── provenance.json
```

唯一 producer 是：

```text
G01ScenarioRunner
→ ImModule::handle
→ Effect::Emit
→ InteractionFrameV2::decode
→ typed validate
→ 字段级语义断言
→ canonical JSON
→ Golden compare / explicit update
```

普通测试只能比较 actual 与 checked-in Golden。仅下面的显式命令允许更新：

```bash
UPDATE_G01_WDIO_FIXTURES=1 \
cargo test -p helix-im --test g01_wdio_fixture_contract_test
```

真实顺序是部分有序，不是固定三帧数组：

```text
P1 message.left-dialog-list group-created 是发送前置
M1 message.timeline sending 必须先发生
M2 message.timeline sent/failed 属于 HTTP/terminal correlation
C1 message.left-dialog-list 属于 WS post/dialog correlation
M2 与 C1 可能竞态，不得断言 C1 固定为第三帧
```

WS 已完成权威结算后，迟到 HTTP success 不得再次 terminal persist 或重复产生终态
Frame。`failed` 是独立场景，不能和 `sent` 同时成为一次发送的 required 结果。

## 4. UI 与运行拓扑

Actor 与 Observer 必须是两个真实隔离实例：

| 角色 | 用户 | 必须隔离 |
|---|---:|---|
| Actor | 444 | profile root、auth、device、SQLite、WebDriver/日志端口、WS session |
| Observer | 678 | profile root、auth、device、SQLite、WebDriver/日志端口、WS session |

UI 操作必须走 MessageV4：

1. 444 打开创建群聊 UI。
2. 当前用户 444 保持隐式 owner，不作为 `user_ids` 重复提交。
3. 搜索并显式选择 678。
4. 点击创建，`im_create_channel.user_ids` 应为 `["678"]`。
5. 等服务端权威结果确认最终成员为 444 与 678。
6. 等双方 apply 对应 Dialog Frame，并出现同一个新 `channelId`。
7. 444 进入新群，输入本次唯一 marker 并点击发送。
8. 等 Actor `sending → sent`、Observer `sent` 及双方 DialogLeft 更新。

禁止测试直接 invoke `im_create_channel` 代替 UI，禁止直接修改 Angular store，禁止通过
既有 dialog ID 交集选择旧频道。

## 5. 统一阶段报告

G01 报告必须按以下 required stage 输出：

```text
prerequisite.identity
prerequisite.create-group-ui
prerequisite.channel-authority
prerequisite.actor-dialog
prerequisite.observer-dialog
g01.send-command
g01.actor-timeline
g01.observer-timeline
g01.actor-dialog-update
g01.observer-dialog-update
g01.correlation
```

每个 stage 至少包含：

```json
{
  "id": "g01.actor-timeline",
  "status": "pass|fail|blocked|skipped",
  "startedAt": "ISO-8601",
  "completedAt": "ISO-8601",
  "durationMs": 0,
  "expected": {},
  "actual": {},
  "assertions": [],
  "evidence": [],
  "blockedBy": null,
  "claimBoundary": "本阶段实际证明的边界"
}
```

所有 Gate 都复用这一报告形状，但只声明自己的业务阶段：

```text
prerequisite.*
gNN.command
gNN.<actor projection>
gNN.<observer projection>
gNN.<secondary surface>
gNN.correlation
```

不能把 G01 的 `sending/sent/failed` 机械复制给其他 Gate。每个 Gate 的 stage inventory
必须由其 command、authority、Surface 与终态语义决定。

## 6. 各阶段断言

### `prerequisite.identity`

- Actor 必须是 444，Observer 必须是 678。
- 两实例的 profile/auth/device/端口不得相同。
- 两个实例均完成 ready，连接状态可观察。

### `prerequisite.create-group-ui`

- 创建动作来自真实 DOM click/type/select。
- 678 被显式选择，444 为隐式当前用户。
- IPC 仅可把 command admission 标绿，不能提前标记频道创建成功。

### `prerequisite.channel-authority`

- 本次产生全新非空 `channelId`，不在运行前 dialog inventory 中。
- 服务端权威成员包含且仅包含预期参与者 444、678。
- HTTP/PG/WS/本地存储证据属于同一 `scenarioRunId`。

### `prerequisite.actor-dialog` / `prerequisite.observer-dialog`

- 双方收到对应 `message.left-dialog-list` Frame。
- Frame 有 matching `FrameApplied`，没有 `FrameRejected`。
- DOM 中存在同一 `data-dialog-id=channelId`。

### `g01.send-command`

- UI 发送产生 `im_send_message`，payload 含 `channel_id/type/text/req_id`。
- `AcceptedCommand.command` 与 `helixCommand` 均为 `im_send_message`。
- 本阶段只声明 accepted，不声明 persisted、sent 或 delivered。

### `g01.actor-timeline`

- `surface=message.timeline`。
- 同一 `temporaryId/channelId/marker/causationId` 先为 `sending`。
- sent 场景最终补齐正式 `postId/eventSeq` 并收敛为 `sent`。
- failed 场景收敛为 `failed`，不得同时要求 Observer 收到正式 post。
- 每个 required transaction 均有 matching `FrameApplied`。

### `g01.observer-timeline`

- Observer 通过真实 WS/恢复链收到同一 `channelId/postId/marker`。
- Observer 最终为 `sent`，不得出现 Actor 的本地 optimistic `sending`。
- Observer Frame 必须 apply 后才允许 DOM stage 标绿。

### `g01.actor-dialog-update` / `g01.observer-dialog-update`

- `surface=message.left-dialog-list`。
- 对应 row 的 preview 严格等于 Helix `simpleMessage`：本次完整 marker 的前 50 个
  Unicode 字符；完整文本只属于 timeline。
- `lastActivityAt` 对应本次正式消息时间并参与权威排序。
- unread/mention/urgent 按当前用户语义断言；自己发送后的 unread 必须等于发送前基线，
  不能错误要求绝对为 0，因为建群系统消息可能已形成未读。
- 不通过固定 Frame 序号或固定 sleep 等待。

### `g01.correlation`

至少关联并验证：

```text
suiteRunId
scenarioRunId
reqId
causationId
channelId
temporaryId
postId
eventSeq
trackId
各目标 transactionId
Actor/Observer FrameApplied
Actor/Observer DOM messageId
```

必须证明 `reqId → causationId`、`temporaryId → postId`、双方
`channelId/postId` 一致，并拒绝重复 terminal persist、重复终态 Frame 或无法归属本次
run 的历史日志。

## 7. WDIO 场景矩阵

| Lane | 场景 | Required |
|---|---|---:|
| Fixture contract | group-created/sending/sent/failed/WS timeline/WS dialog | 是 |
| Mock sent | Accepted + sending + sent + DOM | 是 |
| Mock failed | Accepted + sending + failed + DOM | 是 |
| Mock WS dialog | sending 后独立 replay WS timeline/dialog，二者不固定顺序 | 是 |
| Integration | 真实 Tauri invoke，证明 command 进入 Rust | 是 |
| Single real | 真实 Go/SQLite/PG/FrameApplied/Actor DOM | 是 |
| Dual real | UI 建新群、444/678 timeline/dialog/WS/双端 DOM | 是 |
| Controlled real failure | 真实故障注入后的失败闭环 | 未声明前不能用 Mock 代替 |

Mock failed 可以证明 Angular 消费 `failed` Frame；若报告声称“真实传输失败链 Green”，则
必须另建受控故障 lane，不能用 Mock 充当真实基础设施故障。

## 8. 日志与附件

同一个 run 目录至少保存：

- Browser console：UI action、IPC admission、FrameReceived、FrameApplied/Rejected、DOM latch。
- Tauri/Helix：command ingress、Tick、Persist/HTTP/Timer/Inbound、Effect::Emit、correlation。
- Go/PG/WS：HTTP track、PG commit、outbox/publish、recipient delivery、`postId/eventSeq`。
- Actor/Observer 终态截图；失败时保存诊断截图。
- JUnit/JSON stage report、source/process manifest、Fixture/provenance hash。

日志是断言的证据输入，不是 pass 的替代品。禁止保存 token、cookie、口令、完整敏感
payload；artifact redaction 失败必须使本次 run 失败。

## 9. Canonical Green 判定

只有同时满足以下条件才能报告 `G01 canonical Green`：

- 本次 run 的所有 required stage 都是 `pass`。
- required stage 不得 `blocked`；`skipped` 必须有领域上不适用的明确理由。
- Fixture producer、client mirror、provenance、raw/semantic hash parity 全部通过。
- Mock sent、Mock failed、Mock WS dialog 全部通过。
- 真实 invoke、single-real、dual-real 使用本次 source pin 通过。
- 双实例从 UI 创建新群，没有复用旧频道。
- Actor/Observer timeline 与 DialogLeft 均完成 Frame → FrameApplied → DOM。
- required correlation 闭合，无 FrameRejected、无重复 terminal、无固定 sleep。
- console/Tauri/Go/PG/WS/SQLite/截图/报告均属于同一 `suiteRunId`。
- 报告 source pin、运行二进制 pin、Helix Fixture producer pin 一致且可审计。

历史 Green、不同 pin 的结果、Mock pass 或单实例 pass 都不能填补本次 required stage。

## 10. 当前代码事实

截至 2026-07-24 本轮验证：

- Helix `G01ScenarioRunner` 先创建 444/678 群聊，再用该 `channelId` 执行 HTTP/WS
  发送场景；六份 canonical Frame 与 provenance 的普通 Golden 比较通过。
- Fixture 记录的生产语义 commit 为
  `ee8440e7b6e19d75153f46349e57fbb6c00ca8cf`。
- cses-client 已同步六份 Fixture；contracts 83/83、Mock 4/4、integration 2/2、
  single-real 1/1、dual-real 2/2 通过。
- single-real 已由 MessageV4 UI 创建群并选择 678，再在返回频道发送；1/1 通过。
- dual-real 已由 Actor 444 UI 创建群并选择 Observer 678，Observer 等待同一新
  `channelId` 后双端进入并发送/接收；11/11 required stage 全部 pass。Actor unread
  从建群后的 1 保持为 1，证明自发消息未增加自身 unread。
- Mobile 生产路径已编译期切断 V1 row producer/consumer，G01 C++/QuickJS 5/5 与
  Venus offline Gate 通过；两条 Mobile lane 均先提交选择 678 的建群 command，
  apply `group-created` 后再向同一 `channelId` 提交发送 command。
- Mobile `send/sendWithTrace` 只同步返回 Accepted，不再等待 raw
  `im:post:sending` receipt；delivery 只能由 V2 Frame 推进。
- 双端 dialog 的 `lastActivityAt` 均等于 PG 正式消息时间，完整 timeline marker 与
  50 字符 dialog `simpleMessage` 分别按 Helix 字段合同断言。
- dual-real 证据包 `dual-real-20260725054942765-38416` 已通过 12 文件 redaction 扫描。

因此当前准确状态是：三仓 Gate01 的 producer/consumer 合同、创建群前置、用户行为和
字段语义已经一致，本地 Gate01 验收 Green。发布级统一 suite 尚未把 Mock、single-real、
dual-real、Mobile 离线 Gate 汇入同一个 `suiteRunId`；也未执行真手机双账号或受控真实
网络失败，所以这些更高层声明仍不在本轮范围内。

## 11. 下一步实施顺序

1. 把各 lane 编排进同一个 `suiteRunId`，产出单一发布证据包。
2. 让 reporter 聚合 Helix、PC、Mobile 的 source/content pin，而不是只记录独立 run。
3. 若产品要求真实失败声明，新增可恢复的网络故障注入 lane；Mock failed 继续只证明
   consumer 合同。
4. 真机资源具备后补 Mobile 双账号真 Go 链；不得用 Venus offline Gate 代替。

## 12. 验证命令

Helix：

```bash
cd /System/Volumes/Data/workspace/rust/helix
cargo check -p helix-im
cargo test -p helix-im --test g01_full_flow_test -- --nocapture
cargo test -p helix-im --test g01_wdio_fixture_contract_test -- --nocapture
```

cses-client：

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
yarn message:wdio:contracts
yarn message:wdio:preflight
yarn message:wdio:boundary
yarn message:wdio:build
yarn message:wdio:g01:mock
yarn message:wdio:g01:integration
yarn message:wdio:g01:single-real
yarn message:wdio:g01:dual-real
yarn message:wdio:g01:report-contract
yarn message:wdio:artifact-redaction <suiteRunId>
```

不得跳过前序失败继续拼装“总体 Green”。任何真实鉴权、Go、PG、WS、双实例或 DOM
可观察能力缺失都应在对应 stage 标记 `blocked`，并保留证据后停止晋级。
