# Verification Procedure: G-05

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-05` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`8f3a14d81ffcb9d7d90224d2838fe3d623eeeb0e`；前序 G-04a
  T001-T038 全部完成，mandatory R4 `PASS` 且最终 `CLOSED`。
- cses-client：`f0cc3d187f4d84165a485d21abe91175264519f4`，tracked tree clean；
  Tauri 四个 Helix production dependency 当前统一 pin
  `c9c37ea8fcb398715cff872d4adbdc8a0e412c46`。
- Mobile：`883fca7f744feb6d38c48ff6b607fe7818c42bf3`；tracked tree clean，
  仅保留任务前已有 Helix bundle 与 `tests/.DS_Store` 未跟踪项。production
  Helix pin 为 `bde8d79fba6335f9d6d8ed3feafab05a80fcd151`。
- Strict Matrix 生成文档
  `crates/helix-im/docs/Strict Matrix/UC全链路发布矩阵-v1.md` raw SHA-256 为
  `6f186cf291dc6336798b1178bf3d97493588eea8ccc467f026766654d3fe2354`；
  机器真源 `crates/helix-im/contracts/strict-matrix.v1.json` raw SHA-256 为
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- 当前已有生产转发装配、HTTP per-target admission、可信 WS correlation 与
  dialog Persist barrier；尚无 G05 完整 ScenarioRunner、canonical Fixture、PC
  独立 lane 或 Mobile G05 Frame contract，因此只满足实现准入，不声明 Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用 G01 已验证的生产 fresh-group prerequisite：先经
  `im_create_channel` 取得权威 `channelId` 并持久化 channel/member，再在 source
  group 发送待转发消息；target group 也必须由同一前置能力新建。
- PC 复用 `g01-group-prerequisite.ts` 从真实 UI 创建 source/target fresh group；
  Mock 只回放 Helix canonical Frame，不在 Angular 补建群或转发 reducer。
- Mobile 复用 `UcJsContractFixture::RC.createChannel`，每次 real-chain 新建 source
  与 target group；GTest 的 command fixture 不把固定测试 ID 冒充真实前置完成。
- 三端无需新增第二套 CreateGroup 业务逻辑；本 Gate 只消费其权威 channel ID，
  因此 T003 无生产代码修改。

## T004-T012 Helix Producer Green

- production behavior commit：
  `3798ffdd0bfd89b8922829dd252327bb0a2075ec`；Fixture commit：
  `b61d39bee004a4e195a18fbb0c00fe56301dc6f2`。
- `G05ScenarioRunner` 只通过 public `client_api::build_command`、
  `ImModule::handle`、匹配 `PortReply`、可信 `Tick::Inbound` 与
  `Effect::Emit` 回放；没有测试专用 reducer 或直接修改 module state。
- RED 回放证实并修复两个生产问题：createPosts HTTP admission 曾发布无事实变化的
  causation Frame 并提前完成 ActionRegistry；attached target timeline 曾先消费
  authority ledger，导致随后 left-dialog Frame 丢失 causation。当前模型明确为
  timeline reconciliation 与 dialog patch 两条独立投影，dialog attached 时由其
  durable readback 完成 final target；headless 无 dialog 时 timeline 才是唯一终态。
- `cargo check -p helix-im`：退出码 0；`cargo tree -p helix-core` 对
  `tokio|sqlx|reqwest|web-sys|tauri` 搜索为空。
- `cargo test -p helix-im --test g05_full_flow_test --test
  g05_forward_interaction_test -- --nocapture`：退出码 0，6/6 full-flow 与
  3/3 focused contract 全过。覆盖 fresh target、source authority、业务拒绝、
  transport failure、WS-first/late HTTP、duplicate inbound 与 dialog PersistErr
  replay。
- 显式更新命令 `UPDATE_G05_WDIO_FIXTURES=1 cargo test -p helix-im --test
  g05_wdio_fixture_contract_test -- --nocapture`：退出码 0；普通模式重跑 1/1
  通过，证明未设置开关时只比较 checked-in Golden。
- canonical inventory 为 4：`group-created`、`source-forwardable`、
  `target-forwarded`、`target-dialog`，schema 全为 2；surface inventory 为两个
  `message.left-dialog-list` 与两个 `message.timeline`。
- raw SHA-256：group-created
  `4f3a10813d059d7eb58cb2918f2944ceefd1f2bbf8253107283c1c20e7861123`；
  source-forwardable
  `cc0195a098bd739a3b3ae8cae3d2b147f2f68dce066f451aa5c6b34d5ae3c5c1`；
  target-forwarded
  `1406445940595964e560f0128c68eadb6c3527bfd4b0b1478028376f6935334b`；
  target-dialog
  `bca69ee5f98e2f08564dcd6d632f4d00a90993eed104f2322d107b37097ed641`；
  provenance
  `cd92103657bd7cdc3ef848e957ec8f8d3e4581fe8a3a40392d6ce7d6c4b44ec7`。

结论：G-05 Helix producer 与 canonical Fixture required lane 为 Green；PC 与
Mobile 尚未开始，Gate 整体仍为 `implementing`，不得进入 G06。

## T013-T022 PC Mock/Real/Report Green

- cses-client commits：production overlay 修复 `e36d6c35f`；G05 三链验收
  `424d1c9e5`。Tauri production Helix dependencies 统一 pin
  `3798ffdd0bfd89b8922829dd252327bb0a2075ec`。
- Fixture sync check：`G05_FIXTURE_PARITY_OK count=4`；WDIO contract Jest
  4 suites、19 tests 全过。四份 Frame raw/semantic hash、schema、surface
  inventory 与 provenance 授权的 dynamic path 均 fail-closed。
- `yarn message:wdio:build`：Angular `wdio-e2e` bundle 与 Tauri debug
  candidate 构建退出码 0；candidate 为
  `src-tauri/target/debug/cses-client`，真实链接上述 Helix pin。
- Mock run `mock-20260725202840193-88968`：退出码 0，2/2；证明
  `AcceptedCommand` 不产生业务 ViewModel、dialog authority 与 target timeline
  为独立 correlation，transport rejection 不伪造 target Frame。
- single-real run `single-real-20260725203458024-94018`：退出码 0，1/1；
  从 MessageV4 UI 新建 source/target 群，闭合真实 `im_invoke_action`、
  `createPosts`、Go WS `post`、SQLite/PG、dialog Frame/FrameApplied 与 target
  DOM。证据位于该 run 的 `shared/evidence/single-real*.json`。
- dual-real run `dual-real-20260725203521691-94328`：退出码 0，1/1；
  Actor dialog 与 Observer target timeline 独立等待，最终 canonical post id
  在 Actor/Observer/SQLite/PG 一致。证据位于该 run 的
  `shared/evidence/dual-real*.json`。
- report run `report-20260725202909644-89360`：退出码 0，1/1；Mock、
  single-real、dual-real 报告均带 evidence level、case status、artifact hash
  与 redaction verdict。
- `CSES_WDIO_BOUNDARY_GATE=G05 node
  scripts/message-wdio/check-production-boundary.mjs`：
  `checkedFiles=2735`、`mockProductionReachable=false`、`ok=true`。
- 真实回归发现并修复两个非测试旁路问题：重复使用 opaque `actionRef` 时页面
  读取旧 terminal history；user-scope dialog Frame 曾被测试工具错误按 timeline
  `channelId` 过滤。当前生产页面读取最新 action request，证据 latch 按 surface
  scope 规则匹配。
- WDIO service 启动仍打印全局 `tauri-driver not found` 诊断，但 embedded
  WebKit driver 实际创建 session 且所有 required G05 lanes 退出码 0；该诊断
  不作为跳过或 Green 替代。

结论：G-05 PC Mock、PC Real、report 与 production boundary 为 Green；Mobile
尚未开始，Gate 整体保持 `implementing`，不得进入 G06。

## T023-T032 Mobile Contract Green

- Mobile commits：Helix 生产真源固定 `bc716bb`；Fixture 镜像合同
  `6b19ca7`；G05 typed view 与 UC06 真实链 `1b6b778`。Mobile production
  Helix pin 为 `0147ee8856aa2b072c7a7537d3c968d5bbbfdd50`，bundle SHA-256
  为 `f01ead551d1eefd059119beb5c7146322a6973425fae9aec20a6211e475f8bbd`。
- `node scripts/sync-g05-helix-fixtures.mjs --check` 与
  `node scripts/check-g05-fixture-parity.mjs`：退出码 0，
  `G05_MOBILE_FIXTURE_PARITY_OK count=4`；四份 Frame 与 provenance 均
  byte-exact，schema、surface inventory、raw/semantic hash 无 Mobile 补字段。
- `G05FrameContractTest.*`：5/5 通过。覆盖 envelope/runtime/mutation/scope、
  revision/causation、source forward action、target sent row、target dialog
  preview/activity、timeline/dialog 独立顺序、duplicate/schema/surface/revision
  fail-closed atomicity、生产 C ABI 与 QuickJS typed view。
- `Uc06RelayTest.*-Uc06RelayTest.RealChain`：8/8 通过；`--only UC06`：
  9/9 通过。真实 run `mrc-mobile-uc-20260725205931-8304` 从两个 fresh group
  开始，Actor `im_relay_messages` batch terminal 成功，Observer 经生产
  DomainEvent batch、SurfaceRuntime 与 `im.surface.view` 观察到唯一 forwarded
  sent row。
- 生产调用链复核为 `MobileImSdk::relayMessages -> MobileFfiBridge ->
  helix-driver-ffi DomainEvent batch -> SurfaceRuntime atomic apply -> QuickJS
  im.surface.view`；现有生产接线已满足 G05，因此未在 Mobile 增加第二套 reducer。
- `check-surface-runtime-legacy-hard-cut.mjs`：
  `production V1 producers=0 consumers=0`，G01/G02/G03a/G04/G04a/G05 均
  typed-only；source-pin 与 diff check 同时通过。
- 共享 `run-uc01-16-gtest.sh` 完整执行 150 项，退出码 1，137 pass/13 fail。
  G05 `Uc06RelayTest.RealChain` 通过；失败项为 GS02、GS03、GS04、UC01、
  UC02、UC07、UC08、UC10、UC11、UC12、UC13、UC14、UC15 的真实链缺口，
  分别归入后续/既有 Gate 债务，不反向伪装成 G05 required lane 失败，也不把
  全量回归声明为 Green。

结论：G-05 Mobile canonical contract、生产 SDK/Bridge、SurfaceRuntime 与
QuickJS typed view required lane 为 Green；共享回归债务已显式保留。

## T033-T037 Cross-repo Review

### Fixture Parity

- Helix、cses-client 与 Mobile 三目录 inventory 均为四份 Frame 加一份
  provenance，逐文件 SHA-256 完全一致。raw hash 依次为
  `4f3a1081...`、`cc0195a0...`、`14064459...`、`bca69ee5...`，
  provenance 为 `cd921036...`。
- semantic SHA-256 依次为 `2609222d...`、`de358371...`、
  `ce2e7067...`、`2d524d5c...`；三端 checker 使用同一 projection 规则，
  未发现平台字段补写。

### Behavior Review

- 每次 canonical/real 验收均新建 source/target group，不依赖前序 Gate 遗留
  channel；source row 必须具备 `canForward=true` 后才接受转发。
- Actor command 终态、Observer target timeline 与 target dialog 是独立
  correlation；不假设 dialog 为固定第三帧。目标 timeline 只出现一个 sent
  forwarded row，dialog preview/lastActivityAt 来自权威 WS/persist readback。
- Helix scenario 覆盖权限/业务拒绝、transport failure、duplicate inbound、
  WS-first/late HTTP、headless recovery 与 dialog PersistErr；迟到 HTTP success
  不重复 terminal write/Frame。

### Code Boundary Review

- 转发 authority、幂等、persist barrier 与 Frame composition 只存在于 Helix
  生产 `ImModule::handle` 链。PC 仅 invoke/action + apply Frame，Mobile 仅
  SDK admission、whole DomainEvent batch、atomic apply 与 typed view。
- PC production-boundary 扫描 2735 文件，
  `mockProductionReachable=false`；Mobile production V1 producer/consumer
  均为 0，未注册 `im.store`、test action API 或 raw reducer。
- Mock/GTest/Fixture 只验证生产边界，不为缺失生产代码补业务字段或状态机。

### Frame V2 Review

- 四份 Frame 均 `schemaVersion=2`、runtime `online/ready/idle`，directive
  为空；source timeline snapshot 为 revision 1、无 causation，target timeline
  patch 为 base 1 -> revision 2、causation=reqId。
- left-dialog-list 分别为 base 2 -> revision 3 与 base 3 -> revision 4；
  target dialog causation=reqId。scope、surfaceEpoch、transactionId、
  mutation causation 与 action epoch 均由 typed contract 校验。
- `group-created/source-forwardable/target-forwarded/target-dialog` 保持两条
  timeline/dialog correlation，不把 HTTP terminal 与 WS dialog 错排成单线状态机。

结论：行为、代码边界、Frame V2 与证据四项 Review 均为 Green；等待 mandatory
after_implement loop 的 R4 evidence 判定后才可关闭 T038。

## T038 Mandatory Loop Closure

- `evidence/evidence.json` 已登记 `V-007`，绑定
  `gate-report.json` SHA-256
  `2ef5c194a25da0189bcc822339ad9d7845d5b9546c7d05095bb2ac37e4c263f9`。
- mandatory after_implement：R4 evidence `PASS`；`speckit-converge` 检查
  FR 18、SC 6、用户故事 3、计划链 7，未发现 missing/partial/contradicts/
  unrequested gap，未追加任务。
- mandatory after_converge：`CLOSED`。G-05 现为 `review-green`，允许总控按
  顺序进入下一 Gate；Mobile 共享回归中的 13 个非 G05 failure 仍保留为各自
  Gate 的真实待办，不随本次关闭消失。
