# Verification Procedure: G-14a

## Source Lock

- Helix HEAD：`6755dab003cab2bcb6bf2e91c8133c72b270c95c`。
- cses-client HEAD：`0d19677657d68238b85ddeba75a8736804c33ec2`；production Tauri Helix pin：`5b8b7db031faf55f1fa52de15bac746a20f82b13`。
- Mobile HEAD：`b9cbbb3bd9c71972528f71411bab1edc93709f36`；production Helix source pin：`8ec08fc8f89564c00015953459de361a231e9e36`。
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
