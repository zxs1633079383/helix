# InteractionFrame V2 Cutover Strict Matrix

**Feature**: `018-interaction-frame-v2-cutover`

**Scope**: `G-S1..G-S4`、`G-01`、`G-02`、`G-04..G-16c`，包含 `G-04a/G-10a/G-14a/G-15a/G-16b/G-16c`，排除 `G-03/G-03a`

**Rows**: 19 个业务 UC，25 个机器 Gate

**Status semantics**:

- `CURRENT`: 当前双仓源码或当前固定 pin 的事实。
- `TARGET`: 本 Feature 必须实现并验证的 InteractionFrame V2 合同。
- `GAP`: 从 CURRENT 到 TARGET 的缺口；没有运行证据不得写 PASS。

## 1. 全局 CURRENT / TARGET / GAP

| Dimension | CURRENT | TARGET | GAP / Gate |
|---|---|---|---|
| Client reader | Angular 同时接受 V1/V2，V1 会 normalize；V2 runtime/directive 已能 apply | 源码迁移期可暂存双读代码，但任何全量 E2E candidate 的生产路径必须 V2-only；首次 E2E 后再物理删除 V1 | `R1` V2 reader 负向矩阵；`R5` V2-only 硬切；`R6` 删除 |
| Helix writer | 通用 builder/emitter 为 schema 1；独立 recovery composer 可发 V2 | 25 个 Gate 全部使用统一 V2 builder/emitter | `W1` 全量 writer |
| Render contract | Strict Matrix 使用 RenderNode/v1 和 `*.row.v1` | 每 Gate 直接声明 typed SurfaceMutationV2、完整 VM、runtime/directive | `C1` 合同升级与生成 parity |
| Action ingress | 公共 opaque action 入口存在；逐 Gate 仍需证明没有命名命令旁路 | 所有用户动作只能由 matching ActionBinding 调用；observation Gate 明确无 binding | `A1` 逐 Gate action/observation trace |
| Persist barrier | 多数业务已有 Persist/功能 E2E；现有 Green 未证明全量 V2 | durable Frame 只在 matching PersistOk 后发布 | `P1` 逐 Gate 正负 fixture |
| Client pin | Helix HEAD、Cargo pin、生成合同 pin 不一致 | revision/hash 四面一致 | `PIN` freeze + repin |
| E2E evidence | 旧 017 为业务功能 Green，没有 schema 计数 | 25/25；V1=0；raw fallback=0；全 hop evidence | `E1` cutover E2E |
| Legacy removal | V1 type/codec/fixtures/producer 仍存在 | 已审计范围物理删除，独立版本合同保留 | `D1` inventory；`D2` delete；`E2` post-delete regression |

## 2. 所有 Gate 的共同强制链

```text
Angular semantic intent
  -> projected ActionBinding OR explicit HostObservation/Inbound
  -> SurfaceActionRequested(actionRef,input) OR host fact
  -> Tick::Command / Tick::Inbound / Tick::PortReply / Tick::Timer
  -> deterministic domain decision
  -> Effect(Storage | Http | WsSend | Timer)
  -> correlated Tick::PortReply / inbound authority fact
  -> PersistAtomic(write-set)
  -> PersistOk
  -> SurfaceComposer(attach-aware impact set)
  -> InteractionFrameV2 {
       schemaVersion=2,
       transactionId,
       causationId?,
       runtime,
       mutations[{surface,scope,surfaceEpoch,revision,baseRevision?,mutation}],
       directives
     }
  -> ProjectionBusService
  -> decodeInteractionFrame(V2)
  -> SurfaceRuntime stage/validate/atomic swap
  -> MessageSurfaceFacade + adapter
  -> MessageComponent signal
  -> typed Angular component
  -> directive after commit
  -> FrameApplied / ActionReceipt
```

任一 durable Gate 若缺 `PersistOk`、完整 V2 Frame、matching Surface、组件终态或负向反证，状态均不是 PASS。

## 3. Helix 细粒度 Gate Matrix

