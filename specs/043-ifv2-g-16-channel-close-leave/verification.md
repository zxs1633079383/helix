# Verification Procedure: G-16

## Source Lock

- Helix: `509c2e2896d6b7d50528a1fceb9392efe050d0bc`
- cses-client: `88bdcd22169c151a321343cea73209952ec16dab`
- Mobile QuickJS: `c5896e16b3ce3a050b5d15b925c48415c39501b0`
- Strict Matrix SHA-256: `0e45a64ee1629498f44788613fb142489bfffbf7d02d950e8a51ae06afb419cd`
- 前序 G-15: `phase=closed`、`event_seq=26`、`next_action=none`

工作树边界：Helix 仅含本 Feature 的 SpecKit 状态；PC clean；Mobile 保留既有未跟踪 bundle 与 `tests/.DS_Store`，本 Gate 不删除或提交这些资产。

## T003-T012 Helix Producer And Canonical Fixture

- Fresh-group prerequisite、close/leave 独立 authority、business/transport/persist
  failure、duplicate/late reply 与 cold recovery 均由
  `G16ScenarioRunner -> ImModule::handle` 回放。
- Helix acceptance 为 `0e15c912fb062570445ba634ab7c83ff5b26d501`，
  lifecycle producer 为 `63b7eee5d356e1af08e32cc1fb92c2a466bc7841`。
- `cargo check -p helix-im` PASS；`g16_full_flow_test` 7/7、
  `g16_governance_interaction_test` 2/2、
  `g16_wdio_fixture_contract_test` 1/1 PASS。
- `helix-core` runtime dependency scan 为 0。五份 canonical Frame 覆盖
  active、closed、left、closed recovery、left recovery，普通测试未改写
  Golden。

## T013-T022 PC Mock, Real And Report

- cses-client 最终提交：
  `aac3c1583519b9308acb7c8dd0e415fa1dfc3813`；生产 Helix pin 为
  `63b7eee5d356e1af08e32cc1fb92c2a466bc7841`。
- Fixture parity 5/5、Jest 31/31、production-boundary PASS；Mock 2/2、
  report contract 1/1、single-real 1/1。
- dual-real 首次运行在 Observer `message.channel-summary` runtime 就绪前创建
  fresh group，两个 case 均在 Observer dialog latch 超时。修复为 Actor 与
  Observer 均完成 SurfaceRuntime ready 后再建群，候选
  `dual-real-20260726231338207-28751` 为 2/2 PASS。
- 另一次与 Mobile UC16 real-chain 共用测试身份并重叠执行的 dual-real 失败未
  选为隔离候选；原始失败未删除或提升为 Green。最终 required PC run IDs 为
  `mock-20260726230832037-22211`、
  `single-real-20260726230912246-22862` 与
  `dual-real-20260726231338207-28751`。

## T023-T032 Mobile Contract And Real Observation

- Mobile 最终提交：
  `0b65b4d1fbe92a2e825dcde4ed9bf3a65a88af72`（Fixture/source pin）、
  `937fa47d5ee06a70341e107097b39f644ef52573`（production-linked
  consumer）与 `46455d4d4e88870b75e72fdf1caa9f518eb87835`
  （生成合同 pin）。
- G16 mirror parity 5/5、source-pin、legacy hard-cut 与 UC01-16 contract
  gate PASS；生产 V1 producer=0、consumer=0、raw reducer=0。
- `G16FrameContractTest.*` 6/6、排除需 runtime context 的 focused contract
  8/8 PASS。随后独立 `run-uc01-16-gtest.sh --only UC16` 使用 fresh context
  完成 9/9，包括 owner close=`closed` 与 member leave=`left` typed summary。
- 共享 `run-venus-helix-im-smoke.sh` PASS。SDK command admission、
  MobileFfiBridge DomainEvent batch、SurfaceRuntime atomic apply、QuickJS
  typed view 与 production C ABI 均由生产链接测试覆盖。

## T033 Three-repository Fixture Parity

- Helix、PC、Mobile 的 `provenance.json` SHA-256 均为
  `c83b8041f35414b9b47719a78e68284576254a608a43ea551393e0d43ae9f4d2`。
- 五份 Frame 的 schemaVersion 均为 2，surface inventory 均为
  `message.channel-summary`；逐文件 raw SHA-256、semantic SHA-256 与
  provenance byte-exact 一致，PC/Mobile 未补 lifecycle 或 action 字段。

## T034-T036 Cross-repository Review

- **Behavior Green**：owner close 等待 `channel_close` WS authority，member
  leave 使用本人 SUCCESS authority；两者均在 PersistOk 后发布 terminal
  Frame。transport accepted、business failure、persist failure、duplicate、
  late reply 与 recovery 不产生伪成功。
- **Code Boundary Green**：lifecycle、action availability、revision 与完整
  Frame 只由 Helix 生产。PC 只做 command admission/Frame apply/DOM，Mobile
  只做 SDK/FFI batch/SurfaceRuntime/typed view；Fixture、Mock 与测试入口默认
  production-unreachable。
- **Frame V2 Green**：完整 envelope、channel scope、surfaceEpoch、snapshot
  revision 1、terminal patch `baseRevision=1 -> revision=2`、recovery snapshot、
  lifecycle/action 与 transaction correlation 均一致；invalid schema/surface/
  revision fail-closed 且不部分 apply。

## T037-T038 Gate Report

- 结构化汇总见 `evidence/gate-report.json`。required lanes pass=6、fail=0、
  skip=0，最终 verdict=`Green`。
- 所有原始命令输出均保留在 `evidence/`；Mock、命令受理与单端 replay 未替代
  PC Real、Mobile production contract 或三仓 Review。
- mandatory after_implement 仅在 evidence hash 已登记且 R4 接受本任务
  fingerprint 后允许最终 `CLOSED`。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-16` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
