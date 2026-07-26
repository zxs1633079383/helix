# Verification Procedure: G-10

## T001 Source Lock（2026-07-25）

- Helix HEAD：`48e8d74df519d343d99b6797b6d032644823876f`；G10 开始时仅 mandatory loop 更新本 Feature ledger。
- cses-client HEAD：`a0a083dd6e65051ab60d6d9bb3e044f4b3984f7a`；生产 Tauri 依赖 pin 为 `5308d2b77174e687423c28c862fde42406408456`。
- Mobile HEAD：`3b0ff15feeb95e0d1fe89040d7e9b2ed96a3734f`；既有未跟踪 Helix bundle 与 `tests/.DS_Store` 不属于本 Gate。
- Strict Matrix SHA-256：`b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- 前序 G09：loop `event_seq=19`、`phase=closed`、R1-R4 全 PASS；gate report SHA-256 为 `92dd695adc64a3982b008af5a829eccf18afc7d77baef151365ec71c964b6430`。
- G10 admission：loop `event_seq=7`、`phase=implementing`、`IMPLEMENTATION_ADMITTED`。全局 SpecKit current-feature 仍指向 G09，因此 G10 全程必须显式传 `--feature specs/034-ifv2-g-10-timeline-window`，不得让默认脚本误写 G09。

## T003-T008 Helix Producer Revalidation

- Shared prerequisite：G10 复用 `G01ScenarioRunner::create_group_prerequisite` 的生产 `im_create_channel -> HTTP authority -> PersistFire(channel/member) -> message.left-dialog-list Frame`，未新增第二套建群 reducer。
- Production handler：`local_first.rs` 现有实现已满足 local scan、conditional remote read、cache persist barrier、query generation/epoch、complete snapshot/patch 与 failed Frame；本阶段未为测试修改生产 handler/projector。
- `cargo check -p helix-im`：PASS，退出码 0。
- 首轮 focused test：7/8 PASS；唯一失败揭示 cache persist failure 不建立 coverage，因此只读 remote Frame 的 `hasOlder=false`，不是完整 durable coverage。测试合同修正为明确区分 durable cache success 与 cache failure。
- 批量修正后 `cargo test -p helix-im --test g10_full_flow_test --test g10_timeline_window_interaction_test -- --nocapture`：PASS；full-flow 8/8、interaction 3/3、fail=0、skip=0。
- 六类场景：prerequisite、happy、business invalid input、HTTP failure、cache persist failure、late/duplicate race、local coverage recovery 均由公开 `ImModule::handle` 或公开 command parser 验证。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-10` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
