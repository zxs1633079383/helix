# InteractionFrame v2 - Frame Cache 原子替换与三端渲染合同

> 状态：跨端消费端的目标合同。它规定 PC/Mobile 如何机械消费，不规定具体 Angular component、Flutter Widget、CSS 或动画。

## 1. 三层职责

```text
helix-im
  领域事实、ActionBinding、SurfaceComposer、InteractionFrame v2

PC/Mobile Frame Cache
  decode、schema/epoch/revision 校验、copy-on-write 原子提交、ack/resync

PC/Mobile renderer
  读取 readonly VM、映射语义 renderer key、处理本地 overlay 和 directive
```

Frame Cache 绝不拥有 MessageStore/ChannelStore、temporaryId map、未读计算、排序器、权限判断或 raw event reducer。

## 2. V2 wire 规则

### 2.1 九个 Surface discriminator

```text
message.left-dialog-list
message.unread-summary
message.timeline
message.thread-detail
message.channel-summary
message.member-roster
message.topic-list
message.participant-receipts
message.composer
```

`message.composer` 的 scope 与 `message.channel-summary` 都是 `Channel(channelId)`，但它们是两个独立 slot、独立 revision、独立 VM；前者是交互能力，后者是频道资料/设置。

### 2.2 V2 VM 复审准则

| 准则 | V2 结论 |
|---|---|
| 跨 Surface 重复 | 允许。每个 attached slot 必须无需客户端 join 其他业务 Store。 |
| 根 VM 不相关字段膨胀 | 禁止。用少量一起变化的语义块，而非 feature-by-feature 根属性。 |
| `SurfaceDataVm` / `unknown` | 禁止出现在 v2 下行和上行业务字段。 |
| 新消息种类 | 只扩展 versioned `MessageContent` / input codec registry；不添加 composer 根字段或 global store。 |
| `actions: Vec<ActionBinding>` | 允许且有界；binding 与所属 Row/VM 同一 Frame 更新，避免旧权限地址悬挂。 |
| Thread 双索引 | V2 优先有序 `Vec<MessageRowVm>`；仅当某个合同确实要求 O(1) 节点寻址时才同时传 id index，不能无理由传 `orderedIds + nodes` 两份结构。 |

建议的 Message Row 语义分组：

```rust
struct MessageRowVmV2 {
    identity: MessageIdentityVm,
    author: UserRefVm,
    created_at: Timestamp,
    content: VersionedMessageContentVm,
    delivery: DeliveryVm,
    relationships: MessageRelationshipsVm, // reply/topic
    indicators: MessageIndicatorsVm,       // receipt/urgent/pin/bookmark
    actions: Vec<ActionBinding>,
}
```

这是结构化，不是制造多余数据：每一块有独立的校验、渲染和演进理由。User profile 继续只传 `userId` 引用，交给 Ucenter 展示。

## 3. Frame Cache slot 模型

```rust
type SlotKey = (SurfaceId, CanonicalScope);

struct CachedSlot<TVm> {
    surface_epoch: Epoch,
    revision: Revision,
    vm: TVm,
}

struct FrameCache {
    runtime: RuntimeStatusVm,
    slots: Map<SlotKey, CachedSlot<typed VM>>,
}
```

route/attach 只让平台表示“我希望读取这个 scope”；Helix 才签发 epoch、revision、binding 和 VM。客户端不能自行递增 revision 或重用旧 actionRef。

## 4. 原子 apply 算法

```text
1. decode v2；未知 schema、surface、scope、content kind 或字段 -> 拒绝整帧。
2. 对每个 mutation 验证：scope 与 surface 匹配、epoch、revision、baseRevision、typed VM。
3. 在临时副本上应用全部 mutation：
   - snapshot：仅可建立新 slot/新 epoch；
   - patch：baseRevision 必须精确命中；v2 当前 patch 仍是 replaceView(完整有限 VM)。
4. 所有 mutation 和 runtime 均成功 -> 一次提交副本；失败 -> 一个 slot 也不改。
5. 提交后发 FrameApplied(transactionId)，不等待它才继续业务。
6. 按声明顺序执行 directives；每个结果发 DirectiveApplied(directiveId)。
```

