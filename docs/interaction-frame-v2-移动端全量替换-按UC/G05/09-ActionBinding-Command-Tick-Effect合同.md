# G05 ActionBinding、Command、Tick 与 Effect

共享 Helix 完整合同见 [PC G05 Helix/Frame 合同](../../interaction-frame-v2-全量替换-按UC/G05/07-Helix命令与Frame合同.md)。

| 跳 | 本 Gate 值 | Mobile 职责 |
|---|---|---|
| Action/Observation | `ActionBinding(forward)` | 只转发 Frame grant 或 host fact |
| Command | `im_relay_messages` | 不由 JS 自行选择/拼装 |
| Tick | `Tick::Command/Inbound/PortReply/Timer` | FFI 只批量送入/回收，不执行决策 |
| Effect | `storage_read`、`http_write`、`projection` | C++ 不兑现业务 Effect |
| Persist barrier | `PersistAtomic -> PersistOk` | matching PersistOk 前不确认 UI |

## Mobile 上行 wire 附页

`ActionBinding` 必须从已提交 VM 原样读取：

```json
{
  "actionRef": "<opaque-grant>",
  "kind": "<closed-action-kind>",
  "inputShape": "<versioned-input-contract>",
  "surfaceEpoch": 42,
  "enabled": true,
  "disabledReason": null
}
```

Renderer 只填用户输入；受信 host 注入 actor、requestId 和 slot 版本：

```json
{
  "schemaVersion": 1,
  "requestId": "<host-unique-causation>",
  "actorId": "<trusted-session-actor>",
  "surface": "<binding-owner-surface>",
  "scope": {"kind": "<surface-specific>"},
  "surfaceEpoch": 42,
  "observedRevision": 7,
  "invocation": {
    "actionRef": "<same-opaque-grant>",
    "input": {"<contract-field>": "<user-input>"}
  }
}
```

`ActionReceipt {requestId,status,validationErrors?}` 只结束本地 admission overlay；业务 VM 只由
matching `causationId` 的 Frame 改变。`ActionInvocationEnvelope/v1` 是独立版本域，V1 Frame
清退不得删除它。字段真源见
[interaction_frame.rs](/System/Volumes/Data/workspace/rust/helix/crates/helix-im/src/interaction_frame.rs:254) 和
[SurfaceRuntime.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp:1417)。

Mobile 若发现 PC 合同与当前 Helix 类型不一致，记录 `CONTRACT_DRIFT` 并阻断实现。
