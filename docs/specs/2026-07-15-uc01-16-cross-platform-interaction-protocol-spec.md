# UC01-16 跨端交互协议与呈现状态机 SPEC

> 状态：Validated Design / Implementation Pending
> 日期：2026-07-15
> 取代：`2026-07-15-uc1-16-cross-platform-render-only-projection-spec.md`
> 范围：Message V4 宏 UC01-16；Strict Matrix `G-01..G-16c` 与同步前置 Gate
> 真源：Strict Matrix contract epoch 4、`pd/message` UC 设计真源、Helix/cses-client/mobile 当前代码
> Review：`dioxus-dev` 与 `crux-dev` 两个独立 lens，结论已回填
> 边界：本文是经过代码查证的目标合同，不把当前 `declared/blocked` 误报为运行时 Green。

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
4. Helix 不输出 DOM、Angular component、Flutter widget、CSS selector、像素或动画；它输出平台无关的语义呈现合同。
5. 一个领域事实造成的跨实体 UI 变化必须放入同一 `InteractionFrame` 原子提交。
6. 可见业务状态必须经过 `PersistOk` 后发布；HTTP 200 不是发送完成，WS/sync 权威回声才是。

## 1. 真源与 UC-16 查证

### 1.1 权威顺序

发生语义冲突时按以下顺序停机对账，不由客户端代码反向定义合同：

1. `crates/helix-im/contracts/strict-matrix.v1.json`
2. `crates/helix-im/docs/Strict Matrix/UC全链路发布矩阵-v1.md`
3. `/System/Volumes/Data/workspace/pd/message/docs/message-v4-real-chain-ui/`
4. Helix 当前实现与 `前端UC投影指令与数据清单-最新代码分析-2026-07-09.md`
5. cses-client 与 mobile 当前代码只作为迁移证据，不作为目标架构真源

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

所有类型由 Rust schema 生成 TS、JavaScript、C++、Dart bindings/reducer，禁止各端手写第二套字段解释。

```rust
struct ProtocolEnvelope<T> {
    protocol_version: u16,
    engine_session_id: String,
    account_key: String,
    surface_id: String,
    surface_epoch: u64,
    trace_id: String,
    payload: T,
}
```

- `engine_session_id`：Helix 本次进程/恢复会话标识。
- `surface_id`：窗口、页面或嵌入面实例；不等于 channel ID。
- `surface_epoch`：同一 surface 重建时递增，旧 epoch 输入 fail closed。
- `trace_id`：逐跳 correlation，不承担业务身份。

### 4.2 上行 `UiEvent`

```rust
enum UiEvent {
    SurfaceAttached {
        surface_kind: String,
        capabilities: Vec<String>,
    },
    SurfaceDetached,
    ActionInvoked {
        action_ref: String,
        observed_revision: u64,
        input: Value,
    },
    ViewportObserved {
        ordered_visible_node_keys: Vec<String>,
        first_visible_node_key: Option<String>,
        last_visible_node_key: Option<String>,
        app_visibility: Visibility,
        window_focus: bool,
    },
    HostStateObserved {
        app_visibility: Visibility,
        window_focus: bool,
        connectivity: Connectivity,
    },
    FrameApplied { revision: u64 },
    DirectiveApplied { directive_id: String, outcome: ApplyOutcome },
    ResyncRequested { observed_revision: u64, reason: String },
}
```

`ActionInvoked` 的 `input` 只携带动作绑定声明允许的原始表单值或 opaque host handle。它不携带 command name、channel ID、temporary ID、HTTP body。`ViewportObserved` 上报 projection node key，不上报 UI 自己筛出的“应标已读 message IDs”。

### 4.3 下行 `ActionBinding`

```rust
struct ActionBinding {
    action_ref: String,
    owner_node_key: String,
    action_kind: String,
    enabled: bool,
    disabled_reason: Option<LocalizedText>,
    input_contract: InputContract,
    valid_at_revision: u64,
    replay_policy: ReplayPolicy,
}
```

`action_ref` 是 opaque、surface-scoped、revision-aware 的动作地址。Helix 内部绑定领域上下文和 use case。UI 只渲染按钮并回送引用。

例子：群设置页可以同时收到 `channel.close` 和 `channel.leave` 两种语义 kind，但只会收到当前用户有权执行且绑定了独立 `action_ref` 的动作。UI 不根据角色做分支，也看不到对应的 `im_channel_*` 命令。

### 4.4 下行 `InteractionFrame`

```rust
struct InteractionFrame {
    frame_id: String,
    causation_id: String,
    mode: FrameMode,              // Snapshot | Patch
    base_revision: u64,
    revision: u64,
    mutations: Vec<ViewMutation>,
    action_bindings: Vec<ActionBinding>,
    directives: Vec<UiDirective>,
}

enum ViewMutation {
    UpsertNode { node: ViewNode },
    RemoveNode { node_key: String },
    SetChildren {
        parent_key: String,
        ordered_child_keys: Vec<String>,
    },
    RemoveSubtree { root_key: String },
}

struct ViewNode {
    node_key: String,
    render_kind: String,
    revision: u64,
    props: Value,
    action_refs: Vec<String>,
}
```

约束：

- 一个 frame 内 mutation 按序、原子 apply；应用前不允许组件观察到半帧状态。
- `SetChildren` 给出完整顺序，端侧不得再次排序、去重、slice 或 prepend。
- 节点身份由 `node_key` 稳定表达；server ID 变化不能迫使端侧维护映射。
- Snapshot 权威替换整个 surface；Patch 必须满足 `base_revision == local_revision`。
- gap、未知 epoch、未知 action_ref 一律 fail closed 并请求 snapshot。

