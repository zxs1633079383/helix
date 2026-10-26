# UC01-16 跨端交互协议与呈现状态机 SPEC

> 状态：Validated Design / Implementation Pending
> 日期：2026-07-15
> 取代：`2026-07-15-uc1-16-cross-platform-render-only-projection-spec.md`
> 范围：Message V4 宏 UC01-16；Strict Matrix `G-01..G-16c` 与同步前置 Gate
> 合同真源：Strict Matrix contract epoch 4、Helix 当前领域/存储/投影代码；消费端证据：cses-client 与 mobile 当前代码；机制参照：Dioxus/Crux
> Review：`dioxus-dev` 与 `crux-dev` 两个独立 lens，结论已回填
> Architecture：ADR-015；本 SPEC 是 Helix 新版本多端统一 UI 交互目标，并已同步更新 `docs/GOAL.md` §6
> 边界：本文是经过代码查证的目标合同，不把当前 `declared/blocked` 误报为运行时 Green。
> 实施仓库：`helix`、`cses-client`、`mobile-qucik-c++`；`cses-im-server` 仅在后端权威合同缺口时条件介入。明确排除 `loopforge-tauri-im` 与 `pd/message`。

## 0. 拍板结论

旧 Spec 只把业务数据变成了 `RenderNode/RenderPatch`，但 UI 仍要理解 command 名、业务 ID、事件类型、未读规则、消息对账和操作完成条件。那不是纯渲染端，只是把三套业务引擎换成了三套投影解释器。

新的跨端边界必须是一个完整的双向交互协议：

```text
UiEvent / HostObservation
  -> Helix update
  -> Domain Model
  -> Persist / Transport / Timer Effect
  -> EffectResult / WS / Sync
  -> PersistOk
  -> InteractionFrame
       ViewMutation[]     可重放呈现状态
       ActionBinding[]    Helix 已裁决的可执行动作
       UiDirective[]      一次性跨端 UI 语义
  -> Angular / QuickJS / C++ / Flutter 机械 apply + render + ack
```

核心决策：

1. “UI 意图”只指上行 `UiEvent`，下行分别叫 `ViewMutation`、`ActionBinding` 和 `UiDirective`，不再混用 intent。
2. UI 不知道 `im_send_message`、`im_post_read`、`im_channel_close` 等命令名，也不拼 command payload。
3. UI 不生成 temporary ID，不维护 temporary-to-server map，不按 WS event 前缀猜实体变化。
4. Helix 可以输出平台无关的语义 view tree、renderer key、动作绑定和一次性 UI directive，但不输出 DOM、Angular Component、Flutter Widget、CSS selector、像素或动画实现。
5. 一个领域事实造成的跨实体 UI 变化必须放入同一 `InteractionFrame` 原子提交。
6. 可见业务状态必须经过 `PersistOk` 后发布；HTTP 200 不是发送完成，WS/sync 权威回声才是。

## 1. 真源与 UC-16 查证

### 1.1 权威顺序

发生语义冲突时按以下顺序停机对账，不由客户端代码反向定义合同：

1. `crates/helix-im/contracts/strict-matrix.v1.json`
2. `crates/helix-im/docs/Strict Matrix/UC全链路发布矩阵-v1.md`
3. Helix 当前实现与 `前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`
4. cses-client 与 mobile 当前代码只作为迁移证据，不作为目标架构真源
5. Dioxus 与 Crux 源码只证明机制可行性，不定义 IM 业务合同

`pd/message` 的历史 UI 材料可用于回看产品用例来历，但不再属于本 Spec 的实施仓库、现行合同真源或发布证据入口。

### 1.2 UC 编号不能再混

`UC01-16` 是产品宏用例编号，`G-01..G-16c` 是 Strict Matrix 发布 Gate。一个宏 UC 可以覆盖多个 Gate。

已查证：

- `G-13 / UC13`：会话已读，`channels/view`。
- `G-14 / UC13`：单条消息已读，`post/read`。
- `G-14a / UC14`：模板收到确认。
- `G-16 / UC16`：关闭群、主动退出群。
- `G-16b / UC16`：成员退出广播。
- `G-16c / UC16`：群属性修改。

所以 **UC-16 不是未读**。未读闭环属于 UC13 的 G-13/G-14；UC16 是群治理与生命周期。自退群和管理员移除成员也不是同一个动作，必须是不同 `ActionBinding`。

### 1.3 当前发布状态

Strict Matrix 当前仍以 `active/declared` 为主，G-03 媒体能力仍有 blocked/deferred 面。本文验证的是合同、代码现状和迁移边界，不替代真 Tauri/FFI、HTTP、WS、远端 DB 与投影证据。

## 2. 领域权威模型

### 2.1 四类状态必须分开

| 状态 | 权威归属 | 持久化 | UI 消费形态 |
|---|---|---|---|
| `DomainModel` | Helix | Helix Storage | 不直接暴露 |
| `SurfaceModel` | Helix，按 surface 隔离 | 可恢复或可由 DomainModel 重建 | `InteractionFrame` |
| `ViewModel` | Helix 投影 | 可丢弃快照/补丁 | `ViewMutation` |
| 平台瞬时状态 | 各端 shell | 不作为业务事实 | 焦点句柄、像素滚动、动画、原生文件句柄 |

`activeChannel` 不是全局单例。它属于某个 `surface_id + surface_epoch` 的交互上下文。两个窗口可以同时打开不同频道，手机页面重建后旧 viewport 事件不能污染新页面。

### 2.2 未读不是一个 bool

当前数据中至少有三个相关但不同的领域对象：

```text
ViewerChannelReadState
  key: account_id + channel_id
  source: current viewer 的 ChannelMember
  fields: unread_count, unread_post_id, last_read_seq,
          msg_count, msg_count_all, mention_count, urgent_count

ChannelDialogView
  key: surface/account + channel_id
  source: Channel 公共字段 JOIN/OVERLAY ViewerChannelReadState
  purpose: 会话列表 badge、最后消息、mention/urgent 展示

MessageReceiptState
  key: message_id
  source: message.read_bits + member snapshot
  purpose: 单消息 readers/unreaders/confirmed 状态
```

权威关系：

1. 当前用户频道阅读水位以 `ChannelMember`/`ViewerChannelReadState` 为准。
2. `channel.unread_count` 若暂时保留，只能是当前 viewer 的物化镜像，不能成为第二权威。
3. `message.read_bits` 是消息收件人回执，不能单独清频道 badge，也不能拿来代替 ChannelMember 水位。
4. 重启恢复的 dialog query 必须用 Channel 公共字段 overlay 当前账号 ChannelMember 阅读状态，不得只扫 `channel`。
5. 一次服务端 read settlement 引起的 ChannelMember、dialog badge、总未读和 receipt 变化应在同一持久化边界完成，并在同一 frame 中发布。

当前 Helix 已能持久化部分 `message.read_bits` 和 `channel_member` 未读字段，但代码分析真源已明确记录：`channel.unread_count=0`、`last_read_seq/msg_count_all` 镜像与 type6 sync 的完整闭环尚未完成。这是实现阶段 P0，不得由 Angular/Mobile 补算。

