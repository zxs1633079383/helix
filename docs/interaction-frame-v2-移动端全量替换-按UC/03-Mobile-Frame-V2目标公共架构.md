# Mobile Frame V2 目标公共架构

```mermaid
flowchart TD
  Page["Venus QuickJS page"] --> View["im.surface.view / pending"]
  View --> Intent["host-owned ActionBinding invoke"]
  Intent --> SDK["MobileImSdk"]
  SDK --> Bridge["CoreBridge / helix-driver-ffi C ABI"]

  subgraph HelixInternal["Helix/Go authority internals：逐 Gate 细节复用 PC 文档"]
    Bridge --> Envelope["ActionInvocationEnvelope / HostObservation"]
    Envelope --> Tick["Command / Tick"]
    Tick --> Effect["Effect batch"]
    Effect --> Http["Go HTTP authority write/read"]
    Http --> Pg["remote PostgreSQL"]
    Pg --> Ws["authority WS / PortReply"]
    Ws --> Persist["PersistAtomic -> Helix SQLite -> PersistOk"]
    Persist --> Composer["SurfaceComposer"]
    Composer --> Frame["InteractionFrameV2 schemaVersion=2"]
  end

  Frame --> Batch["FFI on_domain_events_batch"]
  Batch --> Runtime["SurfaceRuntime decode + epoch/revision/causation validate"]
  Runtime --> Cache["atomic typed Surface VM / frame cache"]
  Cache --> View
  View --> Ui["Venus component / mobile UI effect"]
  Runtime --> Ack["FrameApplied / directive ack / action receipt / resync"]
  Ack --> Bridge
```

唯一依赖方向是 Mobile adapter -> Helix contract。Venus 不再维护业务 reducer；
`MobileImSdk + SurfaceRuntime + quickjs/bind_mobile_im` 是唯一 V2 runtime，`venus/` 只负责页面
生命周期、线程切换、QuickJS 注入和有界 batch 队列。

图中的 `HelixInternal` 是完整链路中的折叠边界，不表示 Mobile 重新实现这些逻辑。每个 Gate 的
Command、Tick、Effect、HTTP、WS、PG、SQLite、Persist 和 Frame 字段跳转见
[Helix/PC/Go 外联索引](./07-Helix-PC-Go外联索引.md)。