| Gate | ActionBinding / Observation | Tick / domain decision | Effect and authority | Persist barrier | TARGET V2 Frame / Surface | CURRENT / GAP |
|---|---|---|---|---|---|---|
| G-S1 | `hello` HostObservation；无用户 ActionBinding | Connected → Inbound hello → cursor scan reply → increment facts → PersistOk | Storage scan；load increment HTTP/WS batch | channel + member + message + viewer state + cursor 同批；失败不推进 cursor | runtime recovery；attached left-dialog-list/unread-summary，必要 channel snapshots | recovery V2 部分存在；旧 Matrix 仍 `dialog.list.v1`；补 pin 下 V2 fixture 和 V1=0 |
| G-S2 | sync notify Inbound；无用户 ActionBinding | Inbound notify → sync request reply → normalize batch → PersistOk | SyncNotify HTTP/WS authority | affected facts + cursor 同批 | runtime syncing/recovered + impact-policy selected attached surfaces | recovery V2 部分存在；旧 Matrix 为 `message.left-dialog-list.v1` |
| G-S3 | pong gap Inbound；无用户 ActionBinding | Inbound gap → bounded sync/reload → PersistOk | SyncNotify 或 authoritative reload | facts + cursor；too-long 不得猜测推进 | runtime syncing/failed/recovered + recovered attached surfaces | recovery V2 部分存在；旧 Matrix 为 `system.sync.v1` |
| G-S4 | unknown-channel/enter HostObservation；`select_channel` 只来自 dialog binding | Inbound unknown/enter → load single increment → PersistOk | LoadIncrementByChannelId | channel + member + message + viewer state + cursor | channel-summary + composer bundle；timeline 按 attach；必要 dialog/unread | recovery V2 部分存在；旧 Matrix 为 `channel.row.v1` |
| G-01 | composer `message.create` binding；retry/revoke/reply/react 为 projected bindings | Command invoke → validate grant → temporary fact → PersistOk → HTTP → WS echo → PersistOk | local persist；CreatePost；timer；canonical echo | temporary message + pending_send；后续 canonical identity + cursor | composer + timeline + dialog + unread/channel-summary；clear-draft directive only after sending Frame commit | 通用 action/query writer 仍 V1；旧 Matrix `message.row.v1` |
| G-02 | composer typed document create binding | Command invoke → validate typed content → pending/canonical transitions | persist + CreatePost + echo/sync | message + pending_send + canonical settle | same G01 surfaces with typed document content VM | 当前命令存在但 writer/Matrix 为 V1；G03/G03a 上传流程不在本 Feature |
| G-04 | message revoke binding | Command → revoke authority → inbound/update reply → normalize → PersistOk | revoke HTTP；WS/update | message lifecycle/deletion fact | attached timeline/thread/dialog；attached receipts when affected | 功能 E2E Green，但通用 writer/Matrix 仍 `message.row.v1` |
| G-04a | revoke/delete sync Inbound；无用户 binding | Inbound type-3/delete → normalize → PersistOk | sync/WS authority；无重复 revoke HTTP | message + cursor | same authoritative surfaces as G-04 | 功能基线存在；必须证明 V2 且不产生重复 HTTP |
| G-05 | source message relay binding；targets 只能来自 allowed input codec | Command → validate targets/grant → request → canonical target facts → PersistOk | relay/create posts HTTP + WS/sync | target messages + affected channels/cursors | source/target timeline、dialog、composer policy；forward detail directive as needed | 功能 E2E Green；writer/Matrix 仍 `message.row.v1` |
| G-06 | message quick-reply/reaction binding | Command → validate message/grant → authority result → PersistOk | quick reply/reaction HTTP/WS | message/reaction aggregate | timeline/thread；dialog preview only if content impact policy says so | 功能 E2E Green；writer/Matrix 仍 V1 |
| G-07 | urgent and urgent-confirm bindings | Command → urgent authority → receipt/confirm fact → PersistOk | urgent/confirm HTTP/WS | message + message_receipt | timeline + participant-receipts + dialog/unread impact | 功能 E2E Green；writer/Matrix 为 message/read-receipt V1 |
| G-08 | schedule-create binding | Command → validate schedule → authority create → PersistOk | create schedule HTTP/WS/timer registration | channel_schedule + related message fact | channel-summary/dialog；timeline only when scheduled row is visible | 功能 E2E Green；writer/Matrix `channel.row.v1` |
| G-09 | cancel-schedule binding | Command → validate ownership/state → authority cancel → PersistOk | cancel schedule HTTP/WS/timer cancellation | channel_schedule terminal state | channel-summary/dialog + attached scheduled row | 功能 E2E Green；writer/Matrix `channel.row.v1` |
| G-10 | attach/query HostObservation；timeline `load_older/reply` bindings | Command/query → local window decision → snapshot compose | bounded Storage scan；authority fallback only when required | fetched facts before visible if remote used | timeline snapshot with complete bounded window | 功能 E2E Green；local-first builder 仍 V1 |
| G-10a | timeline `load_older` binding | Command → bounded older window → revision/baseRevision decision | Storage scan and bounded remote fetch if needed | new facts before replacement | timeline bounded replacement/patch preserving anchor contract | 功能 E2E Green；writer/Matrix `message.row.v1` |
| G-11 | timeline locate binding | Command → locate authority/local index → ensure target window → PersistOk | getPosts/local lookup | target/context message facts | timeline + `RevealMessage` directive after commit | 功能 E2E Green；旧 Matrix row 无 V2 directive 证明 |
| G-12 | thread open/reply/branch bindings | Command → load root/branch → normalize graph → PersistOk | replies/branch HTTP or local scan | message + reply_tree | thread-detail + root timeline row；open/focus directive as needed | 功能 E2E Green；旧 Matrix `reply.tree.v1` |
| G-13 | channel-read binding/visibility observation | Command/observation → compute authoritative read boundary → PersistOk | channels view/read HTTP or WS | channel viewer state + message_receipt | dialog + unread-summary + timeline/channel-summary impact | 功能 E2E Green；旧 Matrix read/channel V1 |
| G-14 | post-read binding/visibility observation | Command → validate message boundary → receipt fact → PersistOk | post read HTTP/WS | message_receipt | participant-receipts + row summary when attached | 功能 E2E Green；旧 Matrix `read.receipt.v1` |
| G-14a | template-received binding | Command → confirm template → PersistOk | template received HTTP/WS | message_receipt/template confirmation | timeline/thread row summary + participant-receipts | 功能 E2E Green；旧 Matrix `message.row.v1` |
| G-15a | create-channel binding | Command → validate participants → HTTP authority → reconcile → PersistOk | channel create HTTP/WS | channel + channel_member | creator/member dialog + unread + channel-summary/member-roster；then GS4 | 功能 E2E Green；旧 Matrix channel/member V1 |
| G-15 | make-topic binding | Command → validate parent/root → authority create → PersistOk | makeTopic HTTP/WS | channel + message + reply_tree | topic-list + channel-summary + root timeline/thread | 功能 E2E Green；旧 Matrix channel/reply V1 |
| G-16 | close-channel binding | Command → validate role/lifecycle → authority close → PersistOk | close HTTP/WS | channel + channel_member | channel-summary + dialog + composer disabled + detach directives | 功能 E2E Green；旧 Matrix `channel.row.v1` |
| G-16b | leave-channel binding | Command → validate actor membership → authority leave → PersistOk | leave HTTP/WS | channel_member + viewer state | dialog/unread update + member-roster + detach channel surfaces | 功能 E2E Green；旧 Matrix `member.row.v1` |
| G-16c | display-name/notice/info bindings | Command → validate role/input → authority update → PersistOk | settings HTTP/WS | channel | channel-summary + dialog; exact attached surfaces by impact policy | 功能 E2E Green；旧 Matrix `channel.row.v1` |

