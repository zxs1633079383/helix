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

## Helix Producer Evidence

| Plane | Command | Result | Boundary |
|---|---|---|---|
| Build | `cargo check -p helix-im` | PASS, exit 0, 5.00s | production crate |
| Full flow | `cargo test -p helix-im --test g08_full_flow_test -- --nocapture` | PASS, 7/7, 6.43s | prerequisite/happy/business/infrastructure/race/persist/recovery |
| Fixture update | `UPDATE_G08_WDIO_FIXTURES=1 cargo test -p helix-im --test g08_wdio_fixture_contract_test -- --nocapture` | PASS, 1/1, 6.21s | explicit canonical export only |
| Golden compare | `cargo test -p helix-im --test g08_wdio_fixture_contract_test -- --nocapture` | PASS, 1/1, 0.12s | ordinary run cannot rewrite Fixture |
| Schedule regression | `outbound_commands` + `p2_posts_outbound_test` + `projection_schema_test` | PASS, 13/13 + 18/18 + 47/47 | command/wire/helper contracts |
| WS regression | `cargo test -p helix-im --test s1_ws_actions_test schedule_created_sets_flag_s1 -- --nocapture` | PASS, 1/1 | raw G08 producer absent; atomic writes present |

Production commits:

- `8358082b3fd01b96b332c79deca8d2017315dd19`: strict command, HTTP/WS correlation,
  `channel_schedule` authority fact, atomic read model, typed channel-summary and seven scenarios.
- `6880f003f20a206790c94e0c478d17e9282dc708`: four canonical Frames plus producer/hash provenance.

Canonical inventory:

- `group-created.frame-v2.json`
- `channel-summary-empty.frame-v2.json`
- `channel-summary-scheduled.frame-v2.json`
- `channel-summary-recovery.frame-v2.json`

**Helix verdict**: T004-T012 Green. T003 remains open until PC and Mobile both prove that
their canonical cases reuse the same fresh-group prerequisite. G08 as a whole remains
`implementing`; Mock/Real/Mobile/Review evidence is not yet available.

## PC Consumer Evidence

| Plane | Command | Result | Boundary |
|---|---|---|---|
| Types | `npx tsc -p tests/e2e/wdio/tsconfig.json --noEmit` | PASS, exit 0 | WDIO typed harness |
| Contracts | `yarn message:wdio:contracts` | PASS, 41 suites / 136 tests | schema/fixture/bridge/report parity |
| Mock | `yarn message:wdio:g08:mock` | PASS, 4/4 | happy/admission failure/race/recovery |
| Single real | `yarn message:wdio:g08:single-real` | PASS, 1/1 | fresh group + invoke/SQLite/PG/HTTP/WS/FrameApplied/DOM |
| Dual real | `yarn message:wdio:g08:dual-real` | PASS, 1/1 | Actor owner draft + Observer non-leak |
| Report | `yarn message:wdio:g08:report` | PASS, 1/1 | Gate-specific fail-closed summary |
| Boundary | `yarn message:wdio:g08:boundary` | PASS, 2735 files | `mockProductionReachable=false` |
| Native E2E build | `cargo build --features wdio-e2e --bin cses-client` | PASS, exit 0 | real embedded driver binary |
| Native correlation unit | `cargo test --features wdio-e2e native_e2e_correlation_is_replaceable_without_reading_a_new_environment -- --nocapture` | PASS, 1/1 | E2E-only request correlation |
| Default cargo check | `cargo check --bin cses-client` | BLOCKED, exit 101 | pre-existing capability discovery sees `wdio:allow-execute` without optional plugin; not used as Green evidence |

Production and test commits:

- `17229c817`: G08 typed production projection, canonical Fixture sync/loader/bridge
  contracts and initial WDIO consumer wiring.
- `dd4ff5ce1`: production Helix pin `8358082`, plus explicitly feature-gated native
  WDIO scenario correlation.
- `033131ca0`: G08 Mock/single-real/dual-real/report/boundary suites and evidence
  collection.

PC applies only producer-authored `InteractionFrameV2`. The Mock lane replaces command
admission and replays canonical Frames through the production bus ingress; it does not
construct schedule state. The real lanes create a new two-user group per case. Go evidence
proves exactly one verified `post_schedule_created` delivery to Actor `444`; Observer `678`
has neither a schedule banner nor a local `channel_schedule` row.

**PC verdict**: T013-T022 Green. The non-zero default `cargo check` is recorded as a
separate pre-existing capability-discovery gap; candidate build, native WDIO build,
production-boundary scan and both real lanes passed. G08 remains `implementing` until
T023-T038 close Mobile and independent review.

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
