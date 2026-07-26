# Verification Procedure: G-09

## Source Lock And Prerequisite

| Source | Locked value | Evidence |
|---|---|---|
| Helix admission HEAD | `1a68e0eac9bef49660117921b277d67617e45217` | `git rev-parse HEAD` |
| cses-client HEAD | `033131ca006f0103beea778a91121ba7b9b887c3` | `git -C .../cses-client rev-parse HEAD` |
| Mobile HEAD | `e67c5c5779e2bf381d64db6e235b3abe884a8cb9` | `git -C .../mobile-qucik-c++ rev-parse HEAD` |
| PC Helix pin | `8358082b3fd01b96b332c79deca8d2017315dd19` | `cses-client/src-tauri/Cargo.toml` |
| Strict Matrix SHA-256 | `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac` | canonical matrix |
| Prerequisite | G08 `phase=closed`, `event_seq=21`, R1-R4 PASS | G08 review ledger |

Mobile retains eight unrelated untracked Helix bundles and `tests/.DS_Store`; G09 must
not stage, delete or rewrite them.

## T001-T002 Source Lock

- T001/T002: source lock and CURRENT/TARGET/GAP are current against production code.
- Helix side of T003 is proven: `G09ScenarioRunner` reuses the shared G01 fresh-group
  producer and drives the real G08 createSchedule authority/persist/readback chain before
  cancel. T003 stays open until PC and Mobile prove the same prerequisite without a second
  group or schedule reducer.

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

**Helix verdict**: T004-T012 Green. G09 remains `implementing`; PC/Mobile/Review evidence
is not yet available.

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