## 4. Angular 组件与逻辑处理 Matrix

所有行共享入口：

```text
ProjectionBusService
  -> decodeInteractionFrame
  -> ProjectionStore/SurfaceRuntime
  -> MessageSurfaceFacade
  -> per-surface adapter
  -> MessageComponent
  -> MessageSurfaceRuntimeViewComponent / overlay component
```

| Gate | Angular semantic trigger | Adapter / VM | Component owner | Required logic and DOM evidence |
|---|---|---|---|---|
| G-S1 | shell bootstrap/session ready | dialog-list + unread-summary | `message.component`、`message-surface-runtime-view`、`message-ui-surface-dialog-list` | runtime 从 syncing 到 recovered；列表与未读同帧；无 V1/raw reducer |
| G-S2 | sync notification由 host 注入 | impact-selected adapters | `message-ui-state` + runtime view | 保留最后一致 Cache；恢复后原子更新；无重复 row |
| G-S3 | pong gap由 host 注入 | runtime + recovered surfaces | `message-ui-state`、runtime view | 显示同步中但不冻结已有有效操作；gap 时请求 resync |
| G-S4 | dialog select/unknown channel hydration | channel-summary + composer + optional timeline | runtime view、`message-ui-header`、`message-ui-composer` | hydration 前无可执行 send binding；提交后 summary/composer 一致出现 |
| G-01 | composer submit / message bindings | composer + timeline + dialog/unread | `message-ui-composer`、runtime view、surface message content、action bindings | Sending/Sent/Failed 单行收敛；matching directive 后清 draft；action state 可追踪 |
| G-02 | composer typed document submit | composer + timeline | `message-ui-composer`、`message-content-outlet`、`message-document-content` | typed content 渲染，不读 raw payload；失败恢复输入 |
| G-04 | action menu revoke | timeline/thread/dialog | `message-ui-action-menu`、surface message content、thread drawer | 撤回态/系统提示由完整 VM 决定；旧按钮失效 |
| G-04a | 无 UI 上行动作 | same as G-04 | surface message content、dialog list | 离线删除恢复后与在线撤回同态，不重复发命令 |
| G-05 | forward request/action binding | source/target timeline/dialog | `message-ui-forward-dialog`、`message-ui-forward-detail`、surface forward detail | 多目标选择只作为 binding input；每目标终态由 matching Frame |
| G-06 | reaction/quick action | timeline/thread | `message-ui-surface-quick-actions`、`message-ui-action-menu`、surface content | emoji aggregate、选中态和失败态来自 VM，不本地猜计数 |
| G-07 | urgent/confirm | timeline + participant-receipts | `message-ui-urgent`、`message-ui-read-receipt`、runtime content | 加急标识、确认人数和按钮可用性同权威 Frame |
| G-08 | open/submit schedule | channel-summary/dialog | `message-ui-schedule-dialog`、overlay host | submitting/success/failure；成功后 projected schedule，不以 HTTP response 关窗 |
| G-09 | cancel schedule | channel-summary/dialog | schedule dialog、confirm dialog、overlay host | cancel binding 和 terminal Frame 对齐；重复取消幂等 |
| G-10 | attach active channel | timeline adapter | runtime view、surface message content | bounded snapshot、empty/loading/ready；无局部旧 row 拼接 |
| G-10a | loadOlderRequested | timeline adapter | runtime view | 保持 scroll anchor；replacement 原子；revision gap 请求 resync |
| G-11 | locateRequested | timeline adapter + directive | runtime view、`scroll-anchor-store` | 只在 Frame commit 后 reveal/focus 目标；不存在目标时显示可诊断失败 |
| G-12 | threadRequested/reply branch | thread-detail adapter | `message-ui-surface-thread-drawer`、`message-ui-reply-drawer` | root/replies 完整图；reply action 走 binding；关闭只属 UI-local |
| G-13 | channelReadRequested/visibility | dialog/unread/timeline | sidebar/dialog list、`message-read-reporting` | 未读清零与 read boundary 同 Frame；重复上报有界 |
| G-14 | messageReadRequested/receipt open | participant-receipts | `message-ui-read-receipt`、overlay host | readers/unreaders 完整 snapshot；无本地增减计数 |
| G-14a | templateReceivedRequested | timeline + participant-receipts | `message-template-content`、runtime content | 已确认状态和按钮禁用由 matching causation Frame |
| G-15a | createChatRequested | dialog/channel-summary/member-roster | `message-ui-create-chat`、member drawer、runtime view | 创建中/成功/失败；新频道只在持久化 Frame 后进入列表 |
| G-15 | topicCreateRequested | topic-list/thread-detail/timeline | `message-ui-create-topic`、thread drawer、topic content | topic root、列表和 thread 一致；open directive commit 后执行 |
| G-16 | channelCloseRequested | summary/dialog/composer | `message-ui-group-settings-drawer`、channel-management、header | close binding 权限；成功后 composer disabled、Surface detach |
| G-16b | channelLeaveRequested | member-roster/dialog/unread | member drawer、runtime view | 当前用户退出后列表/成员/active channel 一致收敛 |
| G-16c | channelSettingsRequested | channel-summary/dialog | group settings drawer、header、runtime view | displayName/notice/info 只读完整 VM；失败不乐观覆盖 |

