# Verification Procedure: G-12

## T001-T003 Source Lock And Shared Prerequisite

- Source lock：Helix
  `533f1b654351f6708e34344aa509fcd58b81cf86`；cses-client
  `509d10cae61e66f6d0a90461daa8d1ad192ba1b8`；Mobile
  `49b94309710c511c1ffae1bf41f6a55f022fabe2`。
- Consumer pins：PC 四个 Helix Tauri dependency 当前均为
  `5e3ffc026fc8b25f76a7cce162e75734bddb45ca`；Mobile production source
  lock 为 `60b2d5f579f209a144433db1f96bef8a63f6bd5c`，bundle SHA-256 为
  `127e60902595cddb6324a10c0fe68cb94fb6cf3968e63a813914fa28785673e6`。
- Strict Matrix SHA-256 为
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
  前序 G11 ledger 为 `phase=closed`、R4=`pass`、`next_action=none`。
- 工作树边界：PC clean；Mobile 仅有既存未跟踪 bundle 与
  `tests/.DS_Store`，本 Gate 不覆盖；Helix 只有本次 Spec 指针、ledger、
  SESSION 与 G12 文档改动。
- 三端共享 fresh-group prerequisite 已存在：Helix
  `G01ScenarioRunner::create_group_prerequisite`、PC
  `prepareMockCreatedGroup`/真实建群 harness、Mobile
  `RC.createFreshGroupPrerequisite`。G12 每次 canonical/real run 必须调用该
  能力创建独立 channel/thread，不复制建群 reducer，也不复用 G11 数据。

## T004-T008 Helix Production Flow

- `cargo check -p helix-im` 最终 exit `0`。
- `g12_full_flow_test` 最终为 `9/9`，覆盖 fresh group、wire admission、
  snapshot、branch patch、空回复终态、HTTP failure、mixed authority、
  duplicate reply 与 detached recovery。
- `g12_reply_thread_interaction_test` 为 `3/3`，证明读-only G12 无伪造
  Persist barrier，terminal batch 只有完整 Frame，late/duplicate 与恢复均由
  Helix correlation/projector 决定。
- `read_relay_test` 为 `11/11`；原本期待 raw `im:read:result` /
  `im:channel:replies` 的 stale snapshot 回归已改为只接受
  `im:interaction:frame`，迟到旧 authority 为零输出。
- 生产链现为 `OutboundReplies -> normalize authority ->
  project_reply_thread_frame`：`im_get_replies` 首次产生 revision 1
  `threadDetailSnapshot`，attached `im_get_reply_branch` 产生
  baseRevision 1 / revision 2 的完整 `threadDetailPatch`。HTTP/error/mixed
  authority 不产生空成功 Frame；detached branch 保持 view-silent，后续
  snapshot 恢复。

## T009-T012 Helix Canonical Fixture

- `g12_wdio_fixture_contract_test` 从同一
  `G12ScenarioRunner -> ImModule::handle -> Effect::Emit` trace 导出三份完整
  Frame：`group-created.frame-v2.json`、`thread-snapshot.frame-v2.json` 与
  `thread-branch.frame-v2.json`。测试逐份执行 typed decode、schema 2 校验、
  thread 字段级语义断言以及 Golden byte 比较。
- 只有 `UPDATE_G12_WDIO_FIXTURES=1` 可创建或覆盖 Golden；普通测试只比较
  actual producer 与 checked-in bytes。显式更新与普通比较两次运行均为
  `1/1 PASS`、exit `0`。
- `provenance.json` 固定 producer commit
  `a2207d73f8e72da7374b916b2167ee3a25b9f6c3`、fixture id、schema、
  surface inventory、raw/semantic SHA-256、动态替换与 revision rebase 白名单。
  `jq` inventory/schema/hash 校验为 `true`。
- canonical raw SHA-256：group-created
  `5753a7e4e6bad20f2b46d345f6483b2600bd10f5cfccb16aa17200d54aaf0ab6`；
  thread-snapshot
  `434207276732e4076ddacb206731187e8031a79e25240ce3a3d66e300a2e7c37`；
  thread-branch
  `6a11b920460c031129c28b0bff18303fc4f3e56fa4653da28fe065646755c6e2`。
- Helix Green 命令：`cargo check -p helix-im`、`cargo test -p helix-im
  --test g12_full_flow_test -- --nocapture` 与 `cargo test -p helix-im --test
  g12_wdio_fixture_contract_test -- --nocapture` 均为 exit `0`。跨端 mirror
  在 T013-T033 完成 byte-exact 对账后关闭。

## T013-T022 PC Mock/Real/Report

- cses-client 接受提交为
  `6174658be34a779ded68311848cd5c4851ec9d01`；Tauri 四个 Helix production
  dependency 统一 pin
  `1b797413f2c1b14abde5eacf60eb942b21e9d650`。Fixture 同步器、typed
  loader、AcceptedCommand/replay 分离、G12 PageObject、reporter 与
  production boundary 均完成。
- production MessageV4 只补齐 thread-detail surface attach 与 drawer
  可观察入口；reply authority、树合并、revision 与终态仍由 Helix
  `InteractionFrameV2` 决定。focused Angular 为 `14/14`，Fixture/bridge/
  reporter 为 `38/38`，WDIO TypeScript、production candidate build 均为
  exit `0`。