## 3. 目标职责边界

### 3.1 Helix 必须负责

- 领域 ID、operation ID、projection key 的确定性分配；
- command 选择、HTTP/WS payload、权限和生命周期裁决；
- sending/sent/failed、retry、revoke、schedule、reply、read、channel/member 状态机；
- 读写窗口、去重、排序、锚点、旧请求抑制和消息身份对账；
- 当前 surface 的选择、可见窗口事实解释和已读候选决策；
- 持久化 barrier、EffectResult、WS/sync 回声对账；
- 完整 ViewModel、动作绑定、一次性 UI 语义和恢复 snapshot。

### 3.2 各端 shell 只负责

- 把用户点击、表单值、文件句柄以及焦点/可见性/viewport 事实回送 Helix；
- 执行网络、存储、timer、原生文件选择等 HostEffect；
- 机械应用 frame，按 `render_kind` 选择静态组件并绑定 props；
- 执行 directive 的平台实现并 ack；
- 维护可丢弃的 frame cache，以及像素滚动、动画、原生焦点句柄。

### 3.3 各端明确禁止

- raw `im_*` command、HTTP URL/body/header 和 WS event 分发；
- 生成 temporary ID、猜 server ID、reconcile 消息身份；
- 从 `props` 或 event 名推导实体类型、默认状态、allowed action；
- unread candidate 选择、作者豁免、in-flight/completed read set；
- 消息 merge/dedup/sort/slice、回复索引、频道分组与群成员业务筛选；
- 根据 HTTP 200、command accepted 或 baseline diff 猜操作完成；
- 自行决定 close 还是 leave、成功后跳哪个频道、何时清空输入框。

## 4. `InteractionProtocol/v1`

### 4.1 协议包络

所有类型由 Rust schema 生成 TS、JavaScript、C++、Dart bindings/reducer，禁止各端手写第二套字段解释。协议公开面必须使用 newtype、enum 和 tagged union；`String + Value` 只允许承载真正开放的用户文本或 opaque host handle，不能承载 action、node、capability、reason 或业务 props。

```rust
struct ProtocolEnvelope<T> {
    protocol_version: u16,
    engine_session_id: EngineSessionId,
    account_key: AccountKey,
    surface_id: SurfaceId,
    surface_epoch: u64,
    trace_id: TraceId,
    payload: T,
}

struct SurfaceId(String);
struct ActionRef(String);       // opaque，不可由 UI 解析
struct NodeKey(String);         // opaque，不等于业务实体 ID
struct DirectiveId(String);
```

- `engine_session_id`：Helix 本次进程/恢复会话标识。
- `surface_id`：窗口、页面或嵌入面实例；不等于 channel ID。
- `surface_epoch`：同一 surface 重建时递增，旧 epoch 输入 fail closed。
- `trace_id`：逐跳 correlation，不承担业务身份。

### 4.2 上行 `UiEvent`

```rust
enum UiEvent {
    SurfaceAttached {
        surface_kind: SurfaceKind,
        capabilities: Vec<PlatformCapability>,
    },
    SurfaceDetached,
    ActionInvoked {
        action_ref: ActionRef,
        observed_revision: u64,
        input: ActionInput,
    },
    ViewportObserved {
        ordered_visible_node_keys: Vec<NodeKey>,
        first_visible_node_key: Option<NodeKey>,
        last_visible_node_key: Option<NodeKey>,
        app_visibility: Visibility,
        window_focus: bool,
    },
    HostStateObserved {
        app_visibility: Visibility,
        window_focus: bool,
        connectivity: Connectivity,
    },
    FrameApplied { revision: u64 },
    DirectiveApplied { directive_id: DirectiveId, outcome: ApplyOutcome },
    ResyncRequested { observed_revision: u64, reason: ResyncReason },
}

enum ActionInput {
    SendMessage { text: String },
    RetryMessage,
    OpenEntity,
    UpdateChannelSettings(ChannelSettingsInput),
    SelectMembers { member_node_keys: Vec<NodeKey> },
    OpaqueHostResource(OpaqueHostHandle),
}
```

`ActionInvoked` 的 `input` 只携带动作绑定声明允许的原始表单值或 opaque host handle。它不携带 command name、channel ID、temporary ID、HTTP body。`ViewportObserved` 上报 projection node key，不上报 UI 自己筛出的“应标已读 message IDs”。

### 4.3 下行 `ActionBinding`

```rust
struct ActionBinding {
    action_ref: ActionRef,
    surface_epoch: u64,
    owner_node_key: NodeKey,
    action_kind: ActionKind,
    enabled: bool,
    disabled_reason: Option<LocalizedText>,
    input_contract: InputContract,
    validity: ActionValidity,
    replay_policy: ReplayPolicy,
}
```

`action_ref` 是 opaque、surface-scoped 的动作地址。Helix 内部绑定领域上下文和 use case。`ActionValidity` 至少包含 `surface_epoch + context_version + expires_after_revision`；无关 badge 或窗口 patch 不得让整个 surface 的动作集失效。v1 binding 必须在 wire 中显式携带 `surface_epoch`，作为 renderer 构造 invoke envelope 的唯一 epoch 真源；UI 不得从 revision、本地计数或默认值猜测。UI 只渲染按钮并回送引用。

例子：群设置页可以同时收到 `channel.close` 和 `channel.leave` 两种语义 kind，但只会收到当前用户有权执行且绑定了独立 `action_ref` 的动作。UI 不根据角色做分支，也看不到对应的 `im_channel_*` 命令。

### 4.4 下行 `InteractionFrame`

```rust
struct InteractionFrame {
    frame_id: FrameId,
    causation_id: CausationId,
    mode: FrameMode,              // Snapshot | Patch
    base_revision: u64,
    revision: u64,
    mutations: Vec<ViewMutation>,
    action_bindings: Vec<ActionBinding>,
    directives: Vec<UiDirective>,
}

enum ViewMutation {
    UpsertNode { node: ViewNode },
    RemoveNode { node_key: NodeKey },
    SetChildren {
        parent_key: NodeKey,
        ordered_child_keys: Vec<NodeKey>,
    },
    RemoveSubtree { root_key: NodeKey },
}

enum ViewNode {
    MessageRowV1(Keyed<MessageRowVm>),
    DialogRowV1(Keyed<DialogRowVm>),
    ComposerV1(Keyed<ComposerVm>),
    ReceiptV1(Keyed<ReceiptVm>),
    ChannelSettingsV1(Keyed<ChannelSettingsVm>),
    MemberRowV1(Keyed<MemberRowVm>),
}

struct Keyed<T> {
    node_key: NodeKey,
    revision: u64,
    props: T,
    action_refs: Vec<ActionRef>,
}
```

约束：

