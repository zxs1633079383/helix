# Verification Procedure: G-14

## Source Lock

- Helix HEAD：`d381e42eae0955b5231a55b54fa3eade1ddb81ca`
- cses-client HEAD：`e67384f9a1c78bb2da3aafe4ef4206563d231aa4`
- Mobile HEAD：`fe0fa86a3bc27bf3ba416c3c7f07c89b6c80ecbb`
- cses-client production Helix pin：`c8768f8b6f169f67509c0ec717ef6902c7779e19`
- Mobile Fixture/source pin：`c8768f8b6f169f67509c0ec717ef6902c7779e19`
- Strict Matrix SHA-256：`8b551c0d7aead1e44bbba355f65d65f17c883d84fbb2090854f0942bb4904fb7`
- 前序 G-13：`specs/038-ifv2-g-13-channel-read/review-ledger.json` 为 `phase=closed`，R1-R4 全部 PASS。
- 工作树边界：cses-client clean；Mobile 只存在用户既有的未跟踪 bundle 与 `tests/.DS_Store`，本 Gate 不覆盖；Helix 仅有当前 Spec 状态与 `SESSION.md` 改动。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-14` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Helix Green

- `cargo check -p helix-im`：exit 0，7.81s。
- `cargo test -p helix-im --test g14_full_flow_test --test g14_receipt_template_interaction_test -- --nocapture`：13/13 PASS，fail=0，skip=0。
- `cargo test -p helix-im --lib --test s1_ws_actions_test post_read -- --nocapture`：lib 3/3、S1 3/3 PASS，证明既有 post_read 合同已同步升级为 PersistOk 后释放。
- `UPDATE_G14_WDIO_FIXTURES=1 cargo test -p helix-im --test g14_wdio_fixture_contract_test -- --nocapture`：1/1 PASS，仅本次显式生成 canonical JSON。
- `cargo test -p helix-im --test g14_wdio_fixture_contract_test -- --nocapture`：1/1 PASS，普通运行只比较 production actual 与 checked-in Golden。
- canonical inventory：6 份 Frame；schemaVersion 全为 2；surface inventory 为 `message.left-dialog-list`、`message.timeline` 与 `message.participant-receipts`。`readable-message.frame-v2.json` 提供 incoming message 与 enabled `markRead` binding；`receipt-viewable-message.frame-v2.json` 提供 own message 与 enabled `viewReceipt` binding。PC/Mobile 只能执行 producer-authored action，不得手写领域意图。
- producer pin：`5b8b7db031faf55f1fa52de15bac746a20f82b13`；最终 Helix HEAD：`8ec08fc8f89564c00015953459de361a231e9e36`；provenance raw SHA-256：`245732f0437ab005965948ede08f4fc6bb99e0f906a30255d473cbe60c7f6ade`。
- 行为边界：HTTP success 只代表 admission；WS `post_read` 先写 `message.read_bits`，matching PersistOk 后才触发 receipt authority refresh；duplicate/late reply 不重复写或出 Frame。

## PC Consumer Green

- **PC implementation commit**：`0d19677657d68238b85ddeba75a8736804c33ec2`；production Tauri Helix pin 为 `5b8b7db031faf55f1fa52de15bac746a20f82b13`。
- Fixture parity、typed loader、TauriMockBridge 与 reporter focused Jest：4 suites / 42 tests PASS；`message:wdio:g14:boundary` 扫描 2735 文件并证明 `mockProductionReachable=false`。
- Mock run `mock-20260726123936009-76805`：2/2 PASS、retry=0、exit=0、claim=`mock`。
- Single-real run `single-real-20260726124045771-77766`：1/1 PASS、retry=0、exit=0、claim=`single-real-chain`；真实 UI fresh group、Tauri invoke、Helix bus ingress、FrameApplied 与 DOM 闭合。
- Dual-real run `dual-real-20260726124219026-79159`：1/1 PASS、retry=0、exit=0、claim=`dual-real-chain`；Actor/Observer 两进程按 correlation 自动等待单条已读和回执结果。
- Report run `report-20260726124032438-77545`：1/1 PASS、exit=0、redaction=`pass`。四 lane candidate 均为 `13f1086b...af5`；运行时 client source 为提交前 `e67384f...`，随后 `0d19677...` 只固化同一已验证改动。
- 首次 dual-real 曾发生 WebDriver trace clear script timeout，立即按同一代码和 candidate 重跑后通过；非致命 `tauri-driver not found` 诊断未阻断实际嵌入 driver session，不冒充 readiness 证据。

## Mobile Consumer Green

- **Mobile implementation commit**：`b9cbbb3bd9c71972528f71411bab1edc93709f36`；生产 Helix source pin 为 `8ec08fc8f89564c00015953459de361a231e9e36`，包含 producer `5b8b7db...`。
- `G14FrameContractTest.*` 在 surface-runtime 与 production-linked unit 两个 binary 均为 6/6 PASS，覆盖完整 envelope、fresh group、pending/completed/recovery authority、原子拒绝、SDK command admission、DomainEvent batch、C ABI 与 QuickJS typed view。
- 全量 SurfaceRuntime 回归 130 PASS / 1 SKIP；唯一 skip 是需要显式 source root 的 parity case，该证据由 `check-g14-fixture-parity.mjs` 与 `check-surface-runtime-source-pin.mjs` 独立执行并 PASS，不将 skip 伪报为 pass。
- source traceability、V1 hard-cut、source pin、Strict Matrix 27/27、共享 Venus QuickJS smoke 与 no-JS-facade 均 PASS。G14 未改变 Venus/QuickJS 接线能力，因此按合同复用共享 smoke，不新增专属 Venus Gate。
- 生产缺口修复只允许 `read` receipt 在服务端无逐接收人时间时省略 `completedAt`；其他 receipt kind 仍要求完成时间。业务 receipt authority 未移入 Mobile。

## Cross-repo Fixture Parity

- 三仓 inventory 均为 6 份 Frame + provenance，schemaVersion 全为 2，producer 均为 `5b8b7db031faf55f1fa52de15bac746a20f82b13`，byte-exact parity PASS。
- `group-created.frame-v2.json`：raw `32b88a2ce1592350262576a5958a5452db23029ba006cde7489abb12a66aedd4`，semantic `4ac33ad554d6f0b53a93e834fd1f19744ae4892fd6287a413a4a500ed1637626`。
- `readable-message.frame-v2.json`：raw `b623c4325dafb7af1b292c2b4c9d318cf04bd8f406585bf80765c3ad55ae10db`，semantic `9c4a671ad94719e261cb652960fc9369367168ae37511d8bf222ca300e1e5013`。
- `receipt-viewable-message.frame-v2.json`：raw `248e305013ac2af8b6ada80d190b243f8b98d54b8b838dee28a0c5f1d77728a8`，semantic `1149558f32d73b423f26cb95e90b9152e078ad48c26e959e192230849c795a32`。
- `receipts-pending.frame-v2.json`：raw `0ab72012aacc0eca9bcd0360287415a02f6b2585bbf8690cc8a9f937ed5af12c`，semantic `cbe3cd12b8b4548b1314f834d7751c16f8d9f08be334e91ed9c7ea30768f8514`。
- `post-read-completed.frame-v2.json`：raw `dbcd3e4452bf6991672454ddf777d5d2cef29593f67ace7cfe229719073b2730`，semantic `5b24600d23a0885f3f5c4531628d75f82814397eea028c2b457386ffdbcc80b5`。
- `post-read-recovery.frame-v2.json`：raw `feafd169cd013da389905ee265dfe44a343af038066b1f017fede28ab471a62f`，semantic `5b24600d23a0885f3f5c4531628d75f82814397eea028c2b457386ffdbcc80b5`。
- `provenance.json` 三仓 raw SHA-256 均为 `245732f0437ab005965948ede08f4fc6bb99e0f906a30255d473cbe60c7f6ade`。

## Review

### Behavior

- **Green**：G14 单条 `im_post_read` 与 G13 会话级 `im_channels_view` 保持独立；HTTP success 仅 admission，WS `post_read` + matching PersistOk 才释放 receipt authority Frame。
- **Green**：业务/基础设施失败保留既有回执状态；duplicate authority、late reply 不重复写或出 Frame；durable recovery 不依赖平台 Store。
- **Green**：每个 canonical/real run 使用 fresh group；Actor 可读取 incoming message，Observer 对 Actor own message 的 pending/completed receipt 可见，平台不自行推导接收人集合。

### Code Boundary

- **Green**：唯一 producer 是生产 `ImModule::handle -> Effect::Persist -> PortReply(PersistOk) -> Effect::Emit`，ScenarioRunner 只注入 Tick/PortReply/Inbound 并收集 Emit。
- **Green**：PC Fixture/Mock/PageObject/WDIO API 仅在测试目录，production boundary PASS；Angular 只在用户打开回执入口时 attach surface 并调用生产 command，不在 timeline apply 时私自查询或归约。
- **Green**：Mobile GTest 链接生产 `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch -> SurfaceRuntime -> QuickJS im.surface.view`；默认 production build 不定义 legacy characterization macro，不注册 raw reducer/test injection。

### Frame V2

- **Green**：完整 envelope 含 `schemaVersion=2`、runtime、transactionId、mutations、directives；scope/epoch/revision/baseRevision 全由 Helix producer 给出。
- **Green**：timeline incoming row 含 enabled `markRead`，own-message row 含 enabled `viewReceipt`；平台通过 producer-authored ActionBinding/opaque invocation 调用，不补业务字段。
- **Green**：participant-receipts pending/completed/recovery 都保持 `channelId/messageId/kind/state/summary/completed/pending` 字段级合同；unknown schema/surface/revision gap/duplicate 整帧 fail-closed。

## Evidence Manifest

- `evidence/evidence.json` 已登记 V-001～V-007 的真实命令输出和 SHA-256；首次错误 Jest config 输出保留为 `V-004-pc-mock-attempt1.log`，没有用最终 PASS 覆盖失败证据。
- 汇总报告：`evidence/gate-report.json`，`finalVerdict=Green`；最终 SpecKit R4/closure 仍以 `review-ledger.json` 的机器结果为准。
