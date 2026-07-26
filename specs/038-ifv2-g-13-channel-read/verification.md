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
- Happy path proves `im_channels_view` does not carry `userId/unreadCount`, HTTP SUCCESS emits no Frame, and `update_channel` releases exactly one paired dialog/unread Frame only after matching PersistOk.
- Failure/race coverage proves PersistErr, duplicate authority and late PersistOk produce zero additional Frame.
- Recovery coverage rebuilds the read snapshot from durable dialog storage without a platform cache.

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