- 一个 frame 内 mutation 按序、原子 apply；应用前不允许组件观察到半帧状态。
- `SetChildren` 给出完整顺序，端侧不得再次排序、去重、slice 或 prepend。
- 节点身份由 `NodeKey` 稳定表达；server ID 变化不能迫使端侧维护映射。
- `render_kind` 由 `ViewNode` tagged union 的变体稳定派生，不接受任意字符串；每个变体的 props 是生成的强类型 VM，不使用通用 JSON `Value`。
- Snapshot 权威替换整个 surface；Patch 必须满足 `base_revision == local_revision`。
- gap、未知 epoch、未知 action_ref 一律 fail closed 并请求 snapshot。

### 4.5 Surface attach / resync

端侧只能请求一个已知 surface 的权威重放，不能选择表名、HTTP endpoint、领域 command 或 raw
payload。v1 host-to-core 私有命令是 `__helix_request_surface_snapshot`；宿主必须通过版本化
`SurfaceResyncRequest` builder 发送 `{ surface, scope }`，并由 Helix 验证 `scope` 与 `surface`
精确匹配。renderer 不直接构造该私有 command，也不得把 storage 行或 HTTP body 作为 surface
输入。

八个已接管 surface 的 producer 必须保持以下归属：

| Surface | Helix 权威事实 | 输出 |
|---|---|---|
| `message.left-dialog-list` / `message.unread-summary` | channel / member dialog storage | 同一 Frame 的 paired snapshot/patch |
| `message.timeline` | local message storage，缺失时 Helix HTTP fallback | typed timeline snapshot/patch |
| `message.thread-detail` | replies HTTP | typed thread snapshot/patch |
| `message.channel-summary` | channel storage | typed summary snapshot/patch |
| `message.member-roster` | channel_member storage | typed roster snapshot/patch |
| `message.topic-list` | topic channel storage | typed topic snapshot/patch |
| `message.participant-receipts` | canonical read-receipt HTTP | typed receipt snapshot/patch |

缺少 action authority 时 Helix 必须下发显式 disabled binding/reason，而不是让端侧回退为旧
`im_*` command。所有 attach 的时间复杂度只由其有界 storage/HTTP producer 决定；端侧没有
二次聚合、扫描或补面循环。

### 4.6 type-7 terminal closed

同步 type-7 只接受 Go 的 sealed wire：

```json
{"id":"event-close-01","channelId":"<id26>","eventSeq":42,"eventType":7,"payload":{"state":"closed"}}
```

它不得包含 `msgId`、actor、post、body、时间或任意额外字段，legacy WS 数值帧的 type-7 也必须
拒绝。Helix 将 closed marker 与 `channel_event_cursor.last_event_seq` 放入单个
`PersistAtomic`；只在 `PortReply::Ok` 后更新内存 cursor/tombstone 并发布关闭的 dialog
Frame。`Err`、gap、混合 batch、重复或重启后晚到 WS/sync 都不得推进 cursor、再次写入或复活
surface。driver 未实现跨 op 原子写时必须 fail closed，不能降级为普通 `Persist`。

### 4.7 一次性 `UiDirective`

```rust
struct UiDirective {
    directive_id: DirectiveId,
    causation_id: CausationId,
    kind: DirectiveKind,
    precondition: DirectivePrecondition,
    replay_policy: DirectiveReplayPolicy,
}

enum DirectiveKind {
    ResetInput { node_key: NodeKey, expected_value_revision: u64 },
    FocusNode { node_key: NodeKey },
    RevealNode { node_key: NodeKey, alignment: Alignment },
    SelectSurfaceEntity { node_key: NodeKey },
    DismissSurfaceRegion { node_key: NodeKey },
    AnnounceOutcome { operation_key: OperationKey },
}
```

Directive 表达跨端“做什么”，各端决定“怎么做”。它不得出现 DOM selector、Widget key、CSS、px、动画时长。持久业务结果必须同时存在于 ViewModel，不能只靠一次性 directive。

重复 directive 通过 `directive_id + ack` 幂等处理；`replay_policy` 决定重连后是重放、丢弃还是等待确认。`DirectivePrecondition` 必须阻止延迟到达的旧 `ResetInput` 清空用户随后输入的新内容；不能仅凭 directive ID 假设时序安全。

### 4.8 HostEffect 与 UiDirective 不能混

| 类型 | 执行者 | 例子 | 是否业务权威 |
|---|---|---|---|
| `HostEffect` | native/FFI driver | Storage、HTTP、timer、文件读取 | 结果必须回到 Helix |
| `ViewMutation` | UI frame reducer | Upsert/Remove/SetChildren | Helix ViewModel 的副本 |
| `UiDirective` | platform renderer | focus/reveal/reset/dismiss | 否，必须可 ack |

图中的 `EffectResult` 是概念名，代码实现必须复用现有 `Tick::PortReply + Correlation + CorrelationContext` 回灌链，不新增第二套 effect-result loop。发送、已读和群治理的 operation FSM 归 `helix-im`，不能借协议升级把业务状态塞进 `helix-core`。

## 5. 状态机硬不变量

1. **Persist-before-present**：领域状态持久化成功后，才能发布对应可见 frame。
2. **Persist-before-transport for optimistic send**：发送的 `sending` 记录先落库，再上屏，再允许 HTTP/timer effect。
3. **Echo-before-sent**：HTTP 200 只能更新 operation transport 状态；没有匹配 WS/sync 权威回声不得投影 `sent`。
4. **Atomic presentation**：一次 read settlement、reconcile 或群治理结果造成的多节点变化必须在一个 frame 内发布。
5. **Surface isolation**：所有交互状态按 `engine_session_id + surface_id + surface_epoch` 隔离。
6. **Stale action fail closed**：action revision、surface epoch 或权限不匹配时拒绝，不由 UI 自行降级执行。
7. **Deterministic replay**：相同 UiEvent、HostObservation、EffectResult、WS/sync 序列必须产生字节等价 frame。
8. **Batch across FFI**：一个 frame 作为一个批次跨 FFI，不逐 mutation 调用 Flutter/C++。
9. **No client repair**：破坏 Helix 投影时 PC/Mobile 必须同时失败，端侧不得用本地规则补绿。
10. **Typed contract**：业务 action/node/directive/capability/reason 不得退回任意字符串或 JSON `Value`；canonical encoding 必须固定 tagged-union discriminant、字段默认值和 map 顺序。
11. **Ack non-blocking**：`FrameApplied` 用于诊断、资源回收与恢复水位，不阻塞后续连续 revision frame；正确性由 base revision/gap/resync 保证。需要可靠确认的一次性行为使用独立 `DirectiveApplied`。
12. **Bounded registries**：ActionRegistry/SurfaceStore 必须随 detach、epoch rollover 和 revision 水位淘汰；action lookup 平均 O(1)，frame apply 为 O(mutations + children)，禁止节点逐个全表扫描形成 O(n²)。

## 6. G-01 发送消息完整闭环

### 6.1 目标时序

