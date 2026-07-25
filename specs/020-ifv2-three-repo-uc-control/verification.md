# Verification Procedure: InteractionFrame V2 26 Gate 总控

## Specification Runs

- **V-001**: `node --check scripts/generate_ifv2_three_repo_specs.mjs`；期望退出码 0。
- **V-002**: `node scripts/generate_ifv2_three_repo_specs.mjs`；期望 activeGates=26、childSpecs=25、gapGates=20，重复运行不改变语义内容。
- **V-003**: `python3 scripts/verify-interaction-frame-v2-index.py`；期望文档库存、机器 Gate 与全部本地链接通过。
- **V-004**: 对总控和 25 个子 Feature 执行 mandatory `loop_engine.py advance`；期望 26/26 `r1_spec.status=pass` 且 next_action 为 `run_speckit_plan`。
- **V-005**: `git diff --check`；期望无 whitespace error。

## Delivery Runs

- **V-101**: 按机器 manifest 顺序逐 Gate 执行各自 verification.md；前序未 `review-green` 时后序不得运行。
- **V-102**: 汇总 26 Gate 的 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 与 Review；期望 required pass=100%、fail=0、skip=0。
- **V-103**: 26 Gate 全绿后生成 legacy/V1 删除候选清单；用户未选择前不得执行删除。
- **V-104**: 用户选择删除后重新运行三仓全量验收；期望新候选仍为 26/26 Green，删除前报告不可复用。

## Evidence Rules

- 规格检查只证明 Spec 可进入 plan，不证明任何业务 Gate 已实现或 Green。
- 每个 delivery run 必须引用原始 artifact、source pin、fixture hash 和结构化 report；聚合结论不得遮蔽 blocked/partial。

## Program Setup Revalidation - 2026-07-25

### T001 Manifest And Index

- Helix HEAD：`94754ef79f79d375b732d4854df9ef3a85f7ef90`。
- `node scripts/verify_ifv2_three_repo_plans_tasks.mjs`：PASS；26 个
  Feature 的 R1-R3、25 x 38 子任务、32 个总控任务、合同、索引、链接与 JSON
  全部有效。
- `python3 scripts/verify-interaction-frame-v2-index.py`：PASS；UC 文档库存、
  25 Gate 与历史任务映射有效。
- 首次运行暴露校验器只接受实现前 `ready/admit_implementation`，却拒绝
  mandatory pre-hook 合法进入的 `implementing/implement_tasks`。校验器已收紧为
  只接受这两个受控状态；没有放宽 R1-R3、数量、顺序或链接检查。

### T002 G01 Current Three-repository Baseline

- Source lock：
  - Helix：`94754ef79f79d375b732d4854df9ef3a85f7ef90`。
  - cses-client：`13e42ef2793ea5c9f39b29f5d1ce86b1aca074db`，运行前后 clean。
  - Mobile QuickJS：`674c8fb639c5b945cbbe4a3a9431ba3beeb0cc21`；运行前已有
    Mobile 文档、bundle 与 `.DS_Store` 脏项，本轮未覆盖或提交。
- Helix：
  - `cargo check -p helix-im`：PASS。
  - `cargo test -p helix-im --test g01_full_flow_test -- --nocapture`：
    4/4 PASS。
  - `cargo test -p helix-im --test g01_wdio_fixture_contract_test
    -- --nocapture`：1/1 PASS；六份 Fixture producer/provenance 与 Golden
    一致。
- cses-client：`yarn message:wdio:all` PASS，耗时 `248.40s`：
  contracts 83/83、safe entry 1/1、UI 1/1、Mock 4/4、integration 2/2、
  single-real 1/1、dual-real 2/2、report 1/1；最终 artifact scan
  `report-20260725075841077-15142` 为 5 files、redaction pass。
  integration/performance 中非 required 的无样本 performance case 保持 skip，
  不提升为性能证据；embedded provider 的 `tauri-driver not found` 诊断未阻断
  真实 Native sessions。
- Mobile：
  - `node scripts/sync-g01-helix-fixtures.mjs --check`：6/6 parity PASS，
    Fixture producer commit
    `ee8440e7b6e19d75153f46349e57fbb6c00ca8cf`。
  - `make surface-runtime-source-pin-gate`：PASS；Mobile Rust pin
    `8ac3d2cf693d6012101444bfae5f2e89f707ebe9`。
  - `make surface-runtime-hard-cut-gate`：PASS；生产 V1 producer=0、
    consumer=0。
  - `./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter
    'G01FrameContractTest.*'`：5/5 PASS。
  - `./scripts/run-venus-g01-frame-gate.sh`：`VENUS_G01_FRAME_V2_OK`。