- Mock runId `mock-20260726085609921-34662` 为 `3/3`、exit `0`，覆盖
  canonical branch、command admission failure 与 duplicate Frame，claim
  仅为 `mock`。
- single-real runId `single-real-20260726085414486-33678`、dual-real runId
  `dual-real-20260726085445281-33947` 与 report runId
  `report-20260726085630577-34835` 均为 exit `0`。真实链分别为 `1/1`、
  `1/1`、`1/1`，从 MessageV4 创建 fresh group，经真实 Tauri invoke、
  Helix HTTP、`im:__bus__` Frame ingress、SurfaceRuntime atomic apply 与
  DOM 自动等待闭合；dual-real 证明 Actor/Observer 最终都看到 sent reply。
- production boundary 扫描 `2735` 个生产文件，结果
  `mockProductionReachable=false`。embedded runner 的
  `tauri-driver not found` 仅为非致命诊断；实际 WebKit/Tauri session 已启动
  并完成 real suite，未把 Mock 或该诊断冒充真实链证据。

## T023-T032 Mobile Contract

- Mobile 分类提交为 `7f66684`（Helix production pin）、`767f7bf`
  （canonical mirror）与 `4281946`（GTest/QuickJS typed view）。
  production Helix pin 为
  `1b797413f2c1b14abde5eacf60eb942b21e9d650`，bundle SHA-256 为
  `6ef97f3d3d669034f51357e01141688278a4383ebaad4f578045a4ca57897fea`。
- `sync-g12-helix-fixtures.mjs --check` 与
  `check-g12-fixture-parity.mjs` 均为 `3/3 PASS`；producer
  `a2207d73...` 可从 production pin `1b797413...` 到达。source-pin、
  ABI、Command API 与 legacy hard-cut 均通过，G01-G12 production V1
  producer/consumer/raw fallback 为 `0`。
- `G12FrameContractTest.*` 为 `5/5`，覆盖完整 envelope、fresh-group
  prerequisite、thread snapshot/branch、schema/surface/revision
  fail-closed、SDK command admission、完整 DomainEvent batch、
  SurfaceRuntime atomic apply、QuickJS `im.surface.view` typed view 与
  production C ABI ack。
- `Uc12RepliesTest.*-Uc12RepliesTest.RealChain` 为 `8/8`。真实 UC runner
  已改为 fresh group 后经 `im.posts.getReplies/getReplyBranch` 只观察
  `message.thread-detail` typed view；本次环境未提供
  `MOBILE_UC_RUNTIME_CONTEXT_FILE`，因此 Mobile RealChain 明确不计入 required
  Mobile Contract lane，也不以 dry-run 冒充 Green。
- `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch ->
  SurfaceRuntime -> QuickJS typed view` 的生产接线已存在并由
  production-linked GTest 证明，未复制第二套 reply reducer。本 Gate 未修改
  Venus 接线能力，按合同执行共享 `run-venus-helix-im-smoke.sh`，结果 PASS。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 的 `provenance.json` SHA-256 均为
  `c1bc03de83f24e4375ee1006244596a35e305137e679d993452e52f54c922eb1`。
- 三仓 inventory 均为 `group-created`、`thread-snapshot`、
  `thread-branch`，schemaVersion 全为 `2`，surface inventory 分别为
  `message.left-dialog-list`、`message.thread-detail`、
  `message.thread-detail`。
- 三份 Frame raw SHA-256 在三仓逐文件相同：
  `5753a7e4...0ab6`、`43420727...c37`、`6a11b920...6e2`；semantic
  SHA-256 分别为 `bdec76a9...3e3a`、`fd903984...ff8b`、
  `3effc6ad...fcd3`。provenance byte-exact 一致，PC/Mobile 未补字段。

## T034-T036 Review

- **Behavior Green**：fresh group、root/first-level/nested reply、snapshot、
  branch patch、空结果、HTTP failure、mixed authority、duplicate/late reply
  与 detached recovery 共用 Helix reply-thread model。HTTP success 本身不是
  UI 终态；single/dual real 均等待 FrameApplied 与 typed DOM。
- **Code Boundary Green**：reply authority、排序、branch 合并、revision 与
  完整 Frame 只在 Helix `read_relay + projector/surface runtime`。PC/Mobile
  只做 production command admission、完整 Frame atomic apply 与平台渲染；
  Fixture、Mock、WDIO API、raw reducer 和测试注入默认 production 不可达。
- **Frame V2 Green**：schema `2`、transaction/causation、thread scope、
  surfaceEpoch、snapshot revision `1`、branch `baseRevision=1 ->
  revision=2`、root/nodes/orderedReplyIds/replyCount/page/actions 与完整
  `message.thread-detail` VM 已字段级对齐；duplicate、unknown schema/surface
  与 revision gap 均不部分 apply。

## T037-T038 Gate Report And Quickstart

- 结构化汇总见 `evidence/gate-report.json`。required lanes
  fail=`0`、skip=`0`、final verdict=`Green`；Mobile RealChain 缺少 runtime
  context 作为非 required observation 保留，未提升为合同或真实链证明。
- Helix、PC Mock/Real/Report、Mobile parity/GTest/typed view、production
  boundary、共享 Venus 回归与三仓 byte parity 均完成；三仓
  `git diff --check` 通过。
- mandatory after_implement 只有在 R4 接受本证据、tasks fingerprint 不再
  变化时才允许最终 `CLOSED`。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-12` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