```text
Helix -> frame: composer node + opaque send action_ref
UI -> ActionInvoked(action_ref, {text})
Helix -> 校验 action/surface/draft，解析绑定的 channel context
Helix -> IdSource 分配 operation_id/projection_key
Helix -> Persist PendingMessage(sending)
Driver -> PersistOk
Helix -> 原子 frame:
           Upsert message.row.v1(sending)
           SetChildren(message.window ordered keys)
           Upsert dialog/header preview
           ResetInput(composer)
UI -> FrameApplied + DirectiveApplied
Helix -> HTTP/timer HostEffect
Driver -> HTTP result 回 Helix；HTTP 200 不发布 sent
WS/sync -> 匹配回声
Helix -> reconcile + Persist message(sent/server identity)
Driver -> PersistOk
Helix -> 同一 node_key Upsert message.row.v1(sent)
```

失败与重试：

- 超时/失败由 Helix 持久化 `failed`，frame 下发失败节点和新的 retry `action_ref`。
- UI 只回送 retry action，不判断当前状态能否重试，不选择命令，不重新生成 ID。
- 重启时 UI 只发 `SurfaceAttached`，Helix 恢复 pending/failed、timer 和 snapshot，不由 UI 编排 query 顺序。

### 6.2 当前实现阻塞

`handle_send_message` 已由 module 内确定性 allocator 铸造 `temporary_id`，公开 client API 会删除端侧预生成值；同一 ID 贯穿 optimistic store、投影、HTTP、PendingSend 与 echo reconcile。剩余阻塞是当前仍在同一次 step 中先后吐 `Persist`、Emit 与 HTTP，尚未完成 `PersistOk -> frame -> HTTP` 分阶段状态机。

## 7. G-13/G-14 未读与回执闭环

### 7.1 UI 只上报事实

```text
SurfaceAttached / ActionInvoked(打开频道)
ViewportObserved(visible node keys, foreground, focus)
```

UI 不提交“这些 post 应标已读”。Helix 结合当前 surface、viewer、canonical message window、作者、可见性、持久水位和 in-flight operation 决定：

- 是否触发 G-13 `channels/view`；
- 是否以及哪些消息触发 G-14 `post/read`；
- 作者豁免、重复抑制和跨设备收敛；
- 何时允许 badge 清零。

### 7.2 权威时序

```text
HostObservation
-> Helix ReadDecision
-> HostEffect channels/view；需要时 post/read
-> 不乐观清 badge
-> server update_channel/post_read 或离线 sync 回声
-> Persist ViewerChannelReadState(ChannelMember absolute values)
-> Persist MessageReceiptState(message.read_bits)
-> 重新投影 ChannelDialogView = Channel JOIN current viewer ChannelMember
-> PersistOk
-> 一个 InteractionFrame 原子更新：
     channel badge/mention/urgent
     total unread
     read cursor/operation view
     message receipt node（若当前 surface 展示）
-> UI apply + ack
```

### 7.3 必须关闭的 P0 缺口

- dialog 冷启动 query 不能再只从 `channel.unread_count` 恢复；必须 overlay 当前账号 ChannelMember。
- type6 sync 没有 msgId 时也必须推进阅读水位并收敛 badge，不能只推进 cursor。
- `message.read_bits` 更新不能直接或间接替代频道已读。
- G-13 未经服务端确认和本地 Persist 不得清零。
- Angular 的 candidate/in-flight/completed set 和 author-self 判断全部删除。

## 8. 真正的 UC16 群治理闭环

### 8.1 独立动作

Helix 根据当前用户、角色、频道生命周期和版本生成独立绑定：

- `channel.close`：关闭群；
- `channel.leave`：当前用户主动退出；
- `channel.member.remove`：管理员移除指定成员；
- `channel.settings.update`：修改群资料。

它们不得共用一个 action_ref，也不得由 UI 通过角色或 event 名选择 raw command。

### 8.2 目标时序

```text
Helix -> settings/member nodes + allowed action bindings
UI -> ActionInvoked(action_ref, form input)
Helix -> 权限、版本、生命周期裁决
Helix -> HostEffect
server WS/sync -> closed/left/member/settings fact
Helix -> Persist Channel + ChannelMember + system message/settings version
PersistOk
Helix -> 一个 InteractionFrame：
          更新或移除 channel row
          更新 member list
          更新 settings/allowed actions/system row
          必要时 SelectSurfaceEntity(fallback node)
          必要时 DismissSurfaceRegion(settings/member panel)
UI -> apply/directive ack
```

Close 后频道进入 closed 只读态；self leave 后当前 viewer 的 membership 消失；admin remove 只影响目标 member；三者不能合并为同一 patch。fallback 频道由 Helix 根据当前 surface 的呈现模型选择，UI 不从本地列表猜。

## 9. UC01-16 交互合同矩阵

| 宏 UC / Gate | 上行事实或动作 | Helix 权威决策/持久化 | 下行可直接消费合同 | 端侧必须删除 |
|---|---|---|---|---|
| UC01 / G-01 文本发送 | send action + text | ID、sending、timer、HTTP、echo、sent | message/dialog mutation + reset directive | temp ID、payload、reconcile |
| UC02 / G-02 文档发送 | action + opaque file handle/说明 | 文件计划、wire props、发送状态 | upload/message/operation nodes | attachment props 默认值 |
| UC03 / G-03 媒体 | action + resource handle/type | upload effect、对象键、消息状态 | upload progress + message nodes | IMAGE/FILE 业务分支；保持 blocked 不伪绿 |
| UC04 / G-03a 重发 | retry action_ref | 读失败消息、资格校验、复用身份 | failed->sending message mutation | 本地 retry 判定/新 ID |
| UC05 / G-04/G-04a 撤回 | revoke action_ref | 在线/离线重放、终态与 actor | revoked message node | 撤回字段 merge/文案推导 |
| UC06 / G-05 转发 | form action + selected node refs/mode | 源消息解析、目标批次、每目标状态 | batch operation + target message nodes | Post 组装、批次去重 |
| UC07 / G-06 快捷回复 | reaction action_ref + emoji input | emoji 规范化、权限、聚合 | reaction-ready message node | props/string 聚合 |
| UC08 / G-07 加急 | urgent action + target node refs | 目标、required/confirmed/readers | urgent message + receipt nodes | counts/readers 二次计算 |
| UC09 / G-08/G-09 定时 | schedule/cancel action + time/content | 时间校验、revision、持久化、取消 | schedule/operation nodes + dismiss directive | sendAt payload、baseline 猜成功 |
| UC10 / G-10/G-10a 首屏/历史 | attach/open/load-older action | query、anchor、窗口、排序、边界 | message window snapshot/patch | slice、oldest、merge/prepend |
| UC11 / G-11 定位 | locate action_ref | 本地优先、远端补齐、上下文窗口 | ordered window + reveal directive | 自拼窗口和 highlight |
| UC12 / G-12 回复 | open/load action_ref | snapshot/append、revision、去重保序 | reply tree mutations | reply map/index/preview |
| UC13 / G-13/G-14 已读 | viewport/focus/visibility facts | read decision、ChannelMember 水位、receipt | 原子 unread + receipt frame | candidate、作者豁免、乐观清零 |
| UC14 / G-14a 模板收到 | confirm action_ref/visibility fact | 幂等、权限、received 终态 | template/message node | 自去重请求和状态推导 |
| UC15 / G-15a/G-15 建群/话题 | form action + member node refs | 成员规范化、创建、topic 关联 | channel/member/reply nodes + select directive | dialog 差集、成员 payload |
| UC16 / G-16/G-16b/G-16c 群治理 | 独立 close/leave/remove/settings action | 权限、版本、成员/频道终态 | channel/member/settings/system frame + fallback | 角色分支、event-prefix patch |

