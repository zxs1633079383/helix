# G01 - 文本发送到 Frame 的完整时序与数据

> 状态：G01 的目标实施合同。当前旧链的 `im_send_message -> im:post:sending / im:post:received` 不属于 V2 终局实现。

## 1. G01 的 Surface owner

| 责任 | Owner |
|---|---|
| “当前频道能否发送、可传哪些内容” | `message.composer@Channel(channelId)`。 |
| “这条消息正在发送/已发送/失败” | `message.timeline` 的 `MessageRowVm.delivery`；已打开 thread 时同步其 `MessageRowVm`。 |
| 最新 preview、排序 | `message.left-dialog-list`。 |
| 草稿、光标、按回车、动画 | PC/Mobile UI overlay，不进入业务 VM。 |
| `temporaryId -> serverId`、HTTP/WS 对账 | Helix Domain State，端侧不参与。 |

`message.composer` 是能力目录，不把 `sendText/sendDocument/sendMedia` 写成会无限增长的根字段：它发布 `message.create` capability 和 `im.message-content.v2` 输入合同。文本只是该合同的 `content.kind=text`。

## 2. 进入频道时的初始 Frame

```text
HostObservation::EnterChannel(C, windowHint)
  -> Tick::Command
  -> 读取/恢复 Channel C + ViewerState
  -> SurfaceComposer
  -> InteractionFrame v2 #channel-bundle:
       channelSummarySnapshot@C
       composerSnapshot@C
```

`MessageComposerVm` 的最小形状：

```rust
struct MessageComposerVm {
    availability: ComposerAvailabilityVm, // enabled | disabled(reason)
    capabilities: Vec<ComposerCapabilityVm>,
}

struct ComposerCapabilityVm {
    capability: "message.create",
    binding: ActionBinding,
    input_contract: "im.message-content.v2",
}
```

`ActionBinding` 固定 actor、surface=`message.composer`、scope=C、surface epoch、observed revision、权限和过期策略。timeline 的翻页、窗口替换不改变它；Channel 生命周期、成员资格、禁言或 policy 改变才会刷新 composer Frame 和 actionRef。

## 3. 上行数据

```rust
struct ActionInvocationEnvelopeV2 {
    schema_version: 2,
    request_id: RequestId,
    actor_id: ActorId,             // trusted host 注入
    surface: "message.composer",
    scope: Channel(C),
    surface_epoch: Epoch,
    observed_revision: Revision,
    invocation: {
        action_ref: OpaqueActionRef,
        input: CreateMessageInputV2,
    },
}

struct CreateMessageInputV2 {
    content: { kind: "text", schema_version: 2, text: DisplayText },
    reply: Option<ReplyReference>,
}
```

输入不含 `channelId`、`userId`、temporaryId、HTTP URL、权限或 action kind；它们都已由 binding/scope/host 身份固定。`reply` 是 UI local selection 的提交结果，Helix 必须再次校验其可见性、频道归属和 root 关系。

## 4. 从回车到 Sending 的逐 Tick 时序

| 次序 | 平台 / Helix 输入 | Domain 决策 | 输出 |
|---|---|---|---|
| G01-0 | 用户回车 | UI 立即清空可见编辑器；把本次文本保留为 request-scoped recovery draft，直到出现 Sending Frame。 | `ActionInvocationEnvelopeV2`。 |
| G01-1 | `Tick::Command(invoke)` | 校验 actionRef、actor、scope、epoch、revision、`message.create` 内容 codec、reply。分配/取得 `temporaryId`，登记 `PendingSend(requestId, temporaryId)`。 | `InteractionActionReceiptV2(accepted)` + `Effect::PersistAtomic`。 |
| G01-2 | `Effect::PersistAtomic` | 写入临时 Message、PendingSend、需要的本地关联事实；此时仍未让业务 UI 可见。 | driver storage 操作。 |
| G01-3 | `Tick::PortReply(PersistOk)` | temporary Message 成为可恢复的 `Sending` 领域事实。 | `InteractionFrame v2 #sending` + `Effect::Http(CreatePost)` + `Effect::ScheduleTimer(send timeout)`。 |
| G01-4 | PC/Mobile apply #sending | 全帧验证并原子替换。 | `FrameApplied`；随后执行 `ClearComposerDraft(requestId)`。 |
| G01-5 | `Tick::PortReply(Http 2xx)` | 仅记录 transport 已受理；不把 delivery 改为 `Sent`。 | 无终态 Frame。 |
| G01-6 | `Tick::Inbound(WS post/posted echo)` | 以 requestId/temporaryId/serverId 关联，归一服务端 canonical Message。 | `Effect::PersistAtomic(canonical message + identity settle + cursor)`。 |
| G01-7 | `Tick::PortReply(PersistOk)` | canonical `Sent` 事实可见。 | `InteractionFrame v2 #sent`。 |

