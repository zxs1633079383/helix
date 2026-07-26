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

### T005 G04 Three-repository Review Green

- 子 Feature：`specs/027-ifv2-g-04-online-revoke`，全部 T001-T038 完成，
  helix-loop R4 `PASS` 并最终 `CLOSED`。
- Source pins：Helix
  `96ff88657e10e0247f34dd92317525b11fedfb0d`；Fixture producer
  `f323cd7a997fba0cb330299d98eb0b8bd73d4813`；cses-client
  `c91cafc8359495daff2969ca4ed0db729027829f`；Mobile
  `094a15149f448b7ce952b71688e5bae9fe278769`，production Helix pin
  `ee9b8f4492fd82aa846141be6f01f5729fc64bf8`。
- Helix：build/zero-runtime PASS，G04 deterministic scenario 6/6，
  authority/persist interaction 4/4，canonical Fixture contract 1/1。
- Fixture：四份 schemaVersion=2 producer Frame 与 provenance 在
  Helix/PC/Mobile byte-exact、semantic hash 和 Surface inventory 100% 一致；
  provenance SHA-256 为
  `1792edb6550df9631b383fe7e02fd6782ee753809eab5642d0125fcef2b80267`。
- PC：Mock 2/2、report 1/1、single-real 1/1、dual-real 1/1；fresh group、
  opaque action、Tauri/Helix/SQLite/HTTP/WS/FrameApplied/Actor/Observer DOM
  真实链闭合。
- Mobile：G04 production-linked 5/5、UC05 non-real 7/7；SDK/FFI batch/
  SurfaceRuntime/opaque action/QuickJS typed view/C ABI、source pin、V1 hard-cut
  与共享 Venus 回归全部 PASS。
- Review：behavior、code boundary、Frame V2、evidence 全 Green；结构化报告
  SHA-256 为
  `dcf8bf211e59c93bfc0ac3c089c57a6efa9c2956e47f506f8b27e2d82bf84e35`。
- 非阻塞观察：Mobile real harness 在既有 GoLand 8066 进程上于撤回前的
  remote readback 超时；它不是 required Mobile Contract lane，未为此恢复 V1、
  raw reducer 或测试注入。

结论：G-04 `review-green`，总控顺序解锁 G-04a；G-04a 必须再次创建 fresh group，
不得复用本 Gate 的 post、channel、surface revision 或本地缓存。

### T006 G04a Three-repository Review Green

- 子 Feature：`specs/028-ifv2-g-04a-offline-revoke`，全部 T001-T038 完成，
  helix-loop R4 `PASS` 并最终 `CLOSED`。
- Source pins：Helix
  `bde8d79fba6335f9d6d8ed3feafab05a80fcd151`；Fixture producer
  `c9c37ea8fcb398715cff872d4adbdc8a0e412c46`；cses-client
  `f0cc3d187f4d84165a485d21abe91175264519f4`；Mobile
  `883fca7f744feb6d38c48ff6b607fe7818c42bf3`，production Helix pin
  `bde8d79fba6335f9d6d8ed3feafab05a80fcd151`。
- Helix：build/zero-runtime PASS，G04a deterministic scenario 7/7，
  authority/persist interaction 5/5，canonical Fixture contract 1/1。
- Fixture：四份 schemaVersion=2 producer Frame 与 provenance 在
  Helix/PC/Mobile byte-exact、semantic hash 和 Surface inventory 100% 一致；
  provenance SHA-256 为
  `ac7ef7c8c301da9e7ee76b1815038e75f4313d549fcc2330836b75074f5b435e`。
- PC：Mock 3/3、report 1/1、single-real 1/1、dual-real 1/1；fresh group、
  真实 explicit sync、Tauri/Helix/SQLite/HTTP/FrameApplied/Actor/Observer DOM
  链闭合。
- Mobile：G04a production-linked 5/5、UC05 focused 9/9；SDK/FFI batch/
  SurfaceRuntime/QuickJS typed view/C ABI、source pin 与 V1 hard-cut 全部 PASS。
- Review：behavior、code boundary、Frame V2、evidence 全 Green；结构化报告
  SHA-256 为
  `6d748549638002ce6be77289cd1d734d02c97eec780b5bc10bb6f2f2dd2aedcc`。
- 非阻塞观察：共享 Mobile 150-test runner 中 G04a 再次 PASS，但 14 条尚未进入
  纵向闭环的后续 Gate real-chain 失败；失败清单已保留到 G04a V-006，未恢复
  V1/raw reducer 或将其伪造成全量 Green。

结论：G-04a `review-green`，总控顺序解锁 G-05；G-05 必须创建 fresh group 和
独立 source lock，不得复用本 Gate 的 channel、post、cursor 或 surface revision。

### T007-T014 G05 Through G11 Three-repository Review Green

- 子 Feature `029` 至 `036` 的任务均为 `38/38`，review ledger 均为
  `phase=closed`、`next_action=none`；顺序覆盖 G-05、G-06、G-07、G-08、
  G-09、G-10、G-10a 与 G-11。
