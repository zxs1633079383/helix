# GS01-GS04 - 同步到 Frame 的完整时序与数据

> 状态：GS01-GS04 的目标实施合同。旧 `im:channels:loaded`、`im:channel:increment`、`im:channel:update`、`im:sync:*` 是迁移前输入证据，不是 V2 下行。

## 1. 共用对象与不变量

```rust
enum SyncTrigger {
    BootstrapHello,
    ServerNotify { channel_id: Option<ChannelId>, event_seq: Option<Seq> },
    PongGap { gaps: Vec<ChannelGap> },
    UnknownChannel { channel_id: ChannelId, cause_event_seq: Seq },
}

struct SyncBatchFacts {
    channels: Vec<ChannelFact>,
    members: Vec<ChannelMemberFact>,
    messages: Vec<MessageFact>,
    viewer_states: Vec<ChannelViewerStateFact>,
    cursor_advances: Vec<CursorAdvance>,
}
```

所有 GS Gate 都遵守：

```text
HTTP/WS 事实解析成功
  != 可见 UI

同批 Channel/Member/Message/ViewerState/Cursor PersistAtomic 成功
  -> PersistOk
  -> SurfaceComposer
  -> InteractionFrame v2
```

cursor 不能先于对应消息/频道事实提交。Persist error 时不发最终 Frame，也不推进 cache revision。

## 2. GS01 - hello 与初始增量

### 2.1 目的

登录后的第一批频道、成员、消息和 cursor 进入 Helix，并形成当前用户的 dialog/unread snapshot。UI 全局显示“同步中”，但页面不使用 raw increment 拼 Row。

### 2.2 Tick 到 Effect

| 次序 | 输入 / Tick | Helix 领域动作 | Effect |
|---|---|---|---|
| GS01-1 | `Tick::Connected` | 创建 session，runtime=`connecting/idle/idle`。 | 无业务 Frame。 |
| GS01-2 | `Tick::Inbound(hello)` | 绑定 auth actor、session epoch；初始化 attach 的 user scope。 | `Storage::Scan(channel_event_cursor)`。 |
| GS01-3 | `Tick::PortReply(cursor scan)` | 构造 cursor 集。 | `Http::LoadIncrement { cursors }`；runtime Frame=`online/syncing/idle`。 |
| GS01-4 | `Tick::PortReply(load increment)` 与随后 `increment_channel* / increment_end` 入站 | 只累计 `SyncBatchFacts`，不投影、不发 Row。 | `PersistAtomic(channel, member, message, viewer state, cursor)`。 |
| GS01-5 | `Tick::PortReply(PersistOk)` | 将 batch 标成 committed。 | `SurfaceComposer::compose_bootstrap` -> Frame。 |

### 2.3 Persist 写集

```text
channel                    频道存在、生命周期、排序所需事实
channel_member             成员与角色
message                    已包含在初始/增量中的消息
channel_viewer_state       个人未读、@我、加急、阅读水位
channel_event_cursor       仅在前四类同批成功后推进
```

### 2.4 Composer 影响面和 Frame

已 attach 的 `message.left-dialog-list@user` 与 `message.unread-summary@user` 必须同帧 snapshot；两者不是相互 reduce。

```text
InteractionFrame v2 {
  runtime: { connection: online, initialSync: ready, recovery: idle },
  mutations: [
    dialogListSnapshot(DialogListVm { sections, state: ready }),
    unreadSummarySnapshot(UnreadSummaryVm { ... })
  ]
}
```

如果 route 已恢复到频道 C，C 的 `channel-summary + composer` 以同一频道 interaction bundle snapshot 下发；timeline 的 60 条窗口可在该 bundle 后另帧完成。

### 2.5 PC/Mobile UI

```text
runtime.initialSync=syncing -> 全局“同步中”提示
Frame atomically applied      -> 左侧列表和未读数字同时出现
runtime.initialSync=ready     -> 移除全局初始加载状态
```

UI 不遍历 `increment_channel`、不读 cursor、也不根据 member/raw post 决定 dialog 顺序。

## 3. GS02 - sync notify 离线补事件

### 3.1 触发与 Tick

```text
HostObservation::ServerSyncNotify 或 WS/inbound 缺事件提示
  -> Tick::Inbound
  -> SyncTrigger::ServerNotify
  -> Effect::Http(SyncNotify { affected cursors })
  -> Tick::PortReply(sync entries)
  -> Effect::PersistAtomic(SyncBatchFacts)
  -> Tick::PortReply(PersistOk)
  -> SurfaceComposer -> Frame
```