同步 `G-S1..G-S4` 是所有 UC 的共同前置：cursor、channel event、消息和成员恢复必须先进入 DomainModel，再由同一个 Interaction projection 输出 snapshot；UI 不消费 raw sync event。

## 10. 各端目标消费代码

### 10.1 Angular

```ts
export class MessageSurfaceAdapter {
  constructor(
    private readonly core: HelixInteractionPort,
    private readonly frames: GeneratedInteractionFrameStore,
    private readonly directives: PlatformDirectiveExecutor,
  ) {}

  invoke(actionRef: string, input: unknown): void {
    this.core.dispatch({
      type: 'ActionInvoked',
      actionRef,
      observedRevision: this.frames.revision,
      input,
    });
  }

  onFrame(frame: InteractionFrame): void {
    this.frames.applyAtomically(frame);
    this.directives.apply(frame.directives);
    this.core.frameApplied(frame.revision);
  }
}
```

Angular 页面只遍历 `node_key`、按 `render_kind` 选择已有组件、绑定 `props/action_ref`。`ProjectionStore` 中的 message map/set、temp map、排序、窗口、reply index、unread controller 和 operation baseline 都不再存在。

### 10.2 `mobile-quick.js` 目标代码

当前 mobile 仓没有正式的 `mobile-quick.js`；真实 JS 入口是 `smoke/quickjs_core_bridge_smoke.js`，且仍解析 raw event、调用 `im_send` 和 query。迁移后的正式适配器应缩成下面这种通用代码：

```js
// mobile-quick.js: target adapter, generated contract only
export function createMessageSurface(core, renderer, directiveExecutor) {
  const store = new GeneratedInteractionFrameStore();

  core.onFrame((frame) => {
    store.applyAtomically(frame);
    renderer.render(store.snapshot());

    for (const directive of frame.directives) {
      const outcome = directiveExecutor.apply(directive);
      core.directiveApplied(directive.directiveId, outcome);
    }

    core.frameApplied(frame.revision);
  });

  return {
    attach(surfaceKind, capabilities) {
      core.dispatch({ type: 'SurfaceAttached', surfaceKind, capabilities });
    },
    invoke(actionRef, input) {
      core.dispatch({
        type: 'ActionInvoked',
        actionRef,
        observedRevision: store.revision,
        input,
      });
    },
    observeViewport(observation) {
      core.dispatch({ type: 'ViewportObserved', ...observation });
    },
  };
}
```

这里没有 command 名、temp ID、readBits、event-prefix switch、channel/message/member SQL patch。

### 10.3 C++/FFI

```cpp
void MessageSurface::onFrame(const InteractionFrame& frame) {
  auto result = generated_store_.applyAtomically(frame);
  if (result.needsSnapshot()) {
    core_.requestResync(result.observedRevision(), result.reason());
    return;
  }
  renderer_.render(generated_store_.snapshot());
  directive_executor_.applyBatch(frame.directives());
  core_.frameApplied(frame.revision());
}
```

删除 `MobileImSdk::applyEventProjection` 的 event-name 推断、`ViewModelStore` 的 temp/server reconcile 和 `im.store.applyPatch` 业务写入口。FFI 一次传整帧，不逐 mutation 往返。

### 10.4 未来 Flutter

Flutter 使用同一 schema 生成的 Dart types/store：Widget 只消费 ViewNode 和 ActionBinding，用户动作回送 action_ref，directive 由 Flutter adapter 实现。不得另建 Dart BLoC/Provider 业务 reducer。

## 11. 改造影响面

### 11.1 Helix

建议新边界，不预设最终文件名：

- `helix-im`：新增 SurfaceModel、InteractionProtocol、ActionRegistry、InteractionProjector、read decision、snapshot/revision/ack 状态机。
- `helix-core`：仅在通用 Effect/EffectResult 或确定性 IdSource 端口确有缺口时扩展，保持 sans-IO，不认识 IM。
- `helix-driver-native/ffi`：批量 frame transport、effect result、ack/resync；FFI 不逐条过 mutation。
- Storage：补齐 current viewer ChannelMember read authority、发送阶段 barrier、surface 恢复所需状态。
- Contracts/tests：单一 schema 生成 Rust/TS/JS/C++/Dart 类型与通用 reducer fixture。

### 11.2 cses-client

删除或降级：

- `projection-store.service.ts`：业务 map/set、temp map、默认值、merge、排序、窗口、reply tree。
- `message-page-projection.presenter.ts`：频道分组、route/settings/reply 二次投影。
- `message-read-reporting.ts` 与页面 auto-read controller：candidate/in-flight/completed/作者豁免。
- `shell.service.ts`：raw command、payload、anchor、temporary ID、恢复 query 编排。

保留/新增：generated frame store、action dispatcher、component registry、directive executor、surface host observation adapter。

### 11.3 mobile-qucik-c++

删除或降级：

- `MobileImSdk::applyEventProjection` 的 raw event -> channel/message/member patch 解释。
- `ViewModelStore` 的业务表 schema、默认时间、temp/server reconcile。
- QuickJS 的 `im.store.applyPatch` 和大批 raw business namespace。
- smoke JS 中 raw event parse、`im_send` payload 与 query 编排。

保留/新增：generated frame reducer、批量 FFI bridge、renderer registry、directive executor、host observation adapter。

### 11.4 条件介入与明确排除

- `cses-im-server`：仅当 G-01/G-13/G-14/G-16 的真实 wire、绝对未读、成员或生命周期权威状态不足时修复；它不承载 ViewNode、ActionBinding 或 UiDirective。
- `loopforge-tauri-im`：不改代码、不作为首落地、不参加联调或发布证据。
- `pd/message`：不改代码、不承载合同生成、不参加联调或发布证据。
- Dioxus/Crux：只读参照，不属于产品实施仓库。

## 12. 回归优先的分阶段迁移

### P-1 三仓回归基线先行

InteractionProtocol 本轮范围是 27 个 Gate：同步前置 `G-S1..G-S4`，以及 `G-01..G-16c` 的 23 个业务 Gate。Strict Matrix 当前总计 46 个 `uc_cases`；P-1 必须建立同一份 46-case manifest，其中 27 个标记为 `interaction_migration`，其余 `G-17..G-27` 等 19 个标记为 `regression_only`。不能等改完后再从真实环境重新联调，也不能把范围外能力从全仓回归里删掉。

同一 case 必须保留两份结果，禁止把旧端逻辑直接升级为新真源：

