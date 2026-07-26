# Verification Procedure: G-09

## Source Lock And Prerequisite

| Source | Locked value | Evidence |
|---|---|---|
| Helix admission HEAD | `5308d2b77174e687423c28c862fde42406408456` | `git rev-parse HEAD` |
| cses-client HEAD | `a0a083dd6e65051ab60d6d9bb3e044f4b3984f7a` | `git -C .../cses-client rev-parse HEAD` |
| Mobile HEAD | `3b0ff15feeb95e0d1fe89040d7e9b2ed96a3734f` | `git -C .../mobile-qucik-c++ rev-parse HEAD` |
| PC Helix pin | `5308d2b77174e687423c28c862fde42406408456` | `cses-client/src-tauri/Cargo.toml` |
| Strict Matrix SHA-256 | `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac` | canonical matrix |
| Prerequisite | G08 `phase=closed`, `event_seq=21`, R1-R4 PASS | G08 review ledger |

Mobile retains eight unrelated untracked Helix bundles and `tests/.DS_Store`; G09 must
not stage, delete or rewrite them.

## T001-T002 Source Lock

- T001/T002: source lock and CURRENT/TARGET/GAP are current against production code.
- T003 三端均复用共享 fresh-group prerequisite。Helix 以 G01 shared scenario
  驱动 production create；PC single/dual-real 从真实 MessageV4 UI 创建新群；
  Mobile `run-uc01-16-gtest.sh --only UC09` 通过 `RC.createChannel('schedule')`
  创建新群。三端均未依赖前序 Gate 遗留频道，也未新增第二套建群 reducer。

## T004-T012 Helix Producer Green

| Plane | Command | Result | Boundary |
|---|---|---|---|
| Build | `cargo check -p helix-im` | PASS, exit 0, 6.12s | production crate |
| Full flow | `cargo test -p helix-im --test g09_full_flow_test -- --nocapture` | PASS, 7/7 | prerequisite/happy/business/infrastructure/race/persist/recovery |
| Fixture update | `UPDATE_G09_WDIO_FIXTURES=1 cargo test -p helix-im --test g09_wdio_fixture_contract_test -- --nocapture` | PASS, 1/1 | explicit export only |
| Golden compare | `cargo test -p helix-im --test g09_wdio_fixture_contract_test -- --nocapture` | PASS, 1/1 | ordinary run cannot rewrite Fixture |
| Interaction | `cargo test -p helix-im --test g09_schedule_cancel_interaction_test -- --nocapture` | PASS, 3/3 | field/persist/replay |
| Outbound | `outbound_commands` + `p2_posts_outbound_test` | PASS, 13/13 + 18/18 | registry and Go wire |
| WS | `cargo test -p helix-im --test s1_ws_actions_test -- --nocapture` | PASS, 23/23 | G08/G09 authority barrier |
| Core boundary | `cargo tree -p helix-core` runtime grep | PASS, empty | core stays runtime agnostic |

Production commits:

- `4818572a8bdff3478a36e3fb4f7c6d3c7807b33c`: strict cancel command,
  independent causation, atomic cancel fact/read model, persist barrier and seven scenarios.
- `48424aa`: four canonical Frames, Golden update policy and raw/semantic provenance.
- `686a54e`: old S1 immediate-emit characterization migrated to the G09 durable barrier.

Canonical inventory:

- `group-created.frame-v2.json`
- `channel-summary-scheduled.frame-v2.json`
- `channel-summary-canceled.frame-v2.json`
- `channel-summary-recovery.frame-v2.json`

**Helix verdict**: T004-T012 Green。`5308d2b` 进一步补齐
`message.channel-summary` 的 `cancelSchedule` ActionBinding registry 与
`ActionDispatch::CancelSchedule`，shared scenario 从真实 Frame 提取 binding 后再
驱动既有 cancel HTTP/WS/Persist/readback 链。

## T013-T022 PC Mock/Real/Report

- cses-client 接受提交为 `a0a083dd6e65051ab60d6d9bb3e044f4b3984f7a`；
  Tauri 四个 Helix production dependency 统一 pin `5308d2b...`。
- G09 Fixture parity、typed loader、AcceptedCommand、完整 Frame replay、
  MessageV4 page object、reporter 与 production boundary 已完成。WDIO Jest
  `21/21`、MessageV4 component `27/27`、WDIO TypeScript 与 production
  candidate build 全部 exit `0`。
