# G01 InteractionFrame V2 Contract

## Boundary

这是 G01 application-to-platform 业务边界，不是 DOM/component 合同；不允许 `SurfaceDataVm` 之类的 generic JSON bag。Rust/TypeScript codec 必须与此 discriminated 语义形状同步。公共 JSON wire 一律 camelCase；Rust 的 snake_case 字段只能是 serde 内部实现，不能泄漏为第二套 wire。

## Command ingress

```text
ActionInvocationEnvelopeV2 {
  schemaVersion: 2,
  requestId: RequestId,
  corrKey: CorrelationKey,
  actorId: ActorId,                   // trusted host identity
  surface: "message.composer",
  scope: Channel(ChannelId),
  surfaceEpoch: Epoch,
  observedRevision: Revision,
  invocation: {
    actionRef: OpaqueActionRef,
    inputContract: "im.message-content.v2",
    input: CreateMessageInputV2 {
      content: { kind: "text", schemaVersion: 2, text: DisplayText },
      reply: ReplyReference?
    }
  }
}
```

composer 提供的是 `message.create` capability；其 opaque binding 固定实际 action kind 为 `im.send-message`。invocation 不得重复或自造 action kind，也不得含 user id、alternate channel id、temporary id、permission、URL。host identity + binding 固定这些值。unknown schema、actor/scope mismatch、stale epoch/revision、invalid content/reply、expired binding 都 fail-closed。

## Immediate receipt

```text
ActionReceiptV2 {
  schemaVersion: 2,
  requestId,
  corrKey,
  status: accepted | rejected { code, recoveryDraft: DisplayText? }
}
```

`accepted` 只表示 action admitted + persist effect scheduled，不改变 delivery、不渲染 Row。`rejected` 恢复 request-scoped local draft，且没有 frame Row。

## G01 frames

```text
InteractionFrameV2 {
  schemaVersion: 2,
  transactionId,
  causation: { requestId, corrKey, kind },
  runtime: RuntimeStatusVm,
  mutations: [SurfaceMutationV2...],
  directives: [UiDirectiveV2...]
}
```

### `sending`

只在 temporary message + pending send write set 的 `PersistOk` 后产生。

- Required: `message.timeline@Channel` 中恰一条 `MessageRowVmV2 { identity: temporary, delivery: Sending }`。
- `message.left-dialog-list@Viewer` 仅在 committed preview/sort facts 变化时出现。
- `message.thread-detail@Channel/root` 仅对 attached reply 出现。
- directive 是 `ClearComposerDraft { directiveId, requestId }`，但仅成功 full-frame apply 后执行。

### `sent`

只在匹配 WS/sync canonical facts 的原子持久化 `PersistOk` 后产生。timeline 用 server id + `Sent` 替换 temporary identity，绝不能 append 第二行。dialog/thread 在适用时使用 canonical preview/order。

### `failed`

只在失败事实 durable 后产生，把同一逻辑 row 变为 `Failed { reason, retryable }`。只有 retryable 且 binding 仍合法时才附 row-local retry binding。Rejected 或 pre-Sending persist failure 不产生 Failed Row，只恢复 draft。

### No frame for HTTP accepted

HTTP 2xx 记录 transport accepted，delivery 保持 `Sending`，不得产生 `Sent` 或等价 visual business mutation。

## Frame Cache consumer rules

1. unknown schema/surface/scope/content kind、invalid epoch/revision/baseRevision/typed VM -> 拒绝完整帧。
2. 所有 mutations/runtime 在临时 cache copy apply；任一失败时没有 slot 改变。
3. 一次 commit 后 `FrameApplied { transactionId, corrKey }`，再按顺序执行 directives。
4. 每 directive 都产生 `DirectiveApplied { directiveId, outcome }`; directive failure 不 rollback Frame。
5. invalid frame 只请求受影响 known slot snapshot resync，绝不 replay raw event 或手插 row。

## Compatibility and hard cut

G01 migration 后 `im:post:sending`、`im:post:received` 等 raw business emit 是无效下行源。禁止 V2 consumer fallback 和 producer dual-write。G02-G16c 不由本合同自动获得覆盖。
