# Verification Procedure: G-02

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-02` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Source Lock - 2026-07-25

- Helix：`cfc0895969efcc01b8b25d6e66ef7392042c4c89`。
- cses-client：`13e42ef2793ea5c9f39b29f5d1ce86b1aca074db`，clean。
- Mobile：`674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21`；保留运行前已有
  Mobile 文档、bundle 与 `.DS_Store` 脏项。
- cses-client Helix pin：`ee8440e7b6e19d75153f46349e57fbb6c00ca8cf`。
- Mobile source pin：`8ac3d2cf693d6012101444bfae5f2e89f707ebe9`。
- Strict Matrix raw SHA-256：
  `fcdf3e990acb507e69556ede2b1b9122b4ca2499c9348cfc0a1145eea6e830b4`。
- 前序 G01：总控 T002 已在当前三仓 HEAD 重新验证 Helix、PC Mock/Real/report、
  Mobile GTest/QuickJS 全绿，见
  `specs/020-ifv2-three-repo-uc-control/verification.md`。

## CURRENT / TARGET / GAP Revalidation

- Helix CURRENT 已支持 DOCUMENT command、opaque `props.document` 和 typed
  document timeline projection；缺 G02 独立完整流程与 Fixture。
- PC CURRENT 已支持文档 composer、`sendDocument` 和 typed renderer；缺 G02
  Fixture parity、WDIO Mock/Real/report。
- Mobile CURRENT 只有 generic UC02 command/real-chain readback；缺 G02 Frame
  Golden、SurfaceRuntime/QuickJS typed view 字段合同。
- G02 不产生 upload effect。Strict Matrix 已改为
  `storage_upsert -> http_write -> timer -> projection`，固定 URL 为
  `http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15`。

## T003 Shared CreateGroupPrerequisite

- Helix：`G01ScenarioRunner::create_group_prerequisite` 已提升为公开、确定性的发送类 UC
  前置阶段；仍只通过 `Tick::Command -> HTTP PortReply -> Persist/Emit` 取得权威 channel，
  不允许测试直接制造 channel 状态。
- PC：新增 domain-neutral `prepareMockCreatedGroup`，G01 适配器只提供 canonical group
  Fixture/rebase；通用 helper 不包含 G01 marker、Fixture ID 或业务成功条件。
- Mobile：新增 `createFreshGroupPrerequisite` 并让旧 `createChannel` 兼容入口委托给它；
  每个 UC 仍调用生产 `im.channels.create`、等待 `im:channel:created` 并核验持久化快照。
- PC focused contracts：
  `yarn message:wdio:contracts --runTestsByPath tests/e2e/wdio/harness/__tests__/fixture-loader.test.ts tests/e2e/wdio/harness/__tests__/tauri-mock-bridge.test.ts`，
  2 suites / 16 tests PASS。
- Mobile compile boundary：`./scripts/run-gtest-js-spec.sh unit --compile-only`，exit 0，
  生产 SDK/QuickJS 链接的 GTest binary 成功生成。
- 结论：三端复用的是同一个“每 case 新建权威群后再执行 UC”合同；Mock 只 replay Helix
  Frame，不在端侧补业务 reducer。

## T004-T008 Helix Producer

- RED 首跑：`cargo test -p helix-im --test g02_full_flow_test -- --nocapture`，3/6 PASS；
  暴露三项真实缺口：业务拒绝 timeout 使用 `PersistFire` 无 Frame barrier、DOCUMENT dialog
  降级 generic media、cold recovery 需要 remote/cache persist 后才能形成 snapshot。
- 生产修复：发送 timeout 改走有回报 terminal Persist，并注册
  `TimelineRefreshAfterSendPersist`；DOCUMENT dialog preview 映射为 typed document。
  `send/flow.rs` 现有 DOCUMENT 路径已确认不创建 upload plan，无需为了测试改写生产入口。
- 共享 runner：G01/G02 共用同一 `ImModule::handle` 回放器；Gate 只注入身份、message type、
  fixed props/time/id。G02 六类 case 全走 `Tick::{Command,PortReply,Inbound,Timer}` 与真实 Effect。
- GREEN：
  `cargo test -p helix-im --test g01_full_flow_test --test g01_wdio_fixture_contract_test --test g02_full_flow_test --test interaction_frame_contract_test -- --nocapture`，
  G01 4/4、G01 Fixture 1/1、G02 6/6、Frame contract 20/20 PASS。
- Frame 字段：schema/runtime/transaction/causation、surface/scope/epoch/revision/baseRevision、
  mutation、directives、message actions、delivery、Document title/summary/access、eventSeq 均有
  typed 断言。late HTTP 与 duplicate WS 在 runner 内断言不重复 terminal write/Frame。

## T009-T012 Canonical Fixture / Helix Green

- Fixture RED：普通
  `cargo test -p helix-im --test g02_wdio_fixture_contract_test -- --nocapture`
  因 `group-created.frame-v2.json` 不存在而 exit 101，证明测试不会静默生成或覆盖 Golden。
- 显式生成：
  `UPDATE_G02_WDIO_FIXTURES=1 cargo test -p helix-im --test g02_wdio_fixture_contract_test -- --nocapture`。
  初始 producer pin 为 `82445ae1b69d10bb01ea183a7fa00df88c80d993`。
- Inventory：`group-created`、`sending`、`sent`、`failed`、`ws-timeline-sent`、
  `ws-left-dialog-list` 共 6 份 Frame；schemaVersion 全为 2。timeline Surface 为
  `message.timeline`，建群/WS 摘要 Surface 为 `message.left-dialog-list`。
- 字段合同：Document `title=打开文档`、`summary/url` 为固定 URL、`placeholder=true`；
  这些业务字段不在 parameterizableFields。动态字段仅限 provenance 声明的 actor/team/channel、
  message identity、transaction/revision/epoch 和 group identity。
- Hash 真源：每份 provenance 同时记录 raw `fixtureSha256` 与包含 Document/dialog typed 语义的
  `semanticSha256`；普通测试逐字比较 canonical JSON 与 provenance。
- 最终命令：
  - `cargo check -p helix-im`：exit 0。
  - `cargo test -p helix-im --test g02_full_flow_test -- --nocapture`：6/6 PASS。
  - `cargo test -p helix-im --test g02_wdio_fixture_contract_test -- --nocapture`：1/1 PASS。
- **Helix Green**：G02 T001-T012 完成；PC/Mobile 只能只读同步这组 producer-authored Fixture。

## T013-T022 PC Mock / Real / Report Green

- single-real 首轮证明 canonical dialog preview 与真实 Go `simpleMessage` 漂移：Fixture 使用完整
  message，而生产 `posts/create` 摘要为 50 Unicode scalar。修复只发生在 Helix Scenario
  producer，`marker` 与 `simple_message` 明确分离；当前 producer pin 为
  `ac15abd0b29ef62ea5d359fb22030800dd0dee2b`，G01/G02 Golden 普通模式均 PASS。
- PC 只读同步 G01/G02 各 6 份 Fixture；inventory、schemaVersion、raw SHA-256、
  semantic SHA-256、surface inventory 与 producer pin 检查全部 PASS。动态替换仍仅允许
  provenance 声明字段及 runtime revision rebase，不允许补写 document/dialog VM。
- `yarn message:wdio:contracts`：27 suites / 91 tests PASS。
- `yarn message:wdio:g02:boundary`：扫描 2735 个 production files，
  `mockProductionReachable=false`。
- `yarn message:wdio:build`：Angular `wdio-e2e` 与 Tauri `wdio-e2e` candidate 均 PASS；
  candidate SHA-256 为
  `ddd2e703f0cb34b3c116c937b5bdea9d83be84fd6859cdd2ef4b2f88abb5d737`。
- Mock：`yarn message:wdio:g02:mock`，happy/failure/WS timeline+dialog 3/3 PASS。
  回归 `yarn message:wdio:g01:mock` 4/4 PASS；AcceptedCommand 与 Frame replay 分离。
- Real：每 case 通过真实 MessageV4 UI 新建 444+678 fresh group，再发送固定文档 URL。
  `single-real` 1/1 PASS，`dual-real` 1/1 PASS；最终报告 source pin 为 client
  `39695bbd00d978d619505a79560de8af2e89631e`、Helix
  `ac15abd0b29ef62ea5d359fb22030800dd0dee2b`。
- 提交后复跑额外复现 HTTP terminal 早于 WS reconciliation：G02 PageObject 曾把 temporary
  sent row 当 canonical row，导致 PG/DOM ID 对账 RED。现与 G01 共用同一观察不变量：
  `delivery=sent` 必须等待非 `helix_tmp_` 且 `messageId != temporaryId`；修复后
  single-real 与 dual-real 再次全绿。
- Reporter：G02 Mock/single-real/dual-real/report 分别保持
  `mock`/`single-real-chain`/`dual-real-chain`/`ui-only` evidence level；
  JSON/JUnit/Markdown 同源，redaction scan PASS。缺少独立 `tauri-driver` 的 diagnostics
  不阻断本仓实际使用的 embedded provider，也未掩盖 readiness 或 case 失败。
- 根 `yarn tsc --noEmit` 仍被既有 Jasmine type globals 与缺失 `@worktile/gantt` 阻塞；
  错误不落在本次 G02 文件。实际 Angular+Tauri candidate 生产编译已通过。
- PC commits：`f2e230d10`（G02 Fixture/Mock/Real/report）与 `39695bbd0`
 （canonical sent DOM 竞态修复）。**PC Green**：T013-T022 完成。
