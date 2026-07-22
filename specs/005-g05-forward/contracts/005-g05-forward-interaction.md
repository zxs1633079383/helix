# G05 逐条与合并转发 Interaction Contract

## Status

- **CURRENT**: 仅保留源码与参考文档已证明的 live 能力；不得由类型存在推断。
- **TARGET**: `forwardMessages` 的 ActionBinding -> Tick -> authority -> PersistOk -> Frame -> PC apply 全链。
- **GAP**: 未通过 focused contract/replay 和 017 真链证据的节点保持 GAP。
- Gate 特有裁决: individual 与 merged 是不同 render-ready content；每个 target 独立终态，partial 不得升级为 all-success。

## Invocation

```text
InteractionFrame.ActionBinding(forwardMessages)
  -> ActionInvocationEnvelope(actionRef, actor, scope, epoch, revision, input)
  -> Tick::Command
  -> capability/input validation
  -> Effect
```

平台只能提交 binding 声明的 input；不得生成 actionRef、actor、authority revision 或服务端派生字段。

## Authority and Persistence

```text
Effect accepted
  -> authoritative HTTP query data | WS | sync
  -> normalize ForwardBatch
  -> PersistAtomic(fact + viewer indexes + cursor/eventSeq)
  -> PersistOk(correlation)
```

Mutation 的 HTTP SUCCESS 只结束请求层，不建立业务成功。查询 data 也必须校验 audience/scope 并先持久化。

## Frame and UI

```text
PersistOk
  -> SurfaceComposer(attached affected slots)
  -> InteractionFrameV2(complete VM rows, monotonic revisions, matching causation)
  -> Tauri typed batch
  -> Angular SurfaceRuntime whole-frame apply
  -> DOM/ARIA/interaction
```

不得从 raw WS、HTTP response、SQLite 查询或另一个 Surface 在 Angular 端重算业务结果。

## Failure Contract

| Failure | Required result |
|---|---|
| stale/cross-scope capability | rejected receipt；零 HTTP/Persist/business Frame |
| authority reject/timeout | 保留旧 VM；request-scoped failure |
| PersistErr | 不发布成功 Frame；进入可恢复失败 |
| duplicate/late fact | write-set 与 Frame mutation 均幂等 |
| baseRevision/epoch gap | 整帧拒绝并请求对应 snapshot |

