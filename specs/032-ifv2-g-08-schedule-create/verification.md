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

## T003 Shared CreateGroupPrerequisite

- Helix 的 `G08ScenarioRunner` 复用 G01 production prerequisite，经
  `im_create_channel` 形成权威 `channelId` 后才执行 `im_create_schedule`；不复制建群
  handler，也不依赖前序 Gate 遗留 channel。
- PC 的 single-real/dual-real 均从 MessageV4 UI 创建新的 Actor `444`、Observer
  `678` 双用户群，再在该群执行 G08；Mock 只 replay Helix canonical Frame。
- Mobile 的 `Uc09ScheduleTest.RealChain` 复用
  `UcJsContractFixture::RC.createChannel('schedule')`，run
  `mrc-mobile-uc-20260726020531-27390` 产生 fresh channel
  `6ypfee18ujnpidedyjcsdeg4by`，随后才进入 typed schedule summary。
- 三端均复用已有 CreateGroupPrerequisite 能力，没有新增第二套建群业务 reducer。

## T023-T032 Mobile Contract

- Mobile 生产接受提交为 `e67c5c5779e2bf381d64db6e235b3abe884a8cb9`；
  Helix source lock 为 `e9003261d42b8666ef27fec6798bbb82f1093d7a`，
  bundle `helix-e900326-g08.bundle` SHA-256 为
  `737fdab3b55a9eb52ba7a398c2fc7f3a083b17d69b3180ee8afddbeaf84f1cdf`。
- G08 mirror parity 通过：`count=4`，fixture producer 为 `8358082b...`；
  source-pin 通过且 G08 inventory=`4`；legacy hard-cut 通过，production V1
  producer/consumer/raw fallback 均为 `0`。
- `G08FrameContractTest.*` 为 `5/5`；UC09 非 real 聚焦合同为 `9/9`；
  `./scripts/run-uc01-16-gtest.sh --only UC09` 为 `10/10`。生产
  MobileImSdk command admission、MobileFfiBridge DomainEvent batch、
  SurfaceRuntime atomic apply、C ABI 与 QuickJS `im.surface.view` 均由同一测试链覆盖。
- 共享 `./scripts/run-gtest-js-spec.sh surface-runtime` 在修正 `viewerRole`
  扩展字段的兼容边界后为 `89 passed`，另有 `1` 个要求显式 source 环境的既有
  parity case skip；该证据面由单独通过的 source-pin Gate 补足。
- 共享 `./scripts/run-uc01-16-gtest.sh` 已真实执行，runId
  `mrc-mobile-uc-20260726020531-27390`，总结果 `139/150`。G08 的
  `Uc09ScheduleTest.RealChain` 通过；11 个失败来自 GS02/GS03/GS04、
  UC01/UC02/UC10-UC15 的其他 Gate real lane，均为 query/sync/topic 等待超时，
  与上一 Gate 的共享回归失败集合一致。因此不计入 G08 required lane，但继续作为
  对应后续 Gate 的真实 blocker，且不把 Mobile 跨 Gate 全集宣称为全绿。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 三份 provenance raw SHA-256 均为
  `bd6f6bf476e198eb290201d792a81618a69704cef167d61289e233b3aafe969c`。
- 三仓 inventory 均为同一 `4` 份 Frame，schemaVersion 全为 `2`，surface
  inventory 为 `message.left-dialog-list` 与 `message.channel-summary`。
- 四份 Frame raw SHA-256 在三仓逐文件均为 `3/3` 相同：
  `group-created=f692c464...`、`summary-empty=f42136f3...`、
  `summary-scheduled=df3ad5de...`、`summary-recovery=39825bbb...`。
- provenance 内 semantic SHA-256 也由 PC/Mobile parity gate 逐项校验，
  动态替换仅限 manifest 明示的 correlation/scope/revision 字段。

## T034-T036 Independent Review

- **Behavior Green**：fresh group 前置、owner 权限、scheduled 终态、持久化失败、
  duplicate/late authority、恢复、Actor 可见与 Observer 不泄漏均由 Helix 同一
  `channel_schedule` authority 模型决定。
- **Code Boundary Green**：schedule 持久化、终态与 projection 只存在于 Helix
  production handler/projector；PC 只做 Tauri admission、Frame apply、DOM，
  Mobile 只做 SDK admission、FFI batch、SurfaceRuntime atomic apply 与 QuickJS
  typed view。PC 扫描 `2735` 个 production 文件后
  `mockProductionReachable=false`，Mobile 测试注入和 raw reducer 默认不可达。
- **Frame V2 Green**：四份完整 envelope 的 schema/runtime/scope/surfaceEpoch/
  revision/baseRevision/mutation/action/directive/causation 已字段级对齐。
  `channel-summary-scheduled` 与 recovery 是 producer-authored authority；
  duplicate、revision gap、unknown schema/surface 和非法 schedule revision
  均 fail-closed 且不产生部分提交。

## T037 Gate Report

- 结构化汇总见 `evidence/gate-report.json`。G08 required lanes 的 fail=`0`、
  skip=`0`，pre-review verdict=`Green`。
- Mobile 跨 Gate 共享回归的 11 个 real-lane failure 与 SurfaceRuntime 的
  source-environment skip 均保留在 observation，没有被 required lane PASS 掩盖。

## T038 Quickstart And Mandatory Review

- Helix quickstart 最终复跑：`cargo check -p helix-im` exit `0`；
  `g08_full_flow_test` 为 `7/7`；`g08_wdio_fixture_contract_test` 为 `1/1`，
  普通 Golden compare 未改写 canonical Fixture。
- PC 最终产出仍为 Mock `4/4`、single-real `1/1`、dual-real `1/1`、
  report `1/1`；production-boundary 重跑扫描 `2735` files，exit `0`。
- Mobile 最终 parity、source-pin、legacy hard-cut 与三仓 `git diff --check`
  均 exit `0`；G08 专项与真链保持 Green，跨 Gate 11 个 real-lane blocker
  继续单列。
- mandatory after_implement loop 在 event `15` 给出 R4 evidence `PASS`；
  当前进入收敛/关闭流程，只有最终 `CLOSED` 才宣称 G08 完成并解锁下一 Gate。

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