复杂度是 `O(k)`，`k` 是 Frame mutation 数；单个 list/window 的 replace 限制在其有界大小。任何 base revision gap 都不回放 legacy event，而是对受影响 `(surface, scope)` 请求 snapshot resync。

## 5. runtime 与全局同步 UI

`runtime` 与 surface slots 一起原子提交。

```text
runtime.initialSync=syncing 或 runtime.recovery=syncing
  -> PC/Mobile 全局显示“同步中”
  -> 已有 Surface 仍可读、已有有效 binding 仍可操作

runtime.initialSync=ready / recovery=recovered
  -> 全局提示结束
```

陌生频道例外：没有该频道 `channel-summary + composer` bundle snapshot 前，平台没有 actionRef，编辑器不可发送；hydration Frame apply 后恢复该频道发送。全局同步状态本身不冻结其他频道。

## 6. directive 和 ack

| 类型 | 语义 | 何时执行 | 失败后果 |
|---|---|---|---|
| `ClearComposerDraft(requestId)` | 清本地编辑器文字。 | 对应 Sending Frame 成功 apply 后。 | 不回滚 Sending Row；保留 diagnostics/recovery draft。 |
| `RevealMessage(messageId)` | 显示/滚到已存在的消息行。 | Frame 中目标 row 已存在后。 | 不改业务 VM，可重试或丢弃。 |
| `FocusComposer` | 使当前编辑器取得平台焦点。 | composer slot 已 apply 后。 | 不改业务 VM。 |

`FrameApplied` 表示业务画面换好了；`DirectiveApplied` 表示一次性 UI 动作已尝试。二者都是诊断、去重和恢复信号，不是 Persist/HTTP/WS 的阻塞条件。

## 7. Action invocation 与本地 overlay

```text
renderer 从 readonly VM 读取 ActionBinding
  -> 用户动作
  -> begin local pending overlay(requestId)
  -> 把版本化 ActionInvocationEnvelope 交给 host
  -> ActionReceipt 只改变 overlay 的 accepted/rejected 提示
  -> matching causation 的 Frame 才改变业务 UI
```

草稿、文本选择、文件选择、popover、route、动画、像素滚动都属于 overlay，不能写回 Frame Cache 的业务 VM。G01 回车可以立即清可见输入；若 rejected/Persist fail 早于 Sending Frame，则以 request-scoped recovery draft 恢复。

## 8. PC 和 Mobile renderer 的共同边界

| 共同输入 | PC/Mobile 可以不同 | 两端都不得不同 |
|---|---|---|
| generated `InteractionFrame v2` bytes | Component/Widget、布局、字体、动画、手势。 | decoder、slot key、revision 规则、content kind、ActionBinding、runtime 含义。 |
| typed `MessageRowVmV2` | 气泡、发送中 icon、失败样式。 | `Sending/Sent/Failed` 的领域判断、temporaryId settle。 |
| typed dialog/topic/member rows | 视觉分区、页面结构。 | 排序、未读、权限、preview、成员角色。 |

每个 renderer registry entry 必须是静态、语义 key 到平台实现的映射。没有 registry entry 的 `content.kind` / surface 直接 fail-closed，不可将其降级为 raw JSON 文本或让某端自行猜测。

## 9. resync 与 session 切换

| 条件 | PC/Mobile 行为 | Helix 行为 |
|---|---|---|
| patch baseRevision 不匹配 | 不提交整帧；请求该 slot snapshot。 | 读已持久化事实，生成该 scope snapshot。 |
| unknown/old epoch | 丢弃 mutation；不使用其中 actionRef。 | attach/identity 变化时签新 epoch。 |
| actionRef rejected stale | overlay 标为 rejected；不伪造业务失败行。 | 重新 attach/refresh 必要 capability。 |
| session 更换 | 清空旧用户 slot、overlay、directive 去重表。 | 回收旧 action grant/correlation，建立新 session。 |

## 10. 消费端验收

每个平台针对同一 fixture 要证明：decode 成功、完整 apply 成功、任一 mutation 破坏时全帧不变、resync 请求正确、runtime 全局提示正确、directive 在 FrameApplied 后执行、业务 renderer 无 raw event reducer。
