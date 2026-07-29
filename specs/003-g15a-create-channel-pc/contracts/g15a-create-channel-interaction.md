# G15a Create Channel Interaction Contract

## Surface Capability

`message.left-dialog-list@User` 的 `DialogListVm` 根级必须包含：

```json
{
  "actions": [
    {
      "actionRef": "opaque-host-issued-ref",
      "kind": "channel.create",
      "inputShape": "object"
    }
  ]
}
```

`actionRef` 绑定 actor、scope、epoch、revision；客户端不得自行生成或改写。

## Invocation Input

PC overlay 确认后提交：

```json
{
  "display_name": "项目群",
  "user_ids": ["user-a", "user-b"]
}
```

- 只允许这两个输入字段。
- creator 从可信 action actor 派生，不出现在 `user_ids`。
- 中间草稿、组织树、头像、角色、team 不进入 invocation。

## Canonical Outbound

Helix 严格映射为现有 command：

```json
{
  "type": "P",
  "user_ids": ["user-a", "user-b"],
  "display_name": "项目群",
  "force_create": true
}
```

wire adapter 保持现有服务端字段映射；本功能不新增 HTTP endpoint 或后端 DTO。

## Completion Sequence

```text
ActionInvocation
  -> Tick::Command(CreateChannel)
  -> Effect::Http(channel/create)
  -> AuthoritativeChannelCreated(HTTP | WS | sync)
  -> Effect::PersistAtomic(channel + members + viewer state)
  -> PortReply::PersistOk(corr)
  -> InteractionFrame(dialog insertion/update + applicable summary)
  -> Action terminal receipt / FrameApplied
```

禁止序列：

- HTTP 200 -> success Frame
- 多个独立 Persist Effect -> 同 tick success Frame
- legacy raw `im:channel:created` -> PC reducer -> dialog row
- PC 直接调用 create command 绕开 ActionBinding

## Failure Contract

| Failure | Shared result | PC result |
|---|---|---|
| stale/cross-scope capability | no HTTP Effect; rejected receipt | 保留 draft，提示刷新能力 |
| invalid name/members | no HTTP Effect; validation receipt | 保留 draft，聚焦可修正字段 |
| HTTP reject/invalid envelope | no success Frame; correlated failure | 解除 submitting，保留 draft |
| PersistErr | no success Frame; recoverable/NeedsHuman state | 不出现群聊行，显示关联错误 |
| duplicate authority | idempotent upsert; at most one Frame result | 不出现重复行 |

## Scope Isolation

- 创建者 Frame 可以携带 request causation id 并结束 action。
- 受邀成员在自己的 user scope 处理权威事实，形成自己的 row/unread/capability。
- 未参与用户无 storage result、Frame mutation 或 capability。
- surface 未挂载时允许只完成持久化；下次挂载从事实投影，不重发 create HTTP。

## Compatibility

- `DialogListVm.actions` 是 InteractionFrame schema 的显式扩展，Rust/TypeScript 合同与 fixture 必须同步。
- 旧客户端遇到不支持的新 schema 必须按现有版本策略 fail-closed，不得回退 raw reducer。
- 本合同不包含移动端 renderer、Web 专属 UI、私聊、开放频道、话题或治理动作。