结论：当前 source lock 下 G01 的 Helix producer、PC Mock/Real/report 和 Mobile
Contract/QuickJS 重新 Green，满足 G02 前序准入。该结论不新增真手机双账号或受控真实
网络失败声明。

### T003 G02 Three-repository Review Green

- 子 Feature：`specs/025-ifv2-g-02-document-message`，全部 T001-T038 完成，
  helix-loop R4 `PASS` 并最终 `CLOSED`。
- Source pins：Helix
  `1774d5a38e694b4a56e2b85ea9055c3e825f2222`；Fixture producer
  `ac15abd0b29ef62ea5d359fb22030800dd0dee2b`；cses-client
  `39695bbd00d978d619505a79560de8af2e89631e`；Mobile
  `6730b5be8d11e334ef632a986173598f18ec9e9a`。
- Helix：build/zero-runtime PASS，G02 deterministic scenario 6/6，canonical
  Fixture contract 1/1。
- Fixture：六份 schemaVersion=2 producer Frame 在 Helix/PC/Mobile 的
  inventory、raw/semantic SHA-256 与 Surface inventory 100% 一致。
- PC：Mock 3/3、report 1/1、single-real 1/1、dual-real 1/1；真实
  Tauri/Helix/SQLite/HTTP/WS/FrameApplied/DOM 链闭合。
- Mobile：production-linked GTest 6/6，SDK/FFI batch/SurfaceRuntime/QuickJS typed
  view/C ABI、source pin 与 V1 hard-cut 全部 PASS。
- Review：behavior、code boundary、Frame V2、evidence 全 Green；结构化报告见
  `specs/025-ifv2-g-02-document-message/evidence/gate-report.json`。
- 非阻塞观察：旧 `run-uc01-16-gtest.sh --only UC02` real harness 仍等待已经退出
  production contract 的 raw V1 query；没有为其新增 Mobile reducer。

结论：G-02 `review-green`，总控顺序解锁 G-03a；后续 Gate 不得复用 G02 运行遗留群，
仍须通过共享 CreateGroupPrerequisite 创建新群。

### T004 G03a Three-repository Review Green

- 子 Feature：`specs/026-ifv2-g-03a-retry-failed-send`，全部 T001-T038 完成，
  helix-loop R4 `PASS` 并最终 `CLOSED`。
- Source pins：Helix
  `af3fee1e2e9735d9cd9574a525e05b9d36333d58`；Fixture producer
  `441c01209fa0995bdbc10081601acf6972827fbf`；cses-client
  `08c1843b7b7751029b6fb9b40ec54eb122f646d0`；Mobile
  `7bc78ef0b2510232a182b9999413bfe1a1bebe35`。
- Helix：build/zero-runtime PASS，G03a deterministic scenario 6/6，canonical
  Fixture contract 1/1。
- Fixture：八份 schemaVersion=2 producer Frame 在 Helix/PC/Mobile 的
  inventory、raw/semantic SHA-256 与 Surface inventory 100% 一致；
  provenance SHA-256 为
  `8d0d478eaf9158a8db7e19bb413871ea9def4bca103f51e284fa239c5c7be4e7`。
- PC：Mock 2/2、report 1/1、single-real 1/1、dual-real 1/1；真实
  Tauri/Helix/SQLite/HTTP/WS/FrameApplied/Actor/Observer DOM 链闭合。
- Mobile：production-linked G03a 7/7、UC04 retry 7/7，SDK/FFI
  batch/SurfaceRuntime/QuickJS typed view/C ABI、source pin 与 V1 hard-cut
  全部 PASS。
- Review：behavior、code boundary、Frame V2、evidence 全 Green；结构化报告
  SHA-256 为
  `71e17b4c6b519df580a37612ad14e147aa3e66a37f2c98e486b53e6bcdc2090c`。
- 非阻塞观察：Mobile 共享 legacy characterization 中尚未迁移 Gate 的 16 个
  RealChain 仍等待 raw event/query；没有为其恢复 `im.store`、raw reducer 或
  V1 consumer。

结论：G-03a `review-green`，总控顺序解锁 G-04；G-04 仍须创建 fresh group，
不得依赖本 Gate 的失败消息、channel 或运行缓存。
