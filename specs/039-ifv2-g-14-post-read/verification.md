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
- producer pin：`5b8b7db031faf55f1fa52de15bac746a20f82b13`；provenance raw SHA-256：`bb4137ee4d87566a2505cb094fa2fa6954df6c6c0a99b855c1f9083dfb010cd5`。
- 行为边界：HTTP success 只代表 admission；WS `post_read` 先写 `message.read_bits`，matching PersistOk 后才触发 receipt authority refresh；duplicate/late reply 不重复写或出 Frame。
