# Data Model: G15a PC 创建群聊

## Entities

### CreateChannelCapability

- `action_ref`: opaque、短生命周期、单 surface epoch/revision 有效
- `kind`: 固定 `channel.create`
- `actor_id` / `scope`: 绑定当前登录用户与 `message.left-dialog-list@User`
- `input_shape`: 接受规范化对象，不包含 creator/team/role/avatar
- Invariant: capability 不匹配、过期或跨用户时 fail-closed

### PcCreateChannelDraft

- `display_name`: PC 本地字符串，提交前可为空；规范化后 1..30 字符
- `selected_user_ids`: PC 本地候选成员集合
- `submitting`: PC 本地 UI 状态
- `error`: PC 本地、与当前 request 关联的可操作错误
- Invariant: 不是共享持久事实，不进入 Helix storage 或 Frame cache

### CreateChannelRequest

- `request_id`: host/action invocation 注入的幂等身份
- `creator_id`: 从可信 actor/session 派生
- `display_name`: trim 后的合法名称
- `member_ids`: 排除 creator、空值和重复后的有序唯一集合
- `channel_type`: 固定私有群聊 `P`
- `force_create`: 固定为现有 wire 规则
- Invariant: canonical outbound 只含 `type,user_ids,display_name,force_create`

### AuthoritativeChannelCreated

- `channel_id`: 服务端权威唯一身份
- `display_name` / `channel_type` / `creator_id`: 服务端确认属性
- `members`: 权威或由已确认回复补齐的规范成员集合
- `source`: HTTP reply、WS 或 sync，仅用于诊断，不改变业务语义
- `request_id?`: 创建者链可关联；受邀者/恢复链可为空
- Invariant: 不因 transport 200 自动构造缺失的频道身份

### ChannelCreatedCommit

- `corr`: 等待 storage PortReply 的关联身份
- `channel_id` / `viewer_id`
- `ops`: channel upsert + member upserts/deletes + viewer channel state
- `causation_id?`: 创建者 action request；其他用户可为空
- Invariant: 作为单个 `PersistAtomic` 发出，禁止拆成可见性前的独立写入

### UserScopedChannelResult

- `viewer_id` / `surface_scope`
- `channel_id`
- `dialog_row` / applicable unread summary / action bindings
- `revision` / `epoch`
- Invariant: 每个 viewer 独立计算；未参与用户不存在该结果

## State Machine

```text
Drafting
  -> Submitting(request_id)
  -> AwaitingAuthoritativeFact(request_id)
  -> Persisting(corr, channel_id)
  -> Visible(channel_id)

Invalid/Stale input -> Drafting + rejected receipt
HTTP/authority error -> RecoverableFailure -> Drafting
PersistErr -> RecoverableFailure/NeedsHuman, no Visible transition
Detached PersistOk -> PersistedDetached -> Visible on next mount
Duplicate authoritative fact -> current state idempotent self-loop
```

## Identity and Uniqueness

- Action submission uniqueness: `(actor_id, scope, request_id)`.
- Durable channel uniqueness: `channel_id`.
- Member uniqueness: `(channel_id, user_id)`.
- Per-user visible result uniqueness: `(viewer_id, surface_scope, channel_id)`.
- `display_name` 不是 identity；同名群允许存在。

## Validation Rules

- 名称 trim 后 1..30 字符。
- 至少一名不同于 creator 的非空成员。
- 成员去重并确定性排序后再构造 effect。
- SurfaceData 数组继续受现有 bounded validation 限制；不新增无界集合。
- 未知字段、错误类型、stale action、scope/actor 不匹配均在 HTTP Effect 前拒绝。
- `PersistErr` 不得改变 Frame cache 或发布 success directive。

## Complexity

- 名称校验 `O(n)`，其中 `n <= 30`。
- 成员归一化、存储 op 生成 `O(m log m)`（确定性排序）与 `O(m)` 空间；`m` 受输入集合上限约束。
- channel/member 持久化为一个 batch；无 read-modify-write 循环。
- Dialog patch 复用已挂载的有界窗口算法；不做跨 surface 全量 join。
