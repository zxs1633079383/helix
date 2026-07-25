# G13 Helix、PC、Mobile V2 验收合同

> 本文冻结目标验收合同，不把已有 focused test、旧 desktop artifact 或 command GTest
> 写成当前三端 Green。

## 1. 完成定义

`G13` canonical Green =

`Helix producer replay ∩ canonical Fixture/provenance ∩ PC WDIO Mock ∩ real invoke
∩ single/dual real ∩ console+Tauri/Helix logs ∩ FrameApplied+DOM
∩ Mobile GN GTest ∩ Venus real FFI/apply/ack/view ∩ unified report`。

## 2. Helix producer

- 从真实 Tick/PortReply/Inbound 生成完整 Frame，不手写 JSON 作为 producer。
- 断言 command、Effect 顺序、Persist barrier、authority correlation、Frame inventory、
  typed VM 与负向/幂等/恢复路径。
- Fixture inventory 至少覆盖：`channel-view-command`、`post-read-or-update-channel-authority`、`receipt-frame`、`channel-summary-frame`。
- Fixture 带 Helix commit、contract epoch、raw SHA-256、semantic SHA-256、surface inventory。
- 普通测试只比较 Golden；显式 UPDATE 开关才允许更新。

当前 focused 入口：`crates/helix-im/tests/g13_channel_read_interaction_test.rs`。

## 3. PC WDIO

| Lane | Required assertion | 最大 claim |
|---|---|---|
| contracts | schema/provenance/hash/inventory/negative decode | fixture-contract |
| mock | Accepted 与 Frame replay 分离；真实 bus/runtime/DOM | mock |
| integration | 真 Tauri invoke 进入 Helix；日志可相关 | integration |
| single-real | Actor Go/SQLite/FrameApplied/DOM | single-real-chain |
| dual-real | Actor/Observer 同 authority；各自 FrameApplied/DOM | dual-real-chain |
| report | stage、artifact、redaction、claim boundary | reporting-contract |

禁止在 `wdio.conf.ts` 写 G13 分支。Platform 提供 lane/candidate/hooks；通用模块提供
Mock、real invoke、multiremote、assertion、report、observability；G13 domain 只提供
Fixture、page workflow、业务 stage 与 typed assertion。

## 4. PC 断言

- Surface：`message.participant-receipts`、`message.channel-summary`。
- 接受值：`ParticipantReceiptsVm(kind=read)` 与 `ChannelSummaryVm`；read completion 必须绑定 viewer/channel authority。
- 不变量：未读清零和 readBits 不在 UI 先写；客户端不得提交 userId/unreadCount 冒充服务端权威。
- 每个 required transaction 都有 matching FrameApplied，无 FrameRejected。
- DOM 只在 apply 后标绿，不通过 fixed sleep、raw event 或 store mutation 取巧。
- Actor/Observer profile、auth、device、SQLite、WebDriver port 和 WS session 隔离。

## 5. Mobile GN GTest

目标文件：

`tests/gtest/surface_runtime/G13FrameContractTest.cpp`

必须覆盖：

1. 同源 Fixture byte/hash parity。
2. `schemaVersion=2` decode；V1/未知字段/重复 slot/坏 revision fail closed。
3. C++ `SurfaceRuntime::ingest` 原子 apply 与 transaction ack。
4. QuickJS facade 产生与 PC 同一 canonical command/payload。
5. `im.surface.view` 返回与 PC typed VM 等价的字段和值。
6. duplicate/out-of-order/patch base mismatch/late authority 幂等。
7. 不出现 raw business reducer 或 Mobile-owned counter/status。

现有 command 层入口：`tests/gtest/uc/Uc13ReadTest.cpp`；它不能替代上述 Frame GTest。

## 6. Venus 真实接码

```text
Venus ImRootApi
-> HelixImQuickJsBinding::dispatchCommand
-> HelixImDriverRuntime / helix-driver-ffi
-> CppHostVtable.on_domain_events_batch
-> enqueueDomainEvents / pumpDomainEvents
-> mobile_surface_runtime_ingest_event
-> SurfaceRuntime atomic apply
-> mobile_surface_runtime_ack_next
-> __helix_frame_applied
-> im.surface.view
```

Venus 必须使用同一 Helix command builder 与 Frame consumer，禁止针对 G13 在 JS/C++
增加业务状态机。

## 7. 日志与 Report

同一 `suiteRunId/scenarioRunId` 至少关联：

`gateId,operationId,requestId/reqId,causationId,transactionId,channelId,
messageId/postId,eventSeq,surface,scope,revision,surfaceEpoch,FrameApplied,DOM/view`。

Browser console 保存 action、IPC admission、FrameReceived/Applied/Rejected、DOM latch。
Tauri/Helix 保存 command、Tick、Effect、Persist/HTTP/Inbound、Effect::Emit。
Go/PG/WS 保存 track、commit/outbox/delivery/readback。日志不得保存 token、cookie、密码或
完整敏感 payload；redaction 失败即 run 失败。

报告 stage：

- `g13.channel-view-command`
- `g13.post-read-or-update-channel-authority`
- `g13.receipt-frame`
- `g13.channel-summary-frame`
- `g13.pc-mobile-parity`
- `g13.correlation`

每个 stage 输出 `status,startedAt,completedAt,durationMs,expected,actual,assertions,
evidence,blockedBy,claimBoundary`。

## 8. 当前 stop condition

Strict Matrix 声明 receipts + channel-summary，但 ChannelSummaryVm 无 unread 字段；必须在 producer Fixture 中证明真正承载值或修正 Matrix

只有 producer replay、Matrix、Fixture、PC 与 Mobile 断言对同一字段/Surface/终态达成一致，
才进入真实运行；真实前置缺失时保持 BLOCKED，不用 Mock 或历史 artifact 代替。
