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
