# Data Model: G15 话题与 G15a 复用

## Core Records

### CapabilityLease

- `action_ref`: host 签发的不透明引用
- `actor_id`、`scope`、`surface_epoch`、`revision`
- `kind`: `topic.create`
- Invariant: stale、cross-actor、cross-scope 或未知 input 一律在 Effect 前拒绝。

### TopicLink

- identity: Gate 对应实体主键与 viewer scope
- authority_revision: 服务端 revision/event identity/cursor
- correlation: requestId/trackId/corr_key/causationId
- source: HTTP query、WS 或 sync 中本 Gate 允许的权威来源
- Invariant: transport ack 不是 durable fact；重复/乱序输入不重复推进。

### PersistTransaction

- `correlation`: 唯一关联 PortReply
- `write_set`: 领域事实、viewer index、cursor/eventSeq 的原子集合
- `status`: requested | ok | err
- Invariant: `ok` 前不得合成业务成功 Frame，`err` 保留上一致 VM。

### RenderReadyProjection

- affected surfaces: 本 Gate spec 的 Surface 集合
- complete rows: 每个 mutation 都携带完整 typed VM/row
- revision tuple: surface/scope/baseRevision/revision/surfaceEpoch
- Invariant: whole-frame validate/apply；任一 mutation 非法则整帧拒绝。

## State Machine

```text
Absent -> Creating -> AwaitingAuthority -> Persisting -> Visible
stale/forbidden/invalid -> Rejected(no HTTP, no Persist, no business Frame)
PersistErr -> RecoverableFailure(previous render-ready projection remains authoritative)
duplicate/late authority -> idempotent self-loop
```

## Gate-Specific Invariants

- G15a 建群由 003 独占实现，本组只引用其完成证据；生产任务只实现 G15 topic.create 与关联 Surface。
- Action invocation identity 为 `(actor, scope, action_ref, request_id)`。
- entity/viewer lookup 目标为 O(1) 或 O(log n)；有界 compose 为 O(visible)。
- 平台 overlay、draft、scroll/highlight 动画不是共享持久事实。

## Validation

- canonical replay 的 write-set 与 Frame 必须确定性一致。
- 删除 PersistOk 或打乱 authority 次序时，测试必须可证伪。
- actor/observer 的投影按 viewer scope 隔离，禁止复用另一用户 Frame。

