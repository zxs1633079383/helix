# Verification Procedure: G-07

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-07` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`ccbc354ab2444736400ad45195556fecffce7b94`；前序 G06
  T001-T038 全部完成，mandatory R4 `PASS` 且最终 `CLOSED`。
- cses-client：`ab7833996be5156f4d0e0655c18e7f9c65488961`，tracked tree
  clean；Tauri 四个 Helix production dependency 统一 pin
  `3798ffdd0bfd89b8922829dd252327bb0a2075ec`。
- Mobile：`3bd7650a2545e4a8a2d56b552d2ad20ee1bbce8e`，tracked tree
  clean；任务前已有八份 Helix bundle 与 `tests/.DS_Store` 保持未跟踪。
  production Helix pin 为
  `6a2703971a2e452c191d6a3b6481a5a3c51998fc`，bundle SHA-256 为
  `deefc3ea88a6c01fd394ab618d4177843e56238dbeae71157ec302e20104a9ad`。
- Strict Matrix 机器真源
  `crates/helix-im/contracts/strict-matrix.v1.json` raw SHA-256 为
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`；
  人读生成文档 raw SHA-256 为
  `6f186cf291dc6336798b1178bf3d97493588eea8ccc467f026766654d3fe2354`。
- 当前只具备 urgent outbound、基础 ActionBinding 与平台 admission；没有 G07
  完整 ScenarioRunner、canonical Fixture、PC 独立 lanes 或 Mobile typed
  UC08 contract。因此仅满足 implementation admission，不声明任一 lane Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用 G01 production prerequisite：`im_create_channel` 取得权威
  `channelId`，随后在同一 fresh group 发送 source post；G07 只消费权威
  post/channel ID，不复制建群 handler。
- PC 继续使用 `g01-group-prerequisite.ts` 从真实 MessageV4 UI 创建群并选择
  Actor 444、Observer 678；Mock 只 replay Helix canonical Frame。
- Mobile 继续使用 `UcJsContractFixture::RC.createChannel` 与 `sendMessage`
  创建 fresh group/source post；不依赖固定旧 channel 或前序 Gate 遗留数据。
- 三端现有共享前置满足 G07，无需新增第二套 CreateGroup 业务逻辑。

## T004-T008 Helix Production Flow

- 新增 `G07ScenarioRunner`，仅以 production `ImModule::handle` 驱动 fresh
  group、urgent/urgentConfirm opaque action、HTTP、`post_update`、PersistAtomic、
  local readback 与 Surface resync。
- 发起者链输出 active timeline + pending urgent receipt；Observer 链先提交连续
  eventSeq authority，再输出 acknowledged timeline + completed receipt。HTTP
  failure、Persist failure/replay、duplicate WS、late HTTP 与 cold-start recovery
  均不依赖平台 reducer。
- 修复 production `message.participant-receipts`：`urgentConfirmation` 不再固定
  forbidden，而是 O(1) `StorageOp::Get(message.id)` 后从 durable
  `expedite_map` 生成 Helix-owned `requiredCount/completedCount` 与
  completed/pending rows；template confirmation 仍显式 forbidden，read receipt
  仍走既有 HTTP authority。
- `cargo test -p helix-im --test g07_full_flow_test --test
  g07_urgent_interaction_test -- --nocapture`：exit `0`；full-flow `8/8`，
  focused `5/5`，fail `0`、skip `0`。

## T009-T012 Canonical Fixture

- Fixture producer 固定为 Helix source commit
  `768dc9b3597523c6c40279e387d85e9f240fde8a`；只有
  `UPDATE_G07_WDIO_FIXTURES=1` 可覆盖 Golden。
- inventory 共 `7` 份完整 Frame：`group-created`、`member-roster`、
  `urgentable`、`urgent-active`、`receipts-pending`、`urgent-confirmed`、
  `receipts-completed`。timeline、member-roster 与 participant-receipts 保持独立
  transaction，不假设固定 Frame 序号。
- provenance raw SHA-256：
  `5057646b19aec3c88f315ab7564d40ea3cab296faed3acee586c5262136d557c`；
  每份 raw/semantic hash、schemaVersion=2、surface inventory 与允许动态字段
  已逐项记录。
- `cargo check -p helix-im`：exit `0`。
- `cargo test -p helix-im --test g07_wdio_fixture_contract_test --
  --nocapture`：exit `0`，`1/1`，fail `0`、skip `0`；普通模式逐字比较
  checked-in JSON 与当前 production trace。

## T013-T022 PC Mock/Real/Report

- cses-client 接受提交为 `60ad2d03173b093ccb4d8e42d34d2f6b97c31909`，
  production Tauri 的四个 Helix dependency 均 pin 到 fixture producer
  `768dc9b3597523c6c40279e387d85e9f240fde8a`。
- Fixture parity、typed loader、AcceptedCommand admission、完整 Frame replay、
  MessageV4 page object、reporter 与 production-boundary 已完成；聚焦 Jest
  `19/19`、urgent component `2/2`、WDIO TypeScript、Angular WDIO build 与
  Tauri debug build 均 exit `0`。
