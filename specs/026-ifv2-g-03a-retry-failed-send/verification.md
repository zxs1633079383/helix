# Verification Procedure: G-03a

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-03a` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`a0e897c2164744dd5d88f666c904b8c8d13de63b`；进入 G03a 前
  G02 子 Feature 已 R4 `PASS` 并最终 `CLOSED`。
- cses-client：`39695bbd00d978d619505a79560de8af2e89631e`，clean；当前
  Tauri Cargo pin 为 G02 producer
  `ac15abd0b29ef62ea5d359fb22030800dd0dee2b`。
- Mobile：`6730b5be8d11e334ef632a986173598f18ec9e9a`；仅保留进入任务前已有
  bundle 与 `tests/.DS_Store` 未跟踪项。生产 source pin 为
  `1774d5a38e694b4a56e2b85ea9055c3e825f2222`。
- Strict Matrix raw SHA-256：
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- G03a 当前没有 Helix canonical Fixture、PC WDIO lane 或 Mobile V2 Frame contract；
  本状态只满足实施准入，不声明本 Gate Green。

## T003 Shared CreateGroupPrerequisite

- Helix 继续复用公开 `G01ScenarioRunner::create_group_prerequisite`，通过真实
  `Tick::Command -> HTTP PortReply -> Persist/Emit` 取得权威 channel。
- PC 继续复用 domain-neutral `prepareMockCreatedGroup` 与真实 MessageV4 建群操作；
  G03a helper 不复制 G01/G02 marker 或业务成功条件。
- Mobile 继续复用 `createFreshGroupPrerequisite`，调用生产
  `im.channels.create` 并等待权威 `im:channel:created`；不复用 G02 遗留群。
- 三端能力已在 G02 闭环中重新验证，本 Gate 只复用合同，无需修改共享生产代码。

## T004-T012 Helix Producer Green - 2026-07-25

- 生产 revision：`bb4c6b4054566fe1bd5efa55de928dd96114927c`。
- `im_retry_send` 复用原 `temporaryId` 与 cached/durable body；retry 的
  `sending` 写改为有 correlation 的 `Persist`，只有 `PersistOk` 后才启动
  `posts/create` 与 15 秒 timer。
- `message.timeline` 的 failed delivery 与 retry ActionBinding 共用
  `actionCapabilities.canRetry`；Frame 内 `delivery.retryable=true` 且 `retry`
  binding 可执行，不由 PC/Mobile 补业务字段。
- `G03aScenarioRunner` 通过公开 `ImModule::handle` 覆盖 fresh group、首次发送失败、
  retry happy、业务失败、基础设施失败、WS 先行/迟到 HTTP/重复 WS 和 cold-start
  durable recovery。timeline 与 dialog 使用独立 correlation。
- canonical inventory 为 8 份 Frame，加一份 `provenance.json`：
  `group-created`、`group-timeline-empty`、`initial-sending`、`failed`、
  `retry-sending`、`retry-sent`、`ws-timeline-sent`、`ws-left-dialog-list`。
  普通测试只比较 checked-in Golden；
  唯一更新开关为 `UPDATE_G03A_WDIO_FIXTURES=1`。

### Commands

| Evidence | Command | Result |
|---|---|---|
| V-001 | `cargo check -p helix-im` | PASS，exit 0 |
| V-001 | `cargo tree -p helix-core` | PASS；无 tokio/sqlx/reqwest/web-sys/tauri |
| V-002 | `cargo test -p helix-im --test g03a_full_flow_test -- --nocapture` | PASS，6/6 |
| V-002 focused | `cargo test -p helix-im --test send_test retry_send -- --nocapture` | PASS，4/4 |
| V-003 | `UPDATE_G03A_WDIO_FIXTURES=1 cargo test -p helix-im --test g03a_wdio_fixture_contract_test -- --nocapture` | PASS，1/1；显式生成并立即 Golden 比较 |

### Stop Condition

- Helix producer、typed Frame 与 canonical Fixture 为 Green。
- PC 与 Mobile 的实现与证据见下文；本段仅保留 Helix 阶段结论。

## T013-T022 PC Mock / Real / Report Green

- PC commits：`50837f256c8a84dc948c0cd009f4af79e3807cb2`、
  `305b38163cb86468dd55ec274419d64e113d017e`、
  `08c1843b7b7751029b6fb9b40ec54eb122f646d0`。
- PC mirror byte-exact 同步 8 份 Frame；loader、fixture parity、TauriMockBridge
  admission/replay 与 reporter contract 均由 checked-in Helix provenance 驱动。
- Mock 中 `im_invoke_action` 只返回同步 AcceptedCommand；failed、retry-sending、
  sent 与 WS dialog/timeline 只通过完整 `InteractionFrameV2` replay 进入真实
  Angular SurfaceRuntime。
- `yarn message:wdio:g03a:mock`：2/2 PASS；evidence level=`mock`。
- `yarn message:wdio:g03a:single-real`：1/1 PASS。真实 UI fresh group 后覆盖
  Tauri invoke、Helix、SQLite、HTTP、WS、FrameApplied 与 Actor DOM。
- `yarn message:wdio:g03a:dual-real`：1/1 PASS。Actor/Observer 在同一 fresh group
  内完成 WS 恢复、retry 与最终 timeline/dialog 收敛。
- `yarn message:wdio:g03a:report`：1/1 PASS；JSON/Markdown 同源，
  `redactionVerdict=pass`。
- `yarn message:wdio:g03a:boundary`：扫描 2735 个 production files PASS，
  Fixture、Mock bridge 与 WDIO-only API 不进入 production bundle。
- PC candidate SHA-256：
  `6c9327d30752579fc988e8e94c77f619f9813e21c405d7dae923db6b4e8947da`。

## T023-T032 Mobile Contract Green

- Mobile commits：`6e7f122162a335a3be7a6b60e2b6a8ad48ebe1f6`、
  `4ff6a9b523f0102d68bf48f339e3e04b386bf81c`、
  `f6e4ce209ddd1c168fc1bd6a0cea62bac793c4dc`、
  `244325821aa495937aba51e9196e0cf59a98e24d`、
  `7bc78ef0b2510232a182b9999413bfe1a1bebe35`。
- `node scripts/check-g03a-fixture-parity.mjs`：8 份 Frame 的
  inventory/schema/raw/semantic hash 与 Helix producer
  `441c01209fa0995bdbc10081601acf6972827fbf` 一致。
- `G03AFrameContractTest.*`：5/5 PASS；完整覆盖 failed、retry-sending、
  HTTP/WS 竞态、late terminal、原子拒绝、生产 SDK batch、QuickJS typed view 与 C ABI。
- `Uc04RetrySendTest.*-Uc04RetrySendTest.RealChain`：7/7 PASS；首次错误地在
  `surface-runtime` binary 使用 UC filter 得到 0 tests，未计入 Green，随后使用
  `unit` binary 正确重跑。
- SurfaceRuntime 全量：59 PASS、1 个显式 source-parity 环境 SKIP；独立
  `check-surface-runtime-source-pin.mjs` 已 PASS，故该环境 SKIP 不替代 source proof。
- `check-surface-runtime-legacy-hard-cut.mjs`：production V1 producers=0、
  consumers=0，G01/G02/G03a typed-only。
- GS01 typed real-chain：7/7 PASS。生产修复允许 Helix 重启后的 fresh snapshot
  重基准缓存 revision，同时只接受 baseRevision 精确连续的首个 patch；未建立端侧 reducer。
- 共享 `run-uc01-16-gtest.sh`：149 条中 133 PASS；16 个尚未迁移 Gate 的
  legacy RealChain 等待 raw event/query 而失败。与已 CLOSED 的 G02 口径一致，
  该 lane 使用 legacy characterization，不是本 Gate required V2 Mobile Contract；
  不为它恢复 `im.store`、raw reducer 或 V1 consumer。

## T033 Fixture Parity Review

- Helix、PC 与 Mobile 的 8 份 Frame 加 `provenance.json` 共 9 个文件逐字
  `cmp` 一致；schemaVersion 全为 2。
- Surface inventory：timeline 7 份，left-dialog-list 2 份；HTTP terminal 与
  WS timeline/dialog 保持独立 correlation，不假设 dialog 是固定第三帧。
- canonical raw SHA-256：
  `group-created=fc92288636c4ab48051c8b0ffe821620f04622d476c71890ce9b2222bad75c73`；
  `group-timeline-empty=2bbc318e07d7e1d47a3de10a1feb8cf0bab3d3432478d7d6379013024ef3c623`；
  `initial-sending=7d398c795983cfd3af6299b61bfdfa5e043b4b5aaadd12bb7961acd08419ca6d`；
  `failed=0002af76ffce2b25903e5a0b5b48f2afe7b99e724100fa11744e63274d24cfcf`；
  `retry-sending=d651792c94a4a108b2659813109f71a41a9227527128bc7f1afcfe61fa345495`；
  `retry-sent=d21e894630ef054d21910d09f169eddec255629c3ab072011bdd8d89f9a6ac89`；
  `ws-timeline=8ab229ca1ac61b11e8b9bbfaf9654ead3f2db6f0ff17bef83c2d333e54dd1ee2`；
  `ws-dialog=7d5abcc3f326652e23989f281874c2c5b99ad38dde6294711d08a1ce78b3e3c9`。

## T034-T036 Consistency Review

- **Behavior Green**：每次 canonical/real 验收创建新的双用户群。retry 复用原
  temporary identity；failed 才开放 retry ActionBinding；PersistOk 前不 emit
  sending/HTTP；WS-first、late HTTP、duplicate inbound 与 cold recovery 不重复
  terminal write/Frame。PC single/dual real 和 Mobile typed contract 对 Actor/Observer
  最终结果一致。
- **Code Boundary Green**：业务权限、delivery、retry action、terminal authority、
  timeline/dialog VM 只由 Helix 生产。PC production-boundary 扫描通过；
  Mobile GTest 链接生产 SDK/Bridge/SurfaceRuntime/QuickJS/C ABI，V1 producer、
  V1 consumer 与 raw reducer 均为 0。
- **Frame V2 Green**：8 份 Frame 均核对 schema/runtime/transactionId、
  causationId、surface/scope、surfaceEpoch、revision/baseRevision、mutation、
  ActionBinding、directive、temporaryId/postId/eventSeq。PC FrameApplied 与 Mobile
  ack 均使用 producer correlation。
- **Evidence Green**：required Helix、Fixture、PC Mock、PC Real、Mobile Contract
  与 production-boundary 全部 pass=100%、fail=0、skip=0；future Gate legacy
  observations 单独列示，不冒充 required lane。

## Final Source Pins

- Helix：`af3fee1e2e9735d9cd9574a525e05b9d36333d58`。
- Fixture producer：`441c01209fa0995bdbc10081601acf6972827fbf`。
- cses-client：`08c1843b7b7751029b6fb9b40ec54eb122f646d0`。
- Mobile：`7bc78ef0b2510232a182b9999413bfe1a1bebe35`。
- Mobile Strict Matrix SHA-256：
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- PC candidate SHA-256：
  `6c9327d30752579fc988e8e94c77f619f9813e21c405d7dae923db6b4e8947da`。
