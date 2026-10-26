# G08 FrameV2 三端 Handoff

> 更新时间：2026-07-24 PDT。本文区分当前事实、历史证据、目标与阻塞。

## CURRENT

- Helix V2 schema 与 typed Surface 已存在。
- Helix focused test：`crates/helix-im/tests/g08_schedule_create_interaction_test.rs`。
- cses-client typed component test：`src/pages/message-v4/components/__tests__/message-ui-schedule-dialog.component.test.ts`。
- cses-client 仍有旧 desktop scenario/artifact，但新 WDIO Platform 下的 G08
  fixture/mock/real specs 尚未因此自动存在。
- Mobile 已有 command/UC GTest `tests/gtest/uc/Uc09ScheduleTest.cpp`、real-chain manifest 与通用
  SurfaceRuntime/Venus V2 consumer。

## HISTORICAL

2026-07-22 的 Native Gate artifacts 可用于识别旧场景和失败签名；它们不证明当前 Helix
pin、新 WDIO candidate、Mobile Frame consumer 或当前 Go 进程仍为 Green。

## TARGET

1. Helix 新建 G08 scenario replay 与 canonical V2 Fixture。
2. Fixture provenance/hash 同步到 PC 与 Mobile。
3. PC domain 接入 contracts/mock/integration/single-real/dual-real/report。
4. Mobile 新建 `tests/gtest/surface_runtime/G08FrameContractTest.cpp`。
5. Venus 用真实 helix-driver-ffi 完成 command -> frame -> apply -> ack -> view。
6. 两端对 声明 Surface 为 `message.channel-summary`；owner-only schedule hydrate 至少包含 channelId/scheduleId/scheduleOwnerUserId/scheduleMessage 做字段级等价断言。

## BLOCKED / CONTRACT_DRIFT

`ChannelSummaryVm` 当前没有显式 schedule 字段，focused test 也只验证 owner hydrate；这是 Fixture/WDIO 前必须解决的 CONTRACT_DRIFT

因此当前最大 claim：

`source-focused + command-contract + generic-v2-consumer-present`。

不得声明：

`current G08 PC-Mobile canonical Green`。

## FORBIDDEN

- PC/Mobile 各自实现状态、排序、计数、权限、分页或恢复业务逻辑。
- 把 AcceptedCommand、ActionReceipt、HTTP 200、raw WS 或 Fixture replay 当业务终态。
- 固定 sleep、固定 Frame 序号，或从旧 artifact 复制动态 ID。
- Fixture 没有 Helix producer provenance/hash。
- 通用 WDIO/Mobile runtime 反向依赖 G08 业务字段。

## NEXT OWNERS

| 工作 | Owner | Handoff 输出 |
|---|---|---|
| Frame producer/Fixture | Helix gate frame producer | actual Frame、Golden、provenance |
| WDIO lane/platform | WDIO Platform | domain-neutral lane/candidate/hooks |
| G08 PC workflow | PC Gate domain | page workflow、typed assertions、stage report |
| Mobile GN GTest | Mobile Gate contract | fixture parity、FFI/apply/ack/view |
| Venus integration | Mobile V2 runtime | real driver batch ingress 与 raw fallback=0 |
| 最终裁决 | Cross-platform reviewer | evidence plane 交集与最大 claim |

## STOP CONDITION

- 若 producer/Matrix/typed VM 不一致：标记 `CONTRACT_DRIFT`，停止生成 Fixture。
- 若 Fixture provenance/hash 不完整：停止 PC/Mobile Mock。
- 若当前 Go/凭据/PG/双账号不可用：real lane 标记 `BLOCKED`，不降级。
- 只有 Helix、PC、Mobile 三方字段与终态一致且报告无越级，才允许 canonical Green。