- `observed_legacy`：冻结当前 Helix projection、Angular DOM/ProjectionStore、Mobile ViewModelStore 的实际行为，作为 characterization regression signal。
- `canonical_expected`：由 Strict Matrix + Helix DomainModel/持久化不变量裁决的目标 Effect、InteractionFrame 和最终 snapshot；已知旧 bug 在这里明确标成 expected migration，不得被黄金文件永久冻结。

canonical fixture 建议由 Helix 单一维护，并用 manifest 锁定：`contract_epoch + schema_sha256 + fixture_manifest_sha256 + case_id + gate_id + input_sha256 + expected_effect_sha256 + expected_frame_sha256 + expected_snapshot_sha256`。cses-client 与 mobile 只消费同一 manifest，不各自复制手写 fixture。

#### 各仓先补什么测试

| 仓库 | 迁移前 characterization | 新协议离线回归 | 仓内全量入口 | 真实链路入口 |
|---|---|---|---|---|
| `helix` | 用现有 Tick/WS/PortReply fixture 固化每个 case 的 Effect、持久化操作、legacy projection；记录当前红项，不伪造 PASS | 同输入回放 `InteractionProjector`；断言 snapshot=patch apply、重复/乱序/gap、Persist/HTTP/echo barrier、native/FFI 字节等价 | `node scripts/strict-matrix/check.mjs --check-generated`；`scripts/gate.sh` | `scripts/host-cli-FULL-uc-e2e.sh` |
| `cses-client` | 给 `ProjectionStore`、presenter、read controller、Shell 和关键 DOM 增加 characterization test，冻结 27 个迁移 Gate 的可见语义；其余 manifest case 保持现有回归或显式排除 | generated TS reducer 对 27 个 Helix frame fixture 离线 apply；component registry/directive executor/DOM 只断生成 VM，不重算业务 | 全仓：`npm test -- --runInBand`、`npm run lint`、`npm run rust:check`；Message 聚焦：`npm run test:desktop:uc`、`npm run test:message-v4:strict-contract`、`npm run test:message-v4:gate`、`npm run test:message-v4:real-chain-scope` | `npm run message:desktop:scenario:all` |
| `mobile-qucik-c++` | 用现有 `MobileSdkSpecTest`、`MobileJsSpecTest` 固化 raw event -> ViewModelStore/QuickJS 的当前行为 | generated C++/JS reducer 重放同一 frame fixture；断言 ABI ownership、整帧批量、snapshot=patch、坏 revision fail closed | `make push-gate-local`（含 `full-gate`、Strict Matrix、GoogleTest、manifest） | `make push-gate-real-chain` 或逐 case `REAL_CHAIN_CASE=<case> make real-chain-uc` |
| `cses-im-server`（条件） | 仅在后端有改动时冻结目标 endpoint、WS event、DB readback 与 event_seq | 不消费 InteractionFrame，只验证 wire/绝对领域事实未漂移 | `go test ./...` | 由上述 Helix/Mobile real-chain runner 复核 HTTP+WS+DB evidence |

这不是从零搭测试。第一批直接复用并扩展现有资产：

- Helix：`projection_schema_test.rs`、`strict_matrix_contract_test.rs`、`f5_golden_replay.rs`、`golden_replay_test.rs` 及 `tests/fixtures/real_go_frames/`。
- cses-client：`projection-store.service.test.ts`、`projection-render-contract.test.ts`、`shell.service.test.ts`、`message-read-reporting.test.ts`、`tests/e2e/desktop/` 与现有 Strict Matrix/evidence scripts。
- mobile：`MobileSdkSpecTest.cpp`、`MobileJsSpecTest.cpp`、`MobileStrictMatrixJsSpecTest.cpp`、`MobileRealChainSpecTest.cpp` 与 `real-chain/uc-manifest.json`。

先把这些旧测试按 `case_id` 接入统一 manifest，再新增 InteractionFrame 离线 replay。只有当某个旧测试的业务断言已经被 canonical fixture 和新 reducer 等价覆盖，才允许删除旧测试；不能因为旧实现准备删除，就先删保护网。

#### 五层回归顺序

1. **L0 schema/generator**：Rust schema 生成 TS/C++/JS/Dart，hash 和生成产物零漂移。
2. **L1 单仓离线**：三仓各自重放同一 case，不启动 Go、不启动 Tauri，先消灭字段、顺序、默认值和 reducer 漂移。
3. **L2 跨语言 parity**：Rust 生成的 frame 经 TS 与 C++/JS reducer 得到同一 canonical snapshot；破坏一条 mutation 后三端必须同时失败。
4. **L3 shadow dual-run**：真实输入同时进入 legacy projection 与新 InteractionProjector，新 frame 暂不驱动可见 UI，只比较归一化语义 diff；差异必须归类为 `expected_migration` 或 `regression`。
5. **L4 Gate cutover + real chain**：只有 L0-L3 全绿才按 Gate/surface/account 灰度切换；切换后再跑 HTTP、WS、Storage、InteractionFrame、DOM/移动 UI 全链证据。

迁移开关必须是 per-Gate，不设一个全局大爆炸开关。未迁移 Gate 继续走 legacy；已迁移 Gate 不得在新链失败时偷偷回退旧业务 reducer，否则负向测试无法证明边界已经收口。

全量门禁节奏固定如下：

1. P-1 开始时三仓各跑一次仓内全量门，保存 baseline manifest、命令、SHA、PASS/已知红项和产物路径。
2. 每个 Gate 改动先跑该 Gate fixture/characterization/负向测试，再跑被修改仓库的全量本地门；只跑 focused test 不能合并。
3. 每个 Phase 合并前同时跑三仓本地全量门 + 27 个迁移 Gate 的跨语言离线 parity，并核对全部 46 个 Strict Matrix case 的 regression/explicit-exclusion 状态；任何 case 无声明、缺失或从 PASS 变红都阻断。
4. 只有本地与离线回归全绿后才跑三端 real-chain；真实联调是最后一层验收，不承担首次发现 schema、排序、默认值或 reducer 错误的职责。
5. 当前 runner 若未覆盖全部 27 个迁移 Gate，P-1 必须先补 mapping；全部 46 cases 若存在环境或产品范围排除，manifest 必须逐 case 写明理由和证据状态，不能静默漏掉。覆盖或声明不完整时只能报告 partial，禁止称“全量测试”。

### P0 合同冻结与真实缺口

1. 冻结强类型 `InteractionProtocol/v1`、canonical encoding、surface/revision/ack/resync、action/directive 语义；禁止以任意 `String/Value` 代替 tagged union。
2. 冻结 deterministic ID 机制，修正文档和当前 send 入参冲突。
3. 补 `PersistOk -> frame -> HTTP` 发送状态机。
4. 补 ChannelMember 权威未读、dialog overlay 和 type6 sync 闭环。
5. 先闭合 `G-S1/G-S2/G-S3/G-S4` 的 hello/increment、notify、gap 补偿和陌生 channel 恢复回归，它们是所有业务 Gate 的共同输入前置。
6. 建同一 schema 的 bindings/reducer generator 与跨语言 fixture。