- Mock run `mock-20260726032808410-97555` 为 `4/4`，覆盖 happy、IPC
  failure、late stale Frame 与 recovery，claim 仅为 `mock`。
- single-real run `single-real-20260726032505692-94731` 为 `1/1`；
  dual-real run `dual-real-20260726032730613-97014` 为 `1/1`；report run
  `report-20260726032924879-98710` 为 `1/1`。真实链从 UI fresh group 开始，
  关联 invoke、Helix、SQLite、create/cancel HTTP、actor WS、FrameApplied 与 DOM。
- production boundary 扫描 `2735` 个生产文件，Fixture/Mock/WDIO API
  production reachable=`false`。

## T023-T032 Mobile Contract

- Mobile 依次提交 `e160483`（production Helix pin）、`0a78c7b`
  （canonical mirror）与 `3b0ff15`（GTest/QuickJS/real chain）。
  production Helix pin 为 `5308d2b...`，bundle SHA-256 为
  `5f58b688cade09829bb885129a0b5d742764aa92aae8f74b64583c5d9c6dc990`。
- G09 mirror parity 为 `4/4`；source pin 证明 fixture producer `4818572...`
  可达 production pin；legacy hard-cut 报告 production V1
  producer/consumer/raw fallback 均为 `0`。
- `G09FrameContractTest.*` 为 `5/5`，覆盖完整 envelope、scheduled→canceled
  revision、ActionInvocation admission、DomainEvent batch、SurfaceRuntime atomic
  apply、C ABI ack、recovery 与 QuickJS `im.surface.view` terminal 字段。
- UC09 非 real focused 合同为 `9/9`。共享生产 runner
  `./scripts/run-uc01-16-gtest.sh --only UC09` 为 `10/10`，runId
  `mrc-mobile-uc-20260726034322-7498`；真实创建 fresh group、create schedule、
  cancel schedule、Frame apply、typed view 与 cleanup 全部完成。
- 本 Gate 未修改 Venus/QuickJS 接线能力，因此按总控合同复用共享 Venus
  回归，不新增专属 Gate。生产入口仍为 `MobileImSdk -> MobileFfiBridge /
  helix-driver-ffi -> DomainEvent batch -> SurfaceRuntime -> QuickJS typed view`。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 三份 provenance SHA-256 均为
  `28ba5d6a3db82f49982ccc2b92b90997bd1c6fc44c60e7f145cd091edbc955d8`。
- inventory 均为 `group-created`、`channel-summary-scheduled`、
  `channel-summary-canceled`、`channel-summary-recovery`，schemaVersion 全为 `2`。
- 四份 raw SHA-256 在三仓逐文件相同：
  `0e496f...`、`ebe678...`、`052bc2...`、`d68487...`。provenance
  byte-exact 一致，因此 semantic SHA-256 与 surface inventory 同样一致；
  平台未补业务字段。

## T034-T036 Review

- **Behavior Green**：fresh group、owner schedule prerequisite、取消权限、HTTP/
  Persist failure、重复/迟到 authority、recovery、Actor/Observer 结果均由 Helix
  同一 schedule model 决定；AcceptedCommand 不被当成取消终态。
- **Code Boundary Green**：取消业务只在 Helix command/action/HTTP/WS/persist/
  projector；PC 与 Mobile 只做生产 admission、完整 Frame atomic apply 与平台渲染。
  Fixture、Mock、测试注入默认 production build 不可达。
- **Frame V2 Green**：四份完整 envelope 的 schema/scope/surfaceEpoch/revision/
  baseRevision/mutation/action/directive/correlation 已字段级对齐。scheduled Frame
  提供 enabled `cancelSchedule`；canceled/recovery Frame 删除该 action 与 schedule
  payload，只保留 `scheduleStatus=canceled`、`scheduleRevision=9`。

## T037-T038 Gate Report And Quickstart

- 结构化汇总见 `evidence/gate-report.json`。required lanes fail=`0`、
  skip=`0`，final verdict=`Green`。
- Helix、PC Mock/Real/Report、Mobile parity/GTest/real runner、production
  boundary 与三仓 hash 对账均 exit `0`；`git diff --check` 三仓通过。
- 首次 Mobile 单独执行 `Uc09ScheduleTest.*` 因缺少 real runner context 正确
  BLOCKED；随后共享 runner 提供 context 并真实通过 `10/10`，未以跳过替代。
- mandatory after_implement 与 convergence 只有在 R4 接受本证据、任务 fingerprint
  不再变化时才允许最终 `CLOSED`。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-09` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
