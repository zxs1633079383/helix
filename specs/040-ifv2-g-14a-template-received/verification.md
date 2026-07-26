# Verification Procedure: G-14a

## Source Lock

- Helix 最终实现 HEAD：`f7e7ecf041427559e23ca96a1e982eb8a607e94b`；canonical Fixture producer：`76da0e898db39e2f343d36790ad2ccfbb92bf154`。
- cses-client 最终实现 HEAD：`5b4c5fab0d176024658379cf2f1d91830819903c`。
- Mobile 最终实现 HEAD：`bd534176ec8a7be9247a0ef986499eaf4ad2ae5f`；production Helix source pin：`f7e7ecf041427559e23ca96a1e982eb8a607e94b`。
- Strict Matrix SHA-256：`8b551c0d7aead1e44bbba355f65d65f17c883d84fbb2090854f0942bb4904fb7`。
- 前序 G-14：`specs/039-ifv2-g-14-post-read/review-ledger.json` 为 `phase=closed`，R1-R4 全 PASS；close commit 为 `6755dab003cab2bcb6bf2e91c8133c72b270c95c`。
- 工作树边界：Helix 与 PC clean；Mobile 仅存在用户既有未跟踪 bundle 与 `tests/.DS_Store`，本 Gate 不覆盖。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-14a` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Helix Scenario Evidence

- `cargo test -p helix-im --lib --test g14a_full_flow_test --test g14_receipt_template_interaction_test -- --nocapture`：退出码 0；lib 356/356、G14/G14a receipt 8/8、G14a full-flow 7/7。
- happy path 已证明 `im_template_received -> HTTP admission -> post_update -> PersistAtomic -> PersistOk -> message.timeline Frame`；HTTP 失败、持久失败重放、重复 WS、迟到 HTTP 与 cold recovery 均 fail-closed。
- Frame 字段 Review 发现并修复模板 `userIds` 污染普通 `readReceipt.readerIds` 的 producer 缺口；模板确认和普通已读回执现由独立 authority 驱动。
- `UPDATE_G14A_WDIO_FIXTURES=1 cargo test -p helix-im --test g14a_wdio_fixture_contract_test -- --nocapture`：退出码 0，生成 4 份 Frame 与 provenance。
- `cargo check -p helix-im && cargo test -p helix-im --test g14a_wdio_fixture_contract_test -- --nocapture`：退出码 0；普通模式只比较 Golden，Fixture 1/1 通过。

## PC Consumer Green

- **PC implementation commits**：`421f5a514` 接入 G14a mirror/loader/TauriMockBridge；`5b4c5fab0` 完成 Mock、single-real、dual-real、report 与 boundary。
- `yarn message:wdio:g14a:mock`：2/2 PASS；完整 Helix Fixture 经 `TauriMockBridge.replay()` 进入真实 Angular `SurfaceRuntime`，成功态和 admission rejection 均由 producer-authored Frame 驱动。
- focused Jest 使用 `tests/e2e/wdio/jest.config.js`：4 suites / 44 tests PASS，覆盖 raw/semantic parity、typed loader、AcceptedCommand 与 Frame replay 分离、reporter 合同。首次误用根 Jest config 得到 `No tests found`，原始失败保留在 V-004，不用最终 PASS 覆盖。
- `yarn message:wdio:g14a:report`：1/1 PASS；`yarn message:wdio:g14a:boundary` 扫描 2735 个 production 文件，`mockProductionReachable=false`。
- 当前代码重新执行 `g14a:single-real` 与 `g14a:dual-real`：各 1/1 PASS；非致命 `tauri-driver not found` 诊断没有阻断实际 WebKit/Multiremote session。
- 持久真实链证据包 `mrc-g14a-repair-1784775900` 为 `runtimeStatus=pass`、`canonicalStatus=green`、26/26 observable assertion PASS、14 个 required facet 齐全、`missingFacets=[]`。同一 `reqId/channelId/messageId/confirmUserId` 关联真实 `im_template_received`、Go HTTP、Go WS、远端读回、Helix projection/local readback、双账号 DOM 与 no-mock marker。

## Mobile Consumer Green

- **Mobile implementation commit**：`bd534176ec8a7be9247a0ef986499eaf4ad2ae5f`；4 份 G14a Fixture byte-exact mirror，production source pin 为 `f7e7ecf...`。
- `G14AFrameContractTest.*` 在 standalone SurfaceRuntime binary 为 6/6 PASS；同一合同链接 production SDK/Bridge/FFI/SurfaceRuntime/QuickJS binary 后与 `Uc14TemplateReceivedTest.*` 合计 14/14 PASS。
- 覆盖完整 envelope、fresh group prerequisite、template authority/recovery、整帧原子拒绝、DomainEvent batch、production C ABI 与 QuickJS `im.surface.view` typed view。
- hard-cut 扫描证明截至 G14a 的 production V1 producer=0、consumer=0；source pin、source traceability 20 cases、UC contract 27 Gates、Strict Matrix 27/27、no-JS-facade 与共享 Venus QuickJS smoke 均 PASS。
- G14a 未修改 Venus/QuickJS 接线能力，按合同使用共享 Venus smoke，不新增 Gate 专属 Venus runner。测试 binary 中的历史 `LegacyCharacterization` 仅由显式测试宏编译，production boundary 扫描证明默认产品构建不可达。
- 额外执行旧 real-chain UC14 lane 时，真实建群与发帖成功，随后在 `go-post-readback` 等待 `im:messages:query_result` 超时；该 lane 属 legacy characterization，不替代也不否定本次 Mobile Contract Green，缺口如实保留为 non-required blocker。

## Cross-repo Fixture Parity

- 三仓 inventory 均为 4 份 Frame + provenance；逐文件 `cmp` 通过，schemaVersion 全为 2，provenance producer 均为 `G14AScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode`。
- `group-created.frame-v2.json`：raw `cfba94d611b02378f24350469f6cdd0d2f6d03f32a60e1c35688d1174f68b178`，semantic `9b37ad958a2246a744601aa0267f90fa81a33e85e705f56f3a07204398d6b872`。
- `template-receivable.frame-v2.json`：raw `f78bfa1f8a535375fbacaccd3df5308b8988dd4a883b253a7b92e88661d29e9a`，semantic `1ad149a8e5bc4a7df237e18ae63ce200514acb2104b5a2d1b3f21e2dc39090d8`。
- `template-received.frame-v2.json`：raw `e0e2eba0544ec76407b876cbeb778715eef926f2aa8352e78d799d08326de87e`，semantic `4cdece1da627ddb0b863f2ecfcbe337166b2307079cbfc1236c861c314f38118`。
- `template-received-recovery.frame-v2.json`：raw `9fb5591efeaae67a90c42717a29f81a2612530bd6425f74e379ad7abbd69570f`，semantic `dceb7dae2eb67a7ce076624d1554a5dae85fec1378318e104c606b83ed0904bd`。
- `provenance.json` 三仓 raw SHA-256 均为 `62e2904e9f1b8808a42044ba117e1987193677a76b745ffe3167e3942885533e`。

## Review

### Behavior

- **Green**：fresh group 是独立 prerequisite；发送 TEMPLATE 与 `im_template_received` 是两项真实生产命令，平台不借前序 Gate 遗留数据。
- **Green**：HTTP 仅 admission；WS `post_update` 经持久化屏障后才产生最终 template authority Frame。业务/基础设施失败、重复 WS、迟到 HTTP 与 cold recovery 均由 Helix sans-IO 场景证实。
- **Green**：Actor/Observer 最终都显示 `confirmedCount=1` 和确认人 `678`；Actor 不被伪造为 reader，Observer 重复确认入口消失，普通 `readReceipt.readerIds` 不受模板确认名单污染。

### Code Boundary

- **Green**：唯一 reducer 为 production `ImModule::handle -> Effect -> PortReply/Inbound -> Effect::Emit`；ScenarioRunner 只注入确定性输入并捕获 Emit。
- **Green**：PC Fixture、Mock bridge、PageObject 与 WDIO reporter 仅在测试目录，production boundary 为 false；Angular 不补字段、不重算确认状态。
- **Green**：Mobile GTest 链接 production `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch -> SurfaceRuntime -> QuickJS binding`；默认产品构建不注册测试注入 API、raw reducer 或 V1 producer/consumer。

### Frame V2

- **Green**：所有 Frame 均含 `schemaVersion=2`、runtime、transactionId、mutations、directives；scope/epoch/revision/baseRevision 均由 Helix 给出。
- **Green**：初始 timeline 为 revision 1 snapshot；确认终态为 revision 2/baseRevision 1 patch；cold recovery 重新产生 revision 1 snapshot；`directives=[]`。
- **Green**：`templateConfirm` ActionBinding 从 receivable 的 enabled 状态收敛为终态 unavailable；`templateConfirmation.readerIds=["g14a-wdio-actor"]` 与普通 readReceipt 保持不同 authority。unknown schema/surface/revision gap 均整帧 fail-closed。

## Evidence Manifest

- `evidence/evidence.json` 登记 V-001～V-007 的命令、时间、结果、文件与 SHA-256；Mock、Real、Mobile 与跨仓 Review 分离，任一平面不能替代另一平面。
- 汇总报告为 `evidence/gate-report.json`，`finalVerdict=Green`；最终关闭仍以 `review-ledger.json` 的 R4 机器 verdict 为准。