- Mock run `mock-20260725232835106-7075` 为 `2/2`，claim 仅为 `mock`。
- single-real run `single-real-20260725233241043-10413` 为 `1/1`；真实链关联
  UI、Tauri invoke、Helix、SQLite、HTTP、`post_update` WS、FrameApplied 与 DOM。
- dual-real run `dual-real-20260725233518185-12322` 为 `1/1`；Actor 发起加急、
  Observer 确认后，两端 timeline 终态相关联。report run
  `report-20260725233610911-12991` 为 `1/1`。
- production-boundary 扫描 `2735` 个生产文件，Fixture/Mock/WDIO API
  production reachable=`false`。

## T023-T032 Mobile Contract

- Mobile 接受提交为 `9ab37d2a1137ebbfa7d14ab0f01533e1b847a5ff`，production
  Helix source lock 为 `e26bf167df36b17d97909e70e5f0f58f2e07829b`，
  bundle SHA-256 为
  `0b5477e444efd40999dfc05d4456450d0605f98509f15d177e81915e8f5921e7`。
- G07 mirror parity 通过：`count=7`，fixture producer 为 `768dc9b...`；
  source-pin 通过且 G07 inventory=`7`；legacy hard-cut 通过，
  production V1 producer/consumer/raw fallback 均为 `0`。
- `G07FrameContractTest.*` 为 `5/5`；UC08 非 real 聚焦合同为 `8/8`；
  `./scripts/run-uc01-16-gtest.sh --only UC08` 为 `9/9`，fresh group 中 Actor
  发起加急、Observer 确认并由生产 QuickJS `im.surface.view` 观察
  active/acknowledged typed timeline。
- 共享 `./scripts/run-uc01-16-gtest.sh` 已真实执行，runId
  `mrc-mobile-uc-20260725235447-26068`；总结果 `139/150`。G07 的
  `Uc08UrgentTest.RealChain` 通过，11 个失败均来自 GS02/GS03/GS04、
  UC01/UC02/UC10-UC15 的其他 Gate real lane，因此不计入 G07 required lane，
  但保留为后续 Gate 的真实 blocker，不把共享结果宣称为全绿。
- 曾额外尝试在 Mobile real lane 观察 participant-receipts pending row；真实
  typed view 未暴露该 row。该能力不作为已通过证据，G07 required Mobile
  contract 仅由 canonical production consumer GTest 与真实 timeline 闭环证明。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 三份 provenance raw SHA-256 均为
  `5057646b19aec3c88f315ab7564d40ea3cab296faed3acee586c5262136d557c`。
- 三仓 inventory 均为同一 `7` 份 Frame，schemaVersion 全为 `2`，surface
  inventory 为 `message.left-dialog-list`、`message.member-roster`、
  `message.timeline` 与 `message.participant-receipts`。
- 七份 raw SHA-256 在三仓逐文件均为 `3/3` 相同；provenance 内七份
  semantic SHA-256 也因 provenance byte-exact 一致而相同。平台没有补业务字段。

## T034-T036 Review

- **Behavior Green**：fresh group 前置、发起者权限、target 去重、active 与
  acknowledged 终态、HTTP/Persist 失败、重复/迟到 authority、恢复以及
  Actor/Observer 结果均由 Helix 同一模型决定。
- **Code Boundary Green**：urgent/receipt 聚合只在 Helix production handler 与
  projector；PC 仅 Tauri admission + Frame apply + DOM，Mobile 仅 SDK admission、
  FFI DomainEvent batch、SurfaceRuntime atomic apply 与 QuickJS typed view。
  两端测试 Fixture/Mock/injection 在默认生产构建不可达。
- **Frame V2 Green**：七份完整 envelope 的 schema/scope/surfaceEpoch/revision/
  baseRevision/mutation/action/directive/correlation 已字段级对齐。active timeline
  与 confirmed timeline、pending receipts 与 completed receipts 是可竞态的独立
  correlation，不强制固定第三帧；重复 transaction 与 stale revision fail-closed。

## T037 Gate Report

- 结构化汇总见 `evidence/gate-report.json`。G07 required lanes 的 fail=`0`、
  skip=`0`，final verdict=`Green`；共享 Mobile 跨 Gate 红项与未声明的 receipt
  real observation 均保留在 observation，不被总 PASS 掩盖。

## T038 Quickstart And Mandatory Review

- Helix quickstart 于 `2026-07-25T17:09:00-07:00` 重跑：`cargo check`、
  full-flow `8/8`、focused interaction `5/5`、fixture contract `1/1`，
  全部 exit `0`；原始日志及 SHA-256 已登记到 `evidence/evidence.json`。
- PC G07 final artifacts：Mock `2/2`、single-real `1/1`、dual-real `1/1`、
  report `1/1`；production-boundary 再跑仍为 `2735` files、exit `0`。
- Mobile final parity、source pin、legacy hard-cut 与 `git diff --check` 均
  exit `0`；共享回归的跨 Gate `11` 个 real-lane failure 已如实登记。
- mandatory after_implement loop 于 event `13` 给出 R4 evidence `PASS`。
  targeted convergence 与 after_converge 只有在未追加任务时才允许最终
  `CLOSED`。
