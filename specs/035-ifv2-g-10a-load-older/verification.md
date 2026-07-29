# Verification Procedure: G-10a

## T001 Source Lock（2026-07-25）

- Helix HEAD：`4f398cc05fcdd9346d5d8163ab6110a72be6b949`；G10a 开始时工作树仅含本 Gate mandatory loop ledger 与 SESSION 预期记录。
- cses-client HEAD：`4105aefaf5c470a77c52a23318edae57cdff3b70`；生产 Tauri Helix pin 为 `e10719adbcf5767c2eab2eb288d86f3e2b210477`。
- Mobile HEAD：`0e9400c5044d19dd47f7b97899ff2a611ca57a92`；生产 Helix pin 为 `f221c924d4da606035ed040c5d4583ccb5a250dd`，既有未跟踪 bundle 与 `tests/.DS_Store` 不属于本 Gate。
- Strict Matrix SHA-256：`b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- 前序 G10：commit `4f398cc`，loop `event_seq=14`、`phase=closed`、R1-R4 全 PASS。
- G10a admission：loop `event_seq=7`、`phase=implementing`、`IMPLEMENTATION_ADMITTED`。后续命令必须显式传 `--feature specs/035-ifv2-g-10a-load-older`。

## T003-T008 Helix Producer

- Shared prerequisite：G10a 复用 `G01ScenarioRunner::create_group_prerequisite`，每个 canonical trace 生成 fresh group Frame；未新增第二套建群 reducer。
- Production chain 已从 raw `im:messages:older_loaded` 改为 `postContext HTTP rounds -> visible message BatchUpsert -> PersistOk -> bounded message Scan -> message.timeline Frame V2`。Helix capabilities 与生产 source 不再暴露 raw older-loaded producer。
- attached window 与 anchor 由 `TimelineProjector` 的 typed state 解析；direct command 不接受 window token。detached/ambiguous anchor 在 HTTP 前 fail-closed。
- HTTP、畸形 authority、Persist 或 readback 失败只发布保留既有 20 rows 的 failed replaceView；success 在 durable readback 后发布 40-row patch；exhaustion recovery 将 `hasOlder/hasMore` 置 false。
- `cargo check -p helix-im`：PASS，退出码 0，6.87s。
- focused suite：G10a full-flow `8/8`、既有 mobile older-context `5/5`、paging properties `13/13`、G10/G10a interaction `5/5`，fail=0、skip=0。
- duplicate persist/readback correlation 均产生 0 Effect；success patch 的 `baseRevision` 精确等于初始 G10 revision。

## T009-T012 Helix Canonical Fixture

- Fixture producer 固定为 `G10AScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode`，source revision 为 `c58930ee99bf4a7042e93a1d9aaa68c3910ad81a`。
- inventory 固定为 6 份：`group-created`、`timeline-loaded`、`timeline-http-failed`、`timeline-persist-failed`、`timeline-race-current`、`timeline-exhausted`；另有同源 `provenance.json`。
- 每份 Frame 均执行 typed `validate/decode`、完整 timeline state/page/items/actions 语义投影、raw SHA-256、semantic SHA-256 与 surface inventory 校验。HTTP/Persist failure 的完整 Frame 相同是领域终态等价，不在平台补造错误原因。
- 更新命令仅为 `UPDATE_G10A_WDIO_FIXTURES=1 cargo test -p helix-im --test g10a_wdio_fixture_contract_test -- --nocapture`；普通运行只比较 actual producer 与 checked-in bytes。
- update run：PASS，`1/1`，退出码 0，5.8s。
- canonical comparison：`cargo test -p helix-im --test g10a_full_flow_test --test g10a_wdio_fixture_contract_test --test g10_timeline_window_interaction_test -- --nocapture`，PASS，分别 `8/8`、`1/1`、`5/5`，退出码 0，5.8s。
- raw hash：group `1fbdf45c...8ac2`、loaded/race `c84ea558...9dfb`、failed `e1dd0048...104b`、exhausted `c9bc7cb4...6d49`；完整 hash 与动态字段白名单以 `fixtures/g10a-wdio/provenance.json` 为机器真源。
- **Helix Green**：T001-T012 全部完成；PC/Mobile 只能镜像以上 bytes，不得解释或补造 G10a 业务字段。

## T013-T022 PC Mock/Real/Report

- cses-client 接受提交为 `f1c4ddc0f`；Tauri 四个 Helix production
  dependency 统一 pin `d2b1bf6c...`，并在 native command admission 为
  `im_load_older_context` 注入外层 `req_id` correlation sidecar。Go wire body
  仍只有 `channel_id`、`anchor_post_id` 与 `before`。
- G10a Fixture parity、typed loader、AcceptedCommand/replay split、MessageV4
  page object、reporter 与 production boundary 均完成。WDIO/Jest 合同为
  `42/42`，production candidate build、WDIO TypeScript 与 Tauri
  `wdio-e2e` transport-correlation test 均 exit `0`。
- Mock 为 `4/4`，覆盖 loaded、HTTP/Persist failed、late duplicate 与
  exhausted，claim 仅为 `mock`。single-real、dual-real、report 分别
  `1/1`、`1/1`、`1/1`；均由真实 MessageV4 UI fresh group 开始，经
  Tauri invoke、Helix、SQLite、`posts/postContext` HTTP、FrameApplied 与
  DOM 自动等待闭合。
- production boundary 扫描 `2735` 个生产文件，Fixture、Mock bridge 和
  WDIO-only API production reachable=`false`。嵌入式 runner 仍打印非致命
  `tauri-driver not found` 诊断，但真实 WebKit/Tauri session 已启动并通过，
  该诊断未被拿来替代真实链。

## T023-T032 Mobile Contract

- Mobile 依次提交 `a3b2c8b`（production Helix pin）、`cfa8a4c`
  （canonical mirror）与 `26c7616`（GTest/QuickJS/real chain）。
  production Helix pin 为 `7e93be3...`，bundle SHA-256 为
  `a8753e67e4e1a6b04cd8b70670ace8e463fec91f475f480c41149951d3ef8533`。
- G10a mirror parity 为 `6/6`；source pin 证明 fixture producer
  `d2b1bf6c...` 可达 production pin；legacy hard-cut 报告 production V1
  producer/consumer/raw fallback 均为 `0`。
- `G10AFrameContractTest.*` 为 `6/6`，覆盖完整 envelope、G10
  revision-one prerequisite、loaded/HTTP failed/Persist failed/race/exhausted、
  DomainEvent batch、SurfaceRuntime atomic apply、C ABI ack 与 QuickJS
  `im.surface.view` typed state。
- UC10 非 real focused 合同为 `8/8`。共享生产 runner
  `./scripts/run-uc01-16-gtest.sh --only UC10` 为 `9/9`，runId
  `mrc-mobile-uc-20260726062234-24003`；真实创建 fresh group、发送 42 条、
  首屏 20 条、上拉后 40 条、Frame apply、typed view 与 cleanup 全部完成。
- `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch ->
  SurfaceRuntime -> QuickJS typed view` 的生产接线已存在并由本 Gate
  production-linked GTest 证明，因此 T027-T030 未复制或修改业务逻辑。
  本 Gate 未改变 Venus/QuickJS 接线能力，按总控合同复用共享
  `run-venus-helix-im-smoke.sh`，结果 PASS。
- 全量 legacy-characterization binary 为 `386 PASS / 2 SKIP / 12 FAIL`：
  12 个失败均是该 binary 显式打开
  `MOBILE_ENABLE_LEGACY_V1_CHARACTERIZATION` 后，既有 typed-view 测试仍断言
  `im.store` 不可达的测试配置冲突；生产态 `surface-runtime` lane 和 G10a
  专项均通过。该残余不作为 G10a production consumer 的替代证据。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 三份 provenance SHA-256 均为
  `f450643d7345f98bf1cbd0f6320998c1cfe2351e3cf9c46ad2607bb9b1dddeba`。
- inventory 均为 `group-created`、`timeline-loaded`、
  `timeline-http-failed`、`timeline-persist-failed`、
  `timeline-race-current`、`timeline-exhausted`，schemaVersion 全为 `2`。
- 六份 raw SHA-256 在三仓逐文件相同：group
  `1fbdf45c...8ac2`、loaded/race `c84ea558...9dfb`、
  HTTP/Persist failed `e1dd0048...104b`、exhausted
  `c9bc7cb4...6d49`。provenance byte-exact 一致，因此 semantic SHA-256
  与 surface inventory 同样一致；平台未补业务字段。

## T034-T036 Review

- **Behavior Green**：fresh group、exclusive anchor、before 边界、HTTP/Persist
  barrier、失败保留当前 20 行、late duplicate、40 行 merge 与 exhausted
  分页终态均由 Helix 同一 older-context model 决定；AcceptedCommand 和
  HTTP success 均未被当作 Frame 终态。
- **Code Boundary Green**：业务 reducer 只在 Helix
  `older_context/request/effects/projector`；PC 与 Mobile 只做 production
  command admission、完整 Frame atomic apply 与平台渲染。Fixture、Mock、
  test injection 默认 production build 不可达。
- **Frame V2 Green**：六份完整 envelope 的 schema/scope/surfaceEpoch/
  revision/baseRevision/mutation/action/directive/correlation 已字段级对齐。
  G10 首屏 revision `1` 是共享 prerequisite；G10a loaded/failure/race 为
  `baseRevision=1, revision=2`，exhausted 为
  `baseRevision=2, revision=3`，三端均未自行合并 message row。

## T037-T038 Gate Report And Quickstart

- 结构化汇总见 `evidence/gate-report.json`。required lanes fail=`0`、
  skip=`0`，final verdict=`Green`。
- Helix、PC Mock/Real/Report、Mobile parity/GTest/real runner、production
  boundary、共享 Venus 回归与三仓 hash 对账均完成；三仓
  `git diff --check` 通过。
- mandatory after_implement 与 convergence 只有在 R4 接受本证据、任务
  fingerprint 不再变化时才允许最终 `CLOSED`。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-10a` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
