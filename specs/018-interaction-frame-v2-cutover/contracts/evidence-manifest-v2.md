# Contract: V2 Cutover Evidence Manifest

## Required Root Fields

```text
schemaVersion = 2
candidateId
runId
evidenceEpoch
helixRevision
clientRevision
matrixHash
fixtureHash
startedAt
endedAt
wallTimeMs
phase = V2OnlyFirstRun | PostDeletionRun
```

## Required Gate Facets

```text
gateId
actor.commandOrObservation
actor.http
actor.wsOrSync
observer.http
observer.wsOrSync
go.request
go.authority
pg.beforeAfterDelta
sqlite.beforeAfterDelta
helix.tickEffects
helix.persist
helix.frameV2
client.frameApplied
client.dom
client.visual444
client.visual678
noMock
codeReference
```

Gate 可声明某 facet 为 `not_applicable`，但必须给出机器可审查原因；缺失、空字符串或旧 run artifact 均不能替代。

## Counters

```text
v2Frames > 0
v1Frames = 0
rawBusinessFallbacks = 0
frameAppliedSuccess = committedV2Transactions
requiredFail = 0
requiredSkip = 0
```

## Database Evidence

- 查询必须包含 Gate identity、actor/observer、主键和运行时间窗口。
- 保存 query hash、参数摘要、行 identity 与 before/after delta；不保存凭据或无界正文。
- PG/SQLite 都只读；无权限时 Gate 为 BLOCKED。

### 25-Gate Database Contract

每行是一个独立机器 Gate。`actor=444`、`observer=678`；如 Scenario
交换发起者，artifact 必须记录实际角色映射，不能只记录账号。`before` 在 UI
动作前采集，`after` 在 Go authority、Helix `PersistOk` 和 FrameApplied 后采集。

<!-- gate-contract:G-S1 -->
| Gate | PG authority / key | SQLite authority / key | Required delta |
|---|---|---|---|
| G-S1 | `channel(channel_id)`、`channel_member(channel_id,user_id)`、`channel_event(channel_id,event_seq)` | `channel(channel_id)`、`channel_member(channel_id,user_id)`、`channel_event_cursor(channel_id)` | cold start 后本地 authority 与 PG 当前身份一致；cursor 单调不回退 |
| G-S2 | `channel_event(channel_id,event_seq)`、`post(channel_id,id)` | `channel_event_cursor(channel_id)`、`message(channel_id,id)` | offline window 的 event/message 增量一次提交，cursor 推进到 after authority |
| G-S3 | `channel_event(channel_id,event_seq)`、`post(channel_id,id)` | `channel_event_cursor(channel_id)`、`message(channel_id,id)` | gap 前后 event_seq 连续；重复恢复不重复插入 |
| G-S4 | `channel(id)`、`channel_member(channel_id,user_id)`、`post(channel_id,id)` | `channel(id)`、`channel_member(channel_id,user_id)`、`message(channel_id,id)` | unknown channel hydrate 后三类本地行同时可读 |
| G-01 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | 新 post identity 出现且 actor/observer 本地 canonical id 一致 |
| G-02 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | DOCUMENT post 与 attachment 元数据提交；不保存附件正文 |
| G-04 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | revoke false→true，observer 同一 post identity 收敛 |
| G-04a | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | offline revoke 恢复后同一 post 收敛，cursor 无跳跃 |
| G-05 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | 转发生成的新 canonical post identities 与目标 channel 对应 |
| G-06 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | quick-reply/reaction actor 集合 before→after 精确变化 |
| G-07 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | urgent 状态及确认/取消用户状态按操作收敛 |
| G-08 | `post_schedule(id,channel_id)` | `channel_schedule(id,channel_id)` | schedule row 由无→有，计划时间和 owner identity 匹配 |
| G-09 | `post_schedule(id,channel_id)` | `channel_schedule(id,channel_id)` | 同一 schedule 由 active→cancelled/removed，重复取消幂等 |
| G-10 | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | 首窗 identities 与 PG 有界窗口一致 |
| G-10a | `post(channel_id,id)` | `message(channel_id,id)` | older page 新增 identities，已有窗口不重复且顺序稳定 |
| G-11 | `post(channel_id,id)` | `message(channel_id,id)` | locate identity 在 PG/SQLite 存在并进入有界窗口 |
| G-12 | `post(channel_id,id,root_id)` | `message(channel_id,id)`、`reply_tree(root_id,id)` | reply/root/first-level identity 图 before→after 一致 |
| G-13 | `channel_member(channel_id,user_id)`、`channel_event(channel_id,event_seq)` | `channel_member(channel_id,user_id)`、`channel_event_cursor(channel_id)` | channel read cursor/unread before→after 单调收敛 |
| G-14 | `post(channel_id,id)`、`channel_member(channel_id,user_id)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | post read participant/status 增量精确匹配 actor |
| G-14a | `post(channel_id,id)`、`channel_event(channel_id,event_seq)` | `message(channel_id,id)`、`channel_event_cursor(channel_id)` | template received authority 只作用目标 post/recipient |
| G-15a | `channel(id)`、`channel_member(channel_id,user_id)` | `channel(id)`、`channel_member(channel_id,user_id)` | create channel 由无→有，owner/member 集合与 PG 一致 |
| G-15 | `channel_topic(channel_id,id)`、`post(channel_id,id)` | `channel(channel_id)`、`message(channel_id,id)`、`reply_tree(root_id,id)` | topic/root identity 由无→有且 observer 可读 |
| G-16 | `channel(id)`、`channel_member(channel_id,user_id)` | `channel(id)`、`channel_member(channel_id,user_id)` | close lifecycle before→after，成员可见性按 authority 收敛 |
| G-16b | `channel_member(channel_id,user_id)`、`channel_event(channel_id,event_seq)` | `channel_member(channel_id,user_id)`、`channel_event_cursor(channel_id)` | leaving member 由 active→left/removed，其他成员不变 |
| G-16c | `channel(id)`、`channel_member(channel_id,user_id)` | `channel(id)`、`channel_member(channel_id,user_id)` | settings 仅允许字段 before→after 变化，成员集合不漂移 |

机器校验：

```bash
python3 scripts/verify-interaction-frame-v2-evidence-manifest.py
```

校验必须确认 25 个 Gate 各出现一次、G03/G03a 不出现，并且每行同时声明
PG、SQLite、identity key 与 delta。

## Artifact Isolation

`PostDeletionRun` 必须使用不同 candidateId、runId、evidenceEpoch 和 artifact root。Reducer 遇到删除前路径或 hash 时 fail-closed。

## Privacy And Bounds

- 允许：Gate/actor/observer、表名、列名、主键值 hash、行数、布尔/枚举状态、
  cursor/event_seq/revision、query hash、时间窗和 artifact hash。
- 禁止：token、cookie、Authorization、DSN、密码、消息正文、附件正文、完整 VM、
  未分页行集和原始 WS/HTTP body。
- PG 单查询 `limit <= 500`；SQLite 单查询 `limit <= 100`。超界、无 `WHERE`
  identity 或缺时间窗均视为 `BLOCKED`。
- before/after artifact 必须分别 hash；只有 after 结果、HTTP 200 或 WS 收到不能替代 delta。