### P1 双端运行壳

1. native/FFI 增加批量 frame、UiEvent、ack/resync 通道。
2. 两个 surface 并行、epoch 重建、snapshot recovery 先跑通。
3. legacy/new 双跑只用于对照；新功能不再写旧解释器，已迁移 Gate 禁止 fallback。

### P2 纵切 G-01

用文本发送证明 action_ref、ID、persist barrier、sending/sent/failed、reconcile、reset directive 与重启恢复闭环。通过 P-1/L0-L4 后，删除 Angular/Mobile 的 G-01 发送业务逻辑。

### P3 迁移 G-02..G-09 消息动作

- `G-02`：文档发送，opaque 文件句柄、上传计划、wire props 与 message/operation frame。
- `G-03`：媒体发送；当前 blocked/deferred 面继续 fail closed，不因新协议伪绿。
- `G-03a`：失败重发，复用原 operation/message identity，不生成新 temporary ID。
- `G-04/G-04a`：在线撤回与离线重放，actor、时间和终态由 Helix 投影。
- `G-05`：转发，源消息解析、目标去重、批次和逐目标结果由 Helix 负责。
- `G-06`：快捷回复/表情，规范化、权限和聚合结果进入强类型 message node。
- `G-07`：加急，required/confirmed/readers/unreaders 由领域状态投影，端侧不重算。
- `G-08/G-09`：定时发送与取消，时间校验、revision、持久化、恢复和取消终态在 Helix。

每个 Gate 必须分别完成 fixture、Rust replay、TS/C++ reducer parity、shadow diff、负向破坏和真实链路后才能切换；不能用 G-01 的 PASS 代替 G-02..G-09。

### P4 窗口与回复

迁移 `G-10/G-10a/G-11/G-12`，删除各端 merge/sort/window/reply index。

### P5 未读

迁移 `G-13/G-14/G-14a`，完成三个未读/回执对象的权威关系和原子 frame，删除各端 read controller。

### P6 建群与群治理

迁移 `G-15a/G-15/G-16/G-16b/G-16c`，验证建群/话题、close/leave/remove/settings 独立 action 和 fallback directive。

### P7 UC01-16 收口

对 27 个迁移 Gate 重跑 L0-L4，并对 Strict Matrix 全部 46 cases 重跑仓内 no-regression/explicit-exclusion 检查；删除 raw command/event/store 入口和 shadow legacy path；Strict Matrix 补 interaction contract 证据面，PC/FFI 同输入必须输出等价 frame。

## 13. 验收合同

### 13.1 协议与确定性

- Rust/TS/JS/C++/Dart 类型来自同一 schema，字段漂移检查 fail closed。
- 相同 UiEvent/HostObservation/EffectResult/WS 序列在 native 与 FFI 输出字节等价 frame。
- 同一 frame fixture 经 TS/JS/C++/Dart reducer 得到等价快照。
- 重复 frame 幂等；revision gap 必须请求 snapshot；stale action/epoch 必须拒绝。
- 两个 surface 同时打开不同频道，不共享全局 active channel。

### 13.2 回归目录完整性

- Strict Matrix 当前 46 个 `uc_cases` 全部进入 fixture manifest，`case_id/gate_id` 唯一，并明确标记 27 个 `interaction_migration` 与 19 个 `regression_only`；生成检查 fail closed。
- 每个 case 至少有 Helix replay、TS reducer、C++/JS reducer 三份离线结果；缺任一端不得进入 shadow/real-chain。
- `observed_legacy` 与 `canonical_expected` 分开存储；known drift 必须显式归类，不允许更新 golden 掩盖回归。
- snapshot 与 patch apply 最终状态等价；删除/重排 mutation、损坏 revision 或 action validity 后测试必须失败。
- Gate cutover 后，端侧 legacy 业务 reducer 的对应入口和测试一起删除；未迁移 Gate 的 legacy 路径保持原测试，不做全局重写。

### 13.3 G-01

- Persist 失败时 UI 看不到 sending，HTTP/timer 不执行。
- HTTP 200 无 WS/sync 回声时不得变 sent。
- kill/restart 后 pending/failed/sent 与 frame snapshot 一致。
- UI/QuickJS/C++ 搜不到 temp ID 生成、server ID reconcile、raw send command。

### 13.4 G-02..G-09

- `G-02/G-03`：文件/媒体输入只含 opaque host handle；上传计划、wire props、进度和终态由 Helix 产生；blocked case 保持 blocked。
- `G-03a`：重发复用原 identity；失败、重启、重复点击不产生第二条业务消息。
- `G-04/G-04a`：在线/离线撤回得到相同 canonical revoked snapshot，actor/time 不由 UI 补齐。
- `G-05`：多目标转发的逐目标成功/失败可重放，重复目标不由端侧去重。
- `G-06`：reaction/快捷回复规范化与聚合只在 Helix；TS/C++ snapshot 字节语义等价。
- `G-07`：required/confirmed/readers/unreaders 来自同一领域事实，端侧破坏本地计数不得自愈成绿。
- `G-08/G-09`：定时发送/取消在重启、旧 revision、重复操作下幂等收敛；端侧不根据 baseline diff 猜完成。

### 13.5 G-10/G-10a/G-11/G-12

- snapshot/patch、首屏/历史、定位和回复窗口使用同一 canonical fixture；消息顺序、anchor 和 reply tree 在 TS/C++ apply 后完全一致。
- 端侧删除 merge/sort/prepend/slice/reply index 后，27 个 InteractionProtocol case 离线回归保持通过，全部 46-case regression manifest 无状态倒退。
- 缺帧或旧 viewport 必须 resync/fail closed，不得从端侧缓存猜窗口。

### 13.6 G-13/G-14

- UI 只上报 viewport/focus/visibility，不能提交 read candidate 或先清 badge。
- 重启后 badge 从 current viewer ChannelMember 权威状态恢复。
- `message.read_bits` 单独变化不能清频道 badge。
- 一次 read settlement 的 dialog/total/receipt 变化处于同一 frame。
- 多设备 read 通过绝对状态收敛，旧 revision 不回滚。

### 13.7 G-15a/G-15/G-16/G-16b/G-16c

- close、self leave、admin remove 是不同 action_ref 和不同领域结果。
- 678 等无权限账号收不到 enabled 的管理 binding，UI 不写角色判断。
- 当前频道关闭/离开后由 directive 指向 Helix 选定的 fallback。
- member list、channel lifecycle、settings version、system row 同帧收敛。

### 13.8 负向静态闸门

最终目标代码中以下模式必须为零，生成代码和明确的兼容适配白名单除外：

```text
Angular/Mobile/Flutter: im_send_message|im_post_read|im_channel_close
Angular/Mobile/Flutter: temporaryId generator|temporaryToServer|readBits decoder
Angular/Mobile/Flutter: event.starts_with("im:")|raw WS event switch
Angular/Mobile/Flutter: unread candidate|reply index|message window sort/merge
```

