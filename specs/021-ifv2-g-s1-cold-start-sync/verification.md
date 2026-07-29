# Verification Procedure: G-S1

## Source Lock

- Helix worktree 起点：`3ddce5e9b69eb52355d631bae41ea02e09074ea8`；
  前序 G16c 已由 Spec Kit event 24 `CLOSED`，R1-R4 PASS。
- cses-client worktree：
  `d0d06d87f406646d865cbce07434d5080558a2ae`；Mobile worktree：
  `a3580948fa1f115d3e19ac38d27ac624d601d481`；Go worktree：
  `f2e919c6ea95a898d25827a2a82ea991a3ad0bf3`。
- GS1 producer 提交为
  `79cc5cb14ee669591a204f7a9e7e2b2f72b14975`；当前 Helix source pin
  `33b6cf2d5621f6fc417a1229111b58bd6854d713` 仅追加 Fixture provenance
  允许的 runtime identity scope，PC `src-tauri/Cargo.toml` 与 Mobile
  `contract/helix-source.lock.json` 均已重新 pin 到该提交。
- Strict Matrix SHA-256：
  `caadafd2947ccadf647a134ab7c85f60b1d0feb6966f79a8717b32eef094c9c8`。
- GS1 只处理 hello -> increment batch -> durable dialog projection；GS2 notify、
  GS3 pong gap 与 GS4 unknown-channel hydration 保持后续独立 Gate。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-S1` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Implementation Evidence 2026-07-26

- **V-001/V-002/V-003 PASS**：`cargo check -p helix-im` 退出码 0；
  `gs1_full_flow_test` 6/6、`gs1_wdio_fixture_contract_test` 1/1、
  `gs_persist_barrier_test` 9/9。三份 canonical Frame 均由
  `Gs1ScenarioRunner -> Effect::Emit` 导出。
- **V-004 PASS**：PC fixture parity、Mock bridge、reporter 共 35/35，
  WDIO TypeScript `tsc --noEmit` 通过，production boundary 扫描 2735 文件通过。
  通过真实 macOS bundle 恢复 Tauri `resource_dir` 后，`gs1-mock` 原生 lane
  3/3、5.6s、退出码 0；happy/failure/race 均由 canonical Frame replay，
  报告 claim=`mock`。`gs1-report` contract lane 1/1、退出码 0。
- **V-005 BLOCKED**：`gs1-single-real` 已进入真实 MessageV4 UI，但 fresh-group
  前置在成员 `678` 的组织目录 selector 持续 loading 60s，未产生
  `im:channel:created`。因此 single-real 未闭合且 dual-real 未启动；真实
  invoke、SQLite、HTTP、WS、FrameApplied、DOM 证据不得记为 Green。
  2026-07-27 00:59 PDT 再次运行仍在同一 selector 等待 60s 后失败，
  artifact 为
  `artifacts/message-wdio/single-real-20260727075936585-79883/`，确认前置尚未恢复。
- **V-006 PARTIAL**：Mobile byte parity PASS；`GS1FrameContractTest` 4/4，
  `Gs01HelloSyncTest` 非 RealChain 6/6，legacy hard-cut PASS，
  source-pin 在 `33b6cf2d` PASS。共享 `run-uc01-16-gtest.sh` 退出码 1，
  251.676s，135/151 PASS、16 FAIL；GS1-GS4 与 UC01/02/05-14 的
  RealChain 均在 fresh-group 前置等待 `im:channel:created` 超时。
  UC15/UC16 RealChain 分别在 0.588s/1.391s 通过，说明生产链并非全局失效，
  但不能替代 G-S1 自身 fresh-group 证据。
- **V-007 BLOCKED**：loop event 24 因 convergence 未追加新任务而给出名义
  `CLOSED`，但 `tasks.md` 仍有 T019/T020/T034-T038 七项未完成，且
  V-005/V-006 有真实失败。该 verdict 与证据合同冲突，不采纳为
  review-green；G-S1 仍为 `blocked`，G-S2 不解锁，也不以修改 ledger
  或补写假证据制造 Green。
- **Cross-repo Fixture parity PASS**：Helix、PC 与 Mobile 三端 inventory 均为
  `dialog-empty`、`dialog-loaded`、`dialog-recovery`，schema/raw/semantic
  SHA-256 由 PC 35/35 parity suite 与 Mobile
  `check-gs1-fixture-parity.mjs` 双向校验通过；producer 固定为 `79cc5cb1`。

## Cold-start Closure 2026-07-27

本节以完成物理删库后的新运行覆盖上节 V-005/V-006/V-007 的历史阻塞结论。
测试只删除各 persona 的 Helix SQLite 主库、WAL、SHM 与 journal；未调用远端
删除接口，证据中的 `remoteDeletionPerformed` 均为 `false`。

- **V-005 PC Real PASS**：single run
  `gs1-cold-single-20260727030223` 与 dual run
  `gs1-cold-dual-20260727032800` 均为 seed -> app exit -> guarded reset ->
  same-profile recover。dual run 的 Actor 444 与 Observer 678 都在原
  `tenant.db?mode=rwc` 路径重建数据库，删除主库/WAL/SHM 共 6 个文件，
  residual=0；恢复后的 channel `tjhu7z453prz8jfytjrw1szayo` 在两端均得到
  remote/local SHA-256
  `ac84fafc50b44658e6141ce7d54cb1c19160f5ce91dbc188add977bd2af39622`，
  missing=0、extra=0、mismatch=0。dual seed 约 17.2s，recover 24.888s，
  WDIO exit=0，Actor/Observer dialog、FrameApplied 与 DOM 最终一致。
- **V-006 Mobile Real PASS**：run
  `mrc-mobile-uc-20260727104758-27652` 的 GS01 8/8 PASS，16.367s。
  userA/userB 均先 dispose production runtime，再分别删除同路径 SQLite
  主库/WAL/SHM 共 3 个文件，residual=0，并以原路径重启。
  channel `6ebp7ypfrigozjrqsq479aqoxw` 的两端 remote/local SHA-256 均为
  `4c4eaef40c36043de841ab9722b7d89ee7ee9eba44baeae336530bee191ad87d`，
  missing=0、extra=0、mismatch=0；typed dialog/timeline Frame 与 QuickJS
  typed view 均完成冷启动恢复。
- PC 调试期间暴露的 `14202` WebDriver 端口不在 Tauri remote capability
  中；恢复为已许可的 `14201` 后通过。Mobile 曾遇到一次瞬态 userB WS
  connect timeout，独立重跑后通过。两者均未改变业务、端口或依赖架构，
  因而不创建架构票。

## Cross-repo Reviews

- **T034 Behavior Green**：fresh group 权限与成员角色以远端 PostgreSQL 为
  authority；terminal cursor 与 event cursor 一并规范化对账。duplicate/late
  reply、persist barrier 与 recovery 已由 Helix focused tests 覆盖；PC
  Actor/Observer 和 Mobile userA/userB 的最终 channel/member/cursor、dialog
  与 timeline 结果一致。任何 reset 残留、路径漂移、远端/本地差异都会
  fail-closed。
- **T035 Code Boundary Green**：同步 reducer、cursor 屏障与
  `InteractionFrameV2` 生产仍只位于 Helix。PC 的 reset/orchestration/parity
  位于 `scripts/` 与 `tests/e2e/`，Mobile reset/parity 位于 GTest 与
  `scripts/real-chain/`；production-boundary 扫描通过。Go 新增
  `server-db-evidence gs1-parity` 只执行只读 PostgreSQL 快照，不参与业务写入。
- **T036 Frame V2 Green**：schema、scope、epoch、revision、mutation、
  action、directive 与 correlation 由 canonical Helix Fixture 和
  `GS1FrameContractTest` 校验；PC 以 FrameApplied 后的 DOM 收敛判定，
  Mobile 以原子 apply 后的 typed dialog/timeline view 判定。未知
  schema/surface/revision fail-closed，不存在 raw business reducer fallback。
- **T037 Evidence Green**：四 persona 的 DB 路径、删除文件、零残留、
  remote/local hash、完整 diff 与 typed projection 已汇总到
  `evidence/gate-report.json`。详细运行摘录见
  `evidence/2026-07-27-cold-start-closure.md`。