HTTP `SUCCESS` 只代表拿到候选事实；它本身不能更新 UI 或 cursor。

### 3.2 Frame 选择规则

| 已提交事实 | 受影响 slot |
|---|---|
| 新 Message 成为频道最新 preview | `left-dialog-list@viewer`。 |
| ViewerState 的未读/@我/加急改变 | `left-dialog-list@viewer` + `unread-summary@viewer` 同帧。 |
| 频道 C 的 timeline 已 attach 且消息属于该窗口 | `timeline@C,window`。 |
| 对应 thread 已 attach 且是其 reply | `thread-detail@C,root`。 |
| 频道资料/权限真实变化 | `channel-summary@C` 与必要的 `composer@C`。 |

`runtime.recovery` 在 sync 开始为 `syncing`，持久化并投影完成后为 `recovered`；它与被影响 Surface 在同一个 Frame Cache 事务中提交。

## 4. GS03 - pong gap 补偿

### 4.1 关键区别

`pong`/gap 不是消息 UI 事实。它只触发 `SyncTrigger::PongGap`，随后走与 GS02 相同的 HTTP、Persist、Composer 链。禁止把 pong payload 直接变成 timeline/dialog Row。

### 4.2 时序

```text
Tick::Inbound(pong with gap)
  -> runtime Frame { recovery: syncing }        // 全局提示，不冻结操作
  -> Effect::Http(SyncNotify)
  -> Tick::PortReply(entries | too_long)
  -> PersistAtomic(facts + cursor) 或 reset/reload plan
  -> PersistOk
  -> Frame { changed surfaces, recovery: recovered }
```

`too_long` 不允许尝试本地补洞或假装 cursor 已追平：重置受影响 channel 的同步计划，拉取权威有界/全量数据，最后仍由 PersistOk 后的 snapshot/patch 恢复。

### 4.3 UI 行为

全局显示“同步中”，但已有 Frame Cache 保持可读可操作。用户可以在已授权频道继续发送；未知频道的授权不因全局同步状态被臆造。

## 5. GS04 - 陌生频道 hydration

### 5.1 触发

收到 `post`、深链或 sync entry 指向频道 C，但本地没有足够 Channel/Member 事实时：

```text
Tick::Inbound(unknown channel C)
  -> mark HydrationPending(C)
  -> Effect::Http(LoadIncrementByChannelId { C })
```

禁止先把孤儿 Message 插入任何 timeline/dialog；也禁止客户端用 channel id 拼一个临时频道行。

### 5.2 Commit 和 Frame

```text
Tick::PortReply(single channel increment)
  -> PersistAtomic(Channel C, members, messages, viewer state, cursor)
  -> PersistOk
  -> SurfaceComposer
  -> Frame:
       dialog/unread（如当前 viewer 受影响）
       summary@C + composer@C（如果用户进入/打开 C）
       timeline@C（如果对应 window 已 attach）
```

在 `summary + composer` bundle snapshot 原子 apply 前，C 的编辑器没有有效 `actionRef`，因此不可发送。Hydration 成功后频道 UI 重渲染，编辑器恢复可用；其他频道不受影响。

## 6. GS Gate 的失败、重复和 resync

| 情形 | Helix | Frame Cache / UI |
|---|---|---|
| HTTP/Parse/Persist error | 不推进 cursor，不发布最终业务 Frame；runtime 标 `failed` 或保留 `syncing` 的可诊断状态。 | 保留最后成功 Cache，显示全局同步状态。 |
| 同一 event 重复 | 依据 event/cursor/message identity 幂等；不重复排入 Row。 | 同 revision/transaction 幂等忽略。 |
| Frame revision gap | 不试图本地回放 raw 事件。 | 对该 `(surface, scope)` 请求 snapshot resync。 |
| session/identity 变化 | 清理旧 attach epoch、action grants、未提交 correlation。 | 丢弃旧 session 的 Frame，重新 attach。 |

## 7. GS01-GS04 验收输出

每个 Gate 至少要有：Tick->Effect fixture、Persist barrier 负向 fixture、预期 v2 Frame fixture、PC apply snapshot、Mobile apply snapshot、runtime “同步中” UI 证据。真实链状态必须在 `09` 单列记录，不能由文档或旧 raw emit 代替。
