# Verification Procedure: G-13

## Source Lock

- **Helix HEAD**: `c7bf170d7aad8b2afc8185b36ab447b50bd27fa8` at implementation admission.
- **cses-client HEAD**: `6174658be34a779ded68311848cd5c4851ec9d01`.
- **Mobile HEAD**: `42819464ea4746a888a62e60f14dce18dcc27674`.
- **Downstream Helix pin**: PC/Mobile both start from `1b797413f2c1b14abde5eacf60eb942b21e9d650`; repin only after a committed G13 production producer exists.
- **Strict Matrix**: source-grounded G13 correction raw SHA-256 `8b551c0d7aead1e44bbba355f65d65f17c883d84fbb2090854f0942bb4904fb7`.
- **Prerequisite**: G12 `review-ledger.json` is `phase=closed`, `R4=pass`; Helix close commit is `c7bf170d7aad8b2afc8185b36ab447b50bd27fa8`.

## Current / Target / Gap

- **Current command**: `im_channels_view` in `outbound/channel_admin/change_dedicated.rs` emits only `POST /channels/view`; its HTTP reply is admission, not read-state authority.
- **Current authority**: `ws/handlers/update_channel.rs` parses the current viewer's absolute member dialog fields and can project paired dialog/unread state, but presently emits before a correlated persistence acknowledgement.
- **Target**: `update_channel -> Effect::Persist(channel/channel_member) -> Tick::PortReply(PersistOk) -> one InteractionFrameV2(message.left-dialog-list + message.unread-summary)`.
- **Gap**: add the Persist barrier, deterministic scenario/Fixture, PC Mock/Real/report, and Mobile production-linked typed-view contract. `post_read` and `message.participant-receipts` remain G14.

## Helix Producer Evidence

- `cargo check -p helix-im`: PASS, exit 0.
- `cargo test -p helix-im --test g13_full_flow_test --test g13_channel_read_interaction_test -- --nocapture`: PASS, 12/12.
- `cargo test -p helix-im --test g13_wdio_fixture_contract_test -- --nocapture`: PASS, 1/1 in ordinary Golden-compare mode.
- Happy path proves `im_channels_view` does not carry `userId/unreadCount`, HTTP SUCCESS emits no Frame, and `update_channel` releases exactly one paired dialog/unread Frame only after matching PersistOk.
- Failure/race coverage proves PersistErr, duplicate authority and late PersistOk produce zero additional Frame.
- Recovery coverage rebuilds the read snapshot from durable dialog storage without a platform cache.
- Canonical inventory is four production Frames: `group-created`, `dialog-unread-initial`, `channel-read`, `channel-read-recovery`; producer revision is `a13e17151efd1e7eefc94677110598b6347b08a9`.
- Raw SHA-256: group `15be2554...1600e`, initial `a91573f5...5e44`, terminal `6b00cf38...bf98`, recovery `8a38be83...249d`; provenance `89d19011...0286`.

## PC Consumer Evidence

- **PC implementation commit**: `e67384f9a1c78bb2da3aafe4ef4206563d231aa4`.
- Fixture parity、typed loader、TauriMockBridge receipt/Frame split、G13 PageObject 与 reporter focused Jest：4 suites / 40 tests PASS；TypeScript、candidate build、command API 与 production-boundary gate 均 PASS。
- Mock run `mock-20260726101922965-81298`: 3/3 PASS、retry=0、exit=0、claim=`mock`，candidate `85113ff5...7dbd`。
- Single-real run `single-real-20260726102414009-84034`: 1/1 PASS、retry=0、exit=0、claim=`single-real-chain`，真实 Tauri invoke/bus/FrameApplied/DOM 形成闭环，candidate `83300957...e5e`。
- Dual-real run `dual-real-20260726102844648-86724`: 1/1 PASS、retry=0、exit=0、claim=`dual-real-chain`；Observer 发送后 Actor 获得正未读，Actor 执行 read 后只清理自己的 dialog/unread，candidate 与 single-real 相同。
- Report run `report-20260726101954458-81579`: 1/1 PASS、exit=0、redaction=`pass`。四条 lane 的 metadata 均钉住 Helix `c8768f8...`；实际执行时 PC source pin 为 `6174658b...`，后续提交只固化已验证测试代码。
- 内嵌 driver 曾输出非致命 `tauri-driver not found` 诊断，但实际 Tauri/WebKit session、single-real 与 dual-real 均退出 0；不以该诊断替代真实 lane 结果。

## Mobile Consumer Evidence