如果 G01-1 被拒绝或 G01-2 Persist 失败，在没有 Sending Frame 的情况下恢复该 request-scoped draft 并显示失败；不静默吞字。回车清空是 UI 行为，业务内容仍只以 PersistOk 后的消息行作为权威可见态。

## 5. Persist 写集与身份

```text
temporary Message
  id = temporaryId
  channelId = C
  delivery = Sending
  content = Text(...)
  requestId = R

PendingSend
  requestId = R
  temporaryId = T
  state = awaiting_echo

canonical settle
  T -> serverId
  canonical content / timestamps / server fields
  delivery = Sent
```

所有查询/WS/HTTP 的关联都在 Helix 内完成。PC/Mobile 只看第一帧中的 `MessageRowVm{id=T, delivery=Sending}` 和第二帧中的已 settle identity；不维护 temporaryId map。

## 6. SurfaceComposer 影响图

### 6.1 发送者 PersistOk

```text
CommittedFactSet:
  MessageCreated(T, C, Sending)
  ChannelPreviewChanged(C)            // 仅当 preview/sort 事实确实变化
  PendingSendCreated(R, T)

Affected attached slots:
  timeline@C,window                   -> 必须（当前发送页）
  left-dialog-list@viewer             -> 必须
  thread-detail@C,root                -> 仅 reply 且已 attach
  unread-summary@sender               -> N/A
  composer@C                          -> 通常 N/A，capability 未变
```

### 6.2 Sending Frame 的完整形状

```text
InteractionFrame v2 {
  transactionId: "persist:R",
  runtime: { connection: online, initialSync: ready, recovery: idle },
  mutations: [
    timelinePatch@C/window(replaceView(
      TimelineWindowVm { items: [..., MessageRowVm {
        id: T,
        content: Text(...),
        delivery: Sending,
        actions: []
      }] }
    )),
    dialogListPatch@viewer(replaceView(
      DialogListVm { row(C).preview = Text(...), row(C).lastActivityAt = localActivity }
    )),
    threadDetailPatch@C/root(...) // only when applicable
  ],
  directives: [ ClearComposerDraft { requestId: R } ]
}
```

Frame Cache 必须先把所有 mutation 与 runtime 一起 apply，才执行清草稿 directive。directive 失败不回滚已持久化的 Sending Row；它只留下可诊断的 UI-local 恢复项。

### 6.3 HTTP 成功后的 WS echo

```text
CommittedFactSet:
  MessageCanonicalized(T -> serverId)
  ChannelPreviewChanged(C, canonical)
  CursorAdvanced(C)

Frame:
  timeline@C/window -> 同一逻辑行 Sending -> Sent
  dialog@viewer     -> canonical preview / canonical ordering
  thread@C/root     -> 条件更新
```

HTTP 2xx 没有对应的 `Sent` Frame；WS echo 或权威 sync 才有。

### 6.4 超时、远端失败、重复回声

| 输入 | Persist 后 Frame | Row action |
|---|---|---|
| timeout / retryable remote failure | `delivery=Failed { reason, retryable=true }`；dialog preview 保持同一逻辑消息。 | timeline 该行出现 `retrySend` binding。 |
| non-retryable failure | `delivery=Failed { retryable=false }`。 | 无 retry 或 disabled reason。 |
| duplicate echo | 不再产生第二行/第二次 identity settle。 | 无新增 Frame，或同 revision 幂等。 |
| echo 晚于 sync canonical facts | 以 canonical identity/seq 去重。 | 只保留一个 `Sent` 行。 |

## 7. 接收者视角

发送者的 `Sending` 绝不泄漏给其他 viewer。接收者只在其 own Helix session 得到 WS/sync 后：

```text
PersistAtomic(canonical Message + receiver ViewerState)
  -> PersistOk
  -> Frame:
       dialog list preview
       unread summary（如果未读增加）
       attached timeline/thread
```

这是两个本地原子 Frame，不是跨设备分布式 UI 事务。

## 8. PC/Mobile 渲染

```text
timeline MessageRowVm.delivery=Sending -> 发送中标记
timeline MessageRowVm.delivery=Sent    -> 已发送内容
timeline MessageRowVm.delivery=Failed  -> 失败提示与 retry action
dialog row preview                       -> 最新预览/排序
composer directive                        -> 清输入框
```

平台不得手动 insert 行、移动 dialog、把 HTTP 2xx 改为 Sent、删除 temporaryId 或从 draft 补回业务消息。

## 9. G01 验收

必须有：ActionBinding authorization fixture、Tick/Effect/Persist barrier fixture、Sending/Sent/Failed 三个 Frame fixture、duplicate/late echo fixture、PC/Mobile apply parity、双用户隔离 fixture。真实链证据状态在 `09` 维护。