破坏 Helix InteractionProjector 或删除一条 mutation 后，桌面与移动合同测试必须同时失败；任何单端仍然通过都说明该端还藏有业务逻辑。

## 14. 本次验证证据

### 14.1 Dioxus core lens

- `packages/core/src/mutations.rs`：core 通过 `WriteMutations` 输出平台无关变更；listener 由稳定 `ElementId` 路由。
- `packages/core/src/virtual_dom.rs`：先处理事件/脏工作，再批量输出 mutation。
- `packages/desktop/src/edits.rs` 与 interpreter writer：mutation 累积后批量跨边界。

本 Spec 只借鉴“稳定动作地址 + 批量变更 + 事件回送”机制，不把 DOM mutation、RSX 或 renderer 细节搬进 Helix。

### 14.2 Crux lens

- `crux_core/src/lib.rs`：`App` 明确分离 Event、Model、ViewModel、Effect。
- `crux_core/src/core/mod.rs`：Event -> update -> Command/Effect，`view()` 单独生成 ViewModel。
- `crux_core/src/capabilities/render.rs`：Render effect 只是通知 shell 取新 VM，不等于 ViewModel 本身。
- `docs/src/part-2/shell.md`：shell 负责发送 Event、执行 Effect、呈现 ViewModel。

本 Spec 因此把 HostEffect、ViewMutation 和 UiDirective 分开，而不是把一切都叫 projection 或 intent。

### 14.3 当前仓库证据

- Helix `schema.rs` 同时存在 `channel.unread_count`、`channel_member` 阅读字段和 `message.read_bits`。
- `ws/handlers/update_channel.rs` 已把绝对未读写入 ChannelMember；`query/core.rs`/`dialog_render.rs` 的冷启动 dialog 路径仍主要从 Channel 构建。
- `send/flow.rs` 当前要求调用方 temporary ID，并在同 step 产生 Persist 与 sending Emit。
- Angular `projection-store.service.ts` 已增长为业务 map/set、身份映射、merge/default、排序/窗口引擎。
- Angular `message-read-reporting.ts` 与 `message.component.ts` 仍决定 read candidates 和触发时机。
- Mobile `MobileImSdk::applyEventProjection` 仍按 event 名猜 channel/message/member patch。
- Mobile `ViewModelStore` 仍做 temporary/server identity reconcile；QuickJS 仍公开 raw `im.store.applyPatch`。

这些证据共同证明：目标不能停在 `RenderPatch`，必须升级为带动作、呈现、一次性语义、ack 和恢复的完整交互协议。

## 15. 完成定义

只有同时满足以下条件，才能说“Angular、当前移动端和未来 Flutter 只负责 UI 渲染”：

1. 端侧上行只有生成合同内的 UiEvent/HostObservation/ack，业务动作通过 opaque action_ref 回送。
2. Helix 持有完整领域状态、SurfaceModel、Effect 编排、持久化、ViewModel 和动作裁决。
3. 端侧下行只机械 apply InteractionFrame、执行 UiDirective、静态选择组件。
4. P-1 的 27 个迁移 Gate 在 Helix、TS、C++/JS 三端离线回放完整；全部 46 cases 在 regression manifest 中有 PASS/known-red/explicit-exclusion 状态；L0-L3 通过后才允许真实链路切换。
5. `G-01`、`G-02..G-09`、`G-10..G-12`、`G-13/G-14`、`G-15/G-16` 分组逐 Gate 完成 shadow diff、真实双端、重启、断帧和负向破坏验证。
6. UC01-16 所有 Gate 逐条拥有相同 interaction contract 证据，且 raw command/event/业务 reducer 已从各端删除。

这才是目标边界：**Helix 产出各端可直接消费的交互合同，各端只负责平台事实采集、Effect 执行、frame 应用与 UI 渲染。**

## 16. 工作分解与人员配置

### 16.1 工作包清单

| 阶段 | Gate | 主要工作包 | 完成证据 |
|---|---|---|---|
| P-1 | 27 迁移 Gate / 46 全量 cases | 三仓 characterization、canonical fixtures、scope 分类、manifest、全量入口基线 | L0-L2 离线回归 + 当前红项/排除清单 |
| P0 | G-S1/G-S2/G-S3/G-S4 + 全局前置 | 强类型 ABI、canonical encoding、ID/Persist barrier、同步恢复、ChannelMember 权威缺口 | schema/hash/PortReply/replay/fault tests |
| P1 | 全局前置 | Surface runtime、Action 生命周期、snapshot/patch/resync、native/FFI 批量桥 | 双 surface、epoch、gap、ABI parity |
| P2 | G-01 | 文本发送纵切，Angular/Mobile 删除旧发送解释器 | L0-L4 + 重启/echo/失败恢复 |
| P3 | G-02..G-09 | 文件、媒体、重发、撤回、转发、快捷回复、加急、定时/取消 | 每 Gate fixture/parity/shadow/负向/real-chain |
| P4 | G-10..G-12 | 首屏、历史、定位、回复窗口 | snapshot=patch + anchor/order/reply parity |
| P5 | G-13/G-14/G-14a | 未读、回执、多设备收敛 | ChannelMember 权威 + 原子 frame + 重启 |
| P6 | G-15/G-16 系列 | 建群、话题、关闭、退出、成员、设置 | 权限/action/fallback/system row parity |
| P7 | 27 迁移 Gate + 46 全量 cases | interaction 重跑、全仓 no-regression、删除 shadow/legacy、发布证据收口 | Strict Matrix + PC/FFI interaction Green + regression 无倒退 |

### 16.2 角色与 QA 闸门

| 角色 | 责任 |
|---|---|
| Rust dev | 唯一主线 owner：`helix-im` 状态机、必要的 core 通用机制、host/native/FFI；同一阶段避免多 agent 并写身份/发送主链 |
| Angular dev | `cses-client` generated store/adapter/registry/directive executor，逐 UC 删除 ProjectionStore/Shell 业务 reducer |
| Mobile/FFI dev | `mobile-qucik-c++` 批量 frame bridge、generated reducer、QuickJS/Flutter 消费适配，删除 raw event 解释 |
| 独立 Rust QA | 不与实现者自验自判；负责确定性回放、故障注入、ABI ownership/no-unwind、native/FFI parity |
| Dioxus-dev reviewer | 每阶段只读审查 stable identity、keyed mutation、批次边界、renderer/executor 分离和 O(n²) 风险 |
| Crux-dev reviewer | 每阶段只读审查 Event → Model → Effect/PortReply → View、状态所有权、typed contract 与 shell 边界 |

强制 QA：Persist 失败不得 present/HTTP；HTTP 200 无 echo 不得 sent；重复/乱序/缺帧/旧 epoch/旧 action fail closed；延迟 reset 不得清新输入；两个 surface 隔离；snapshot 与 patch 最终状态等价；破坏 Helix projector 后 PC/Mobile 必须同时失败；每个 UC 分别保留 HTTP、WS、Storage、InteractionFrame 和可见 UI 证据。
