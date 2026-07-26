# Verification Procedure: G-08

## Source Lock

| Source | Locked value | Evidence |
|---|---|---|
| Helix HEAD | `4799972d96a3a0f03064587e0f091eeaf9ab72af` | `git rev-parse HEAD` |
| cses-client HEAD | `60ad2d03173b093ccb4d8e42d34d2f6b97c31909` | `git -C .../cses-client rev-parse HEAD` |
| Mobile HEAD | `9ab37d2a1137ebbfa7d14ab0f01533e1b847a5ff` | `git -C .../mobile-qucik-c++ rev-parse HEAD` |
| PC Helix pin | `768dc9b3597523c6c40279e387d85e9f240fde8a` | `cses-client/src-tauri/Cargo.toml` |
| Strict Matrix SHA-256 | `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac` | `shasum -a 256 crates/helix-im/contracts/strict-matrix.v1.json` |
| Prerequisite | G07 `phase=closed`, `event_seq=23`, `next_action=none` | `specs/031-ifv2-g-07-urgent-message/review-ledger.json` |

Source lock recorded at `2026-07-25T17:30:00-07:00`. Helix only contains the
SpecKit-selected `.specify/feature.json` and G08 ledger transition before implementation.
cses-client is clean. Mobile retains eight untracked Helix bundles plus
`tests/.DS_Store`; G08 must not stage, delete, or rewrite them.

## Phase 1 Verdict

- **T001 PASS**: 三仓 HEAD、PC producer pin、Strict Matrix hash 与 G07 CLOSED 前置均已锁定。
- **T002 PASS**: CURRENT/TARGET/GAP 已按当前生产源码和测试重新写入
  [research.md](research.md)；旧 `specs/008-g08-schedule-create` 的完成标记不作为本轮证据。
- **Stop condition**: 当前仍为 `IMPLEMENTATION_ADMITTED`，不是 Helix/PC/Mobile Green。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-08` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