### 4.5 一次性 `UiDirective`

```rust
struct UiDirective {
    directive_id: String,
    kind: DirectiveKind,
    replay_policy: DirectiveReplayPolicy,
}

enum DirectiveKind {
    ResetInput { node_key: String },
    FocusNode { node_key: String },
    RevealNode { node_key: String, alignment: Alignment },
    SelectSurfaceEntity { node_key: String },
    DismissSurfaceRegion { node_key: String },
    AnnounceOutcome { operation_key: String },
}
```

Directive 表达跨端“做什么”，各端决定“怎么做”。它不得出现 DOM selector、Widget key、CSS、px、动画时长。持久业务结果必须同时存在于 ViewModel，不能只靠一次性 directive。

重复 directive 通过 `directive_id + ack` 幂等处理；`replay_policy` 决定重连后是重放、丢弃还是等待确认。

### 4.6 HostEffect 与 UiDirective 不能混

| 类型 | 执行者 | 例子 | 是否业务权威 |
|---|---|---|---|
| `HostEffect` | native/FFI driver | Storage、HTTP、timer、文件读取 | 结果必须回到 Helix |
| `ViewMutation` | UI frame reducer | Upsert/Remove/SetChildren | Helix ViewModel 的副本 |
| `UiDirective` | platform renderer | focus/reveal/reset/dismiss | 否，必须可 ack |

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

当前 `handle_send_message` 仍要求调用方传 `temporary_id`，并在同一次 step 中先后吐 `Persist` 与 Emit；cses-client 与 Mobile 都还在生成 temporary ID。实现必须先完成确定性 IdSource/AllocateId 机制和 `PersistOk -> frame -> HTTP` 分阶段状态机，旧 Spec 中的 `ids.next_id()` 伪代码不能算完成。

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

## 12. 分阶段迁移

### P0 合同冻结与真实缺口

1. 冻结 `InteractionProtocol/v1`、surface/revision/ack/resync、action/directive 语义。
2. 冻结 deterministic ID 机制，修正文档和当前 send 入参冲突。
3. 补 `PersistOk -> frame -> HTTP` 发送状态机。
4. 补 ChannelMember 权威未读、dialog overlay 和 type6 sync 闭环。
5. 建同一 schema 的 bindings/reducer generator 与跨语言 fixture。

### P1 双端运行壳

1. native/FFI 增加批量 frame、UiEvent、ack/resync 通道。
2. 两个 surface 并行、epoch 重建、snapshot recovery 先跑通。
3. 保留旧投影只作对照，不允许新功能继续写旧解释器。

### P2 纵切 G-01

用发送消息证明 action_ref、ID、persist barrier、sending/sent/failed、reconcile、reset directive 与重启恢复闭环。通过后删除 Angular/Mobile 的发送业务逻辑。

### P3 窗口与回复

迁移 G-10/G-10a/G-11/G-12，删除各端 merge/sort/window/reply index。

### P4 未读

迁移 G-13/G-14/G-14a，完成三个未读/回执对象的权威关系和原子 frame，删除各端 read controller。

### P5 群治理

迁移 G-15/G-16 系列，验证 close/leave/remove/settings 独立 action 和 fallback directive。

### P6 UC01-16 收口

迁移剩余 Gate，删除 raw command/event/store 入口；Strict Matrix 补 interaction contract 证据面，PC/FFI 同输入必须输出等价 frame。

## 13. 验收合同

### 13.1 协议与确定性

- Rust/TS/JS/C++/Dart 类型来自同一 schema，字段漂移检查 fail closed。
- 相同 UiEvent/HostObservation/EffectResult/WS 序列在 native 与 FFI 输出字节等价 frame。
- 同一 frame fixture 经 TS/JS/C++/Dart reducer 得到等价快照。
- 重复 frame 幂等；revision gap 必须请求 snapshot；stale action/epoch 必须拒绝。
- 两个 surface 同时打开不同频道，不共享全局 active channel。

### 13.2 G-01

- Persist 失败时 UI 看不到 sending，HTTP/timer 不执行。
- HTTP 200 无 WS/sync 回声时不得变 sent。
- kill/restart 后 pending/failed/sent 与 frame snapshot 一致。
- UI/QuickJS/C++ 搜不到 temp ID 生成、server ID reconcile、raw send command。

### 13.3 G-13/G-14

- UI 只上报 viewport/focus/visibility，不能提交 read candidate 或先清 badge。
- 重启后 badge 从 current viewer ChannelMember 权威状态恢复。
- `message.read_bits` 单独变化不能清频道 badge。
- 一次 read settlement 的 dialog/total/receipt 变化处于同一 frame。
- 多设备 read 通过绝对状态收敛，旧 revision 不回滚。

### 13.4 UC16

- close、self leave、admin remove 是不同 action_ref 和不同领域结果。
- 678 等无权限账号收不到 enabled 的管理 binding，UI 不写角色判断。
- 当前频道关闭/离开后由 directive 指向 Helix 选定的 fallback。
- member list、channel lifecycle、settings version、system row 同帧收敛。

### 13.5 负向静态闸门

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
4. G-01、G-13/G-14、G-16 三条代表链路通过真实双端、重启、断帧和负向破坏验证。
5. UC01-16 其余 Gate 逐条拥有相同 interaction contract 证据，且 raw command/event/业务 reducer 已从各端删除。

这才是目标边界：**Helix 产出各端可直接消费的交互合同，各端只负责平台事实采集、Effect 执行、frame 应用与 UI 渲染。**