- **Mobile source-pin commit**: `a81c1de`; **G13 contract commit**: `fe0fa86a3bc27bf3ba416c3c7f07c89b6c80ecbb`。
- `G13FrameContractTest.*`: 6/6 PASS，覆盖完整 envelope、fresh-group、paired atomic apply、schema/surface/gap/duplicate fail-closed、生产 SDK DomainEvent batch、QuickJS typed view 与 C ABI ack。
- `run-uc01-16-gtest.sh --only UC13`: 8/8 PASS；runId `mrc-mobile-uc-20260726105047-128`，真实链创建 fresh group，Observer typed unread 从正数变为零，`clientReducer=false`，cleanup 完成。
- Fixture parity、UC source traceability、source pin、legacy hard-cut、command API、Strict Matrix `27/27` 与共享 Venus smoke 均 PASS。
- `make uc-contract-gate` 最终 PASS。Gate runner 只执行生成 native contract artifact 的 focused test，避免同一 legacy characterization binary 使 V2 正向测试错误暴露 `im.store`；production/surface-runtime build 仍不定义该 macro。

## Cross-repo Fixture Parity

- 三仓 inventory 均为四份 Frame + provenance，schemaVersion 均为 2，producer 均为 `a13e17151efd1e7eefc94677110598b6347b08a9`。
- `group-created.frame-v2.json`: raw `15be2554cb439f6fedc60717a1fe809574c288a150da81466e17957c3c41600e`，semantic `1c2958a3062badf1ba704ee4b21c5bdaba395b963374bb8ec5a278cf357284e1`。
- `dialog-unread-initial.frame-v2.json`: raw `a91573f53312f8c307ae49fc2a5cfd63e3cc33f118917c567b033a6170f95e44`，semantic `e2217f10d146b08d3e3cc6753108bfa4d361657a417d7f0f1e8932f62a6512f2`。
- `channel-read.frame-v2.json`: raw `6b00cf380cbef1e8dabdcd432d776927ae0dade6e7e0ae18409a9d1dff00bf98`，semantic `c7cf03c217463c519289fc2f6e7bfb17a0a07860ca47ccb0b1c766f3bc86e3dc`。
- `channel-read-recovery.frame-v2.json`: raw `8a38be83af62331801a27278051bf6c31a0da69eb1c6a6bdf4fedb1ddf92249d`，semantic `6e63b9e7ae3cf09afb3a2f1bbdd3b8bce045d1823aed644777a72e399c47bdfa`。
- `provenance.json` 三仓 raw SHA-256 均为 `aea3480b12bbf40e6063a097e7d91eeffdcaa380919ddf7c964d61b34cc6acb6`，byte-exact parity PASS。

## Review

### Behavior

- **Green**：G13 只负责会话级 `im_channels_view`；HTTP SUCCESS 仅为 admission，最终未读取自 viewer 定向 `update_channel` 的绝对 member fact。G14 的 `im_post_read`/participant receipts 未混入。
- **Green**：PersistErr、重复 authority、迟到 PersistOk 不发布额外终态；durable recovery 可重建相同 paired state。
- **Green**：每个 canonical/real run 都新建群；Actor/Observer 结果按 viewer 隔离，未使用前一 Gate 遗留群。

### Code Boundary

- **Green**：唯一业务 producer 是生产 `ImModule::handle -> Effect::Persist -> PortReply(PersistOk) -> Effect::Emit`；Fixture producer 测试未复制 reducer。
- **Green**：PC Fixture/Mock/PageObject/WDIO API 仅存在于测试目录且 production bundle boundary PASS；Angular 只 apply Frame。
- **Green**：Mobile GTest 链接生产 `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch -> SurfaceRuntime -> QuickJS im.surface.view`。production build 不定义 `MOBILE_ENABLE_LEGACY_V1_CHARACTERIZATION`，raw reducer/test injection 不可达。

### Frame V2

- **Green**：完整 envelope 含 `schemaVersion=2`、runtime、transactionId、mutations、directives；G13 read terminal 是同一 transaction 下的 `message.left-dialog-list` + `message.unread-summary` paired mutations。
- **Green**：scope 为 viewer user/team，surfaceEpoch=1；initial snapshot revision=1/baseRevision=null，terminal patch revision=2/baseRevision=1，recovery snapshot 可独立重建。
- **Green**：本 Gate 无 ActionBinding/UiDirective 是明确语义，不由平台补造；duplicate/stale/gap/unknown schema/surface 均整帧 fail-closed。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-13` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