## 5. 每行必需证据字段

每个 Gate 的机器结果必须至少包含：

```json
{
  "gateId": "G-XX",
  "candidate": {
    "helixRevision": "...",
    "clientRevision": "...",
    "clientHelixPin": "...",
    "matrixSemanticHash": "...",
    "fixtureManifestHash": "..."
  },
  "ingress": {
    "kind": "actionBinding|hostObservation|inbound",
    "actionRef": "...",
    "requestId": "...",
    "surface": "...",
    "scope": {}
  },
  "ticks": [],
  "effects": [],
  "persist": {
    "operationId": "...",
    "result": "ok|error",
    "writeSet": []
  },
  "frame": {
    "schemaVersion": 2,
    "transactionId": "...",
    "causationId": "...",
    "surfaces": [],
    "surfaceEpochs": [],
    "revisions": [],
    "directives": []
  },
  "angular": {
    "apply": "applied|rejected|resync-required",
    "component": "...",
    "domAssertion": "...",
    "frameApplied": true
  },
  "counts": {
    "v1Frames": 0,
    "rawBusinessFallbacks": 0
  }
}
```

Observation Gate 可以没有 `actionRef/causationId`，但必须有可替代的 connection/session/operation correlation。

## 6. 迁移与验收顺序

### R1 - 客户端 V2 reader 就绪

