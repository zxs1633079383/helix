# Verification Procedure: G-04

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-04` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`bc03fb0d13c7062ada71467fc6ae14ea861d31ef`；前序 G03a 子
  Feature 已完成 T001-T038，R1-R4 `PASS` 且最终 `CLOSED`。
- cses-client：`08c1843b7b7751029b6fb9b40ec54eb122f646d0`，clean；当前 Tauri
  Cargo pin 为 `41ef427500757ecd627b337a156cb45818dcfc84`。
- Mobile：`7bc78ef0b2510232a182b9999413bfe1a1bebe35`；仅保留进入任务前已有
  bundle 与 `tests/.DS_Store` 未跟踪项。生产 source pin 为
  `af3fee1e2e9735d9cd9574a525e05b9d36333d58`。
- Strict Matrix raw SHA-256：
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- G04 当前已有生产 `im_revoke`、WS `posts_update` PersistAtomic 和 timeline
  Recalled 投影，但没有本轮要求的完整 ScenarioRunner、canonical Fixture、PC
  独立 WDIO lane 或 Mobile Frame V2 contract；只满足实现准入，不声明 Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用公开 `G01ScenarioRunner::create_group_prerequisite`，先从生产
  `Tick::Command -> HTTP PortReply -> Persist/Emit` 取得权威 channel，再发送一条
  可撤回消息作为 G04 目标。
- PC 复用 domain-neutral `prepareMockCreatedGroup` 与真实 MessageV4 建群/发消息
  操作；G04 helper 不复制前序 Gate marker 或成功条件。
- Mobile 复用 `createFreshGroupPrerequisite`，调用生产 `im.channels.create` 并等待
  权威 `im:channel:created`；不复用 G03a 遗留群或失败消息。
- 三端 prerequisite 已由 G01-G03a 闭环持续验证，本 Gate 只复用合同，不修改共享
  生产业务逻辑。

## T004-T008 Helix Scenario Green

- `G04ScenarioRunner` 复用共享 fresh-group producer，并通过生产
  `im_query_messages_by_channel` 取得 Helix 签发的 opaque revoke binding。
- action ingress 使用严格 `ActionInvocationEnvelope`；renderer 不提交 channelId、
  postId 或业务 command，`module/action.rs` 从已授权 binding 映射到 `im_revoke`。
- HTTP accepted/success/error 均不直接产生 recalled Frame。权威 `posts_update`
  先形成 message revoke + cursor 的 `PersistAtomic`，只有 `PersistOk` 后才 readback
  durable row 并生成 `content=Recalled`、`delivery=Recalled`、actions 为空的完整 Frame。
- 场景覆盖 fresh group、happy path、HTTP infrastructure failure、PersistAtomic
  failure/replay、duplicate WS、late HTTP 和 cold-start durable recovery。
- `cargo test -p helix-im --test g04_full_flow_test -- --nocapture`：6/6 PASS。
- 本阶段未发现生产 reducer 缺口，故 `module/action.rs`、`surface_composer.rs` 与
  projector 无需行为修改；新增内容只扩展共享 ScenarioRunner 与字段级合同。

## T009-T012 Helix Canonical Fixture Green

- 唯一 producer 为 `G04ScenarioRunner -> ImModule::handle -> Effect::Emit ->
  InteractionFrameV2::decode`，producer revision 固定为
  `f323cd7a997fba0cb330299d98eb0b8bd73d4813`。
- canonical inventory 共 4 份：`group-created`、`revocable`、`recalled` 与
  `recovered-recalled`；建群 Frame 为 `message.left-dialog-list`，其余为
  `message.timeline`，全部 `schemaVersion=2`。
- `provenance.json` 为每份 Fixture 记录 raw/semantic SHA-256、surface inventory、
  producer、source revision 与允许参数化/rebase 字段；`shasum -a 256` 与记录的
  `fixtureSha256` 逐份一致。
- 显式更新命令
  `UPDATE_G04_WDIO_FIXTURES=1 cargo test -p helix-im --test
  g04_wdio_fixture_contract_test -- --nocapture`：1/1 PASS。
- `cargo check -p helix-im`：退出码 0。
- `cargo test -p helix-im --test g04_full_flow_test --test
  g04_revoke_interaction_test --test g04_wdio_fixture_contract_test --
  --nocapture`：6/6 + 4/4 + 1/1 PASS。
- 普通 Fixture contract 测试只重新生成 actual Frame 并执行 byte-exact Golden
  比较，不设置 `UPDATE_G04_WDIO_FIXTURES` 时不会写盘。

## T013-T022 PC Green

- PC mirror 只由 `sync-helix-fixtures.mjs --gate=g04` 从 Helix canonical 目录同步；
  `--check` 返回 `G04_FIXTURE_PARITY_OK count=4`，未补写业务字段。
- fresh runtime rebase 首轮暴露 `ActionBinding.surfaceEpoch` 未随 mutation epoch 更新。
  修复回到 Helix provenance 真源 `43008acfc464a17711613fe5f37a56bb1146d22e`，
  明确授权 timeline/dialog action epoch 路径；PC loader 只按白名单机械替换。
- focused contracts：Fixture parity、TauriMockBridge 与 G04 reporter 共 3 suites、
  14/14 PASS；`yarn tsc -p tests/e2e/wdio/tsconfig.json --noEmit` 退出码 0。
- `yarn message:wdio:g04:mock`：2/2 PASS。命令 admission 只返回
  `AcceptedCommand`，recalled/recovery DOM 均来自完整 Helix Frame replay。
- Tauri 四个生产 Helix 依赖统一回钉
  `43008acfc464a17711613fe5f37a56bb1146d22e`；`yarn message:wdio:build`
  完成 Angular WDIO candidate 与 Rust debug binary，candidate SHA-256 为
  `a0bcaf714d263fc09fc853f75c1ddcb15b733d1deabb8029951e8847f0018fe2`。
- `yarn message:wdio:g04:single-real`：1/1 PASS。证据目录
  `artifacts/message-wdio/single-real-20260725171610025-68431/`，覆盖 fresh
  group、真实 UI、`im_invoke_action`、SQLite、`/api/cses/posts/revoke`、
  `posts_update`、FrameApplied 与 recalled DOM。
- `yarn message:wdio:g04:dual-real`：1/1 PASS。证据目录
  `artifacts/message-wdio/dual-real-20260725171634953-68740/`，Actor 444 与
  Observer 678 在隔离 profile 中收敛到同一 recalled messageId；本 Gate
  canonical surface 仅为 `message.timeline`，dialog mutation 明确 N/A。
- `yarn message:wdio:g04:report`：1/1 PASS；single/dual report 均
  `exitCode=0`、`redactionVerdict=pass`，并保存 JSON、Markdown、JUnit、
  Go/SQLite/PG/Frame/DOM evidence hash。
- `yarn message:wdio:g04:boundary`：
  `checkedFiles=2735`、`mockProductionReachable=false`、`ok=true`。
- 日志中的 `tauri-driver not found` 为 service 探测诊断；实际 lane 使用
  embedded provider 并均以退出码 0 完成，不作为跳过或替代证据。