- 每个 Gate 的 Helix canonical Fixture 均由生产
  `ImModule::handle -> Effect::Emit` 生成；PC 与 Mobile 只同步 canonical
  bytes，并完成各自的 raw/semantic/provenance parity。
- 每个 Gate 均完成 PC Mock、single-real、dual-real、report 与 production
  boundary，以及 Mobile production-linked GTest、SDK/Bridge、
  SurfaceRuntime、QuickJS typed view、V1 hard-cut 和适用的共享 Venus 回归。
- 各子 Feature 的 `evidence/gate-report.json` 最终结论均为 `Green`，行为、
  代码边界、Frame V2 与 evidence Review 均已通过 mandatory R4。
- G-11 source lock：Helix acceptance
  `60b2d5f579f209a144433db1f96bef8a63f6bd5c`，Fixture producer
  `5e3ffc026fc8b25f76a7cce162e75734bddb45ca`，cses-client
  `509d10cae61e66f6d0a90461daa8d1ad192ba1b8`，Mobile
  `49b9430`；三仓 provenance SHA-256 均为
  `420c6a7b749b407a6debfbf077fc9843ce997a3897940dd51d66a2ae6178b3fd`。
- G-11 required lanes fail=`0`、skip=`0`。共享 Mobile 全量 runner 的
  `141 PASS / 9 FAIL` 中，G11 自身 `7/7 PASS`；其余失败归属后续 Gate
  RealChain，继续由各 owning Gate 收敛，不将局部绿冒充全量绿。

结论：G-05 至 G-11 均为 `review-green`；总控 T007-T014 完成并按顺序解锁
G-12。G-12 必须创建 fresh group/thread，不复用 G-11 的 channel、post 或
surface revision。

### T015 G12 Three-repository Review Green

- 子 Feature：`specs/037-ifv2-g-12-reply-thread`，T001-T038 全部完成，
  helix-loop R1-R4 `PASS` 并最终 `CLOSED`。
- Source pins：Helix `1b797413f2c1b14abde5eacf60eb942b21e9d650`；
  Fixture producer `a2207d73f8e72da7374b916b2167ee3a25b9f6c3`；
  cses-client `6174658be34a779ded68311848cd5c4851ec9d01`；Mobile
  `4281946`。
- Helix：full flow 9/9、interaction 3/3、Fixture contract 1/1。
- Fixture：三份 schemaVersion=2 producer Frame 在三仓 byte-exact；
  provenance SHA-256
  `c1bc03de83f24e4375ee1006244596a35e305137e679d993452e52f54c922eb1`。
- PC：Mock 3/3、single-real 1/1、dual-real 1/1、report 1/1；
  production boundary PASS。
- Mobile：Frame 5/5、focused contract 8/8、共享 Venus PASS；SDK/Bridge/
  SurfaceRuntime/C ABI/QuickJS typed view 与 V1 hard-cut 全部 PASS。
- Review：behavior、code boundary、Frame V2、evidence 均 Green，required
  fail=0、skip=0；结构化报告位于
  `specs/037-ifv2-g-12-reply-thread/evidence/gate-report.json`。

结论：G-12 `review-green`，总控顺序解锁 G-13。

### T016 G13 Three-repository Review Green

- 子 Feature：`specs/038-ifv2-g-13-channel-read`，T001-T038 全部完成，
  helix-loop R1-R4 `PASS` 并最终 `CLOSED`。
- Source pins：Helix `c8768f8b6f169f67509c0ec717ef6902c7779e19`；
  Fixture producer `a13e17151efd1e7eefc94677110598b6347b08a9`；
  cses-client `e67384f9a1c78bb2da3aafe4ef4206563d231aa4`；Mobile
  `fe0fa86a3bc27bf3ba416c3c7f07c89b6c80ecbb`。
- Helix：scenario/interaction 12/12、Fixture contract 1/1，HTTP admission
  与 viewer `update_channel` authority 分离，PersistOk 后才发布 paired Frame。
- Fixture：四份 schemaVersion=2 producer Frame 与 provenance 在三仓
  byte-exact；provenance SHA-256
  `aea3480b12bbf40e6063a097e7d91eeffdcaa380919ddf7c964d61b34cc6acb6`。
- PC：Mock 3/3、single-real 1/1、dual-real 1/1、report 1/1；
  fresh-group、Tauri invoke、FrameApplied 与 Actor/Observer DOM 闭合。
- Mobile：Frame 6/6、UC13 8/8、真实 fresh-group read chain、QuickJS typed
  dialog/unread view、C ABI、source pin、Strict Matrix 27/27 与 V1 hard-cut
  全部 PASS。
- Review：behavior、code boundary、Frame V2、evidence 均 Green，required
  fail=0、skip=0；结构化报告位于
  `specs/038-ifv2-g-13-channel-read/evidence/gate-report.json`。

结论：G-13 `review-green`，总控顺序解锁 G-14；G-14 只负责
`im_post_read`/participant receipts，不得复用 G13 的 channel、post 或
surface revision。