- V2 codec、runtime、surface adapters、action causation、directive 和 FrameApplied 正负测试通过。
- V1 reader 代码可以在迁移期间暂存，但不得进入全量 E2E candidate 的生产可达路径。

### W1 - Helix 全量 V2 writer

- 建立唯一 V2 builder/emitter。
- 按 25 个 Gate 迁移 producer。
- 每 Gate 先跑 replay/contract/persist-barrier，不按 Gate 构建 Native。

### PIN - 更新并冻结 pin

- Helix revision、Cargo pin、生成 Matrix revision/hash、fixture manifest hash 一致。
- 不允许使用 working HEAD 代替 Native 实际 pin。

### R5 - 切断 V1 运行链

- Helix 生产路径只允许 V2 builder/emitter；V1 writer 命中即 FAIL。
- Angular production codec 对 InteractionFrame V1 fail-closed，不执行 legacy normalize。
- `v1Frames/v2Frames/rawBusinessFallbacks` 必须由 collector 结构化计数。

### E1 - 全量 cutover E2E

- 一个 Native candidate、双用户、一次连续执行 25 Gate。
- `required fail=0`、`required skip=0`、`v1Frames=0`、`rawBusinessFallbacks=0`。
- 这是物理删除的准入，不是删除后最终完成。

### D2 - 删除兼容代码

- 删除 Helix V1 writer/builder、Angular V1 union/normalize、V1 fixtures和已确认旧 RenderNode consumer。
- 保留 ActionInvocation/Matrix 文档等独立版本合同，除非另有批准。

### E2 - 删除后最终回归

- 重新执行静态删除清单、跨仓 fixture parity、focused/full Rust 与 Angular tests。
- 重新构建最终 Native candidate并执行同一聚合回归。
- 只有 E2 仍为 25/25 PASS，Feature 才能完成。

## 7. Fail-Closed 判定

以下任一情况使对应 Gate 为 FAIL：

- 收到 `schemaVersion=1` 或未知 schema；
- raw `im:*` 改写业务 VM；
- action 组件绕过 projected binding 选择命令或业务 identity；
- durable Frame 早于 matching PersistOk；
- mutation 缺 surfaceEpoch、scope/revision 不匹配或 VM 不完整；
- 整帧部分 apply、directive 在 commit 前执行或 FrameApplied 伪报；
- Helix/client/contract/fixture pin 不一致；
- 旧证据没有当前 candidate 的 schema 计数却被复用为 PASS。

环境、连接器或只读数据证据不可用时结果是 BLOCKED，不得降级成 PASS。
