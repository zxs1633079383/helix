# Verification Procedure: G-10

## T001 Source Lock（2026-07-25）

- Helix HEAD：`48e8d74df519d343d99b6797b6d032644823876f`；G10 开始时仅 mandatory loop 更新本 Feature ledger。
- cses-client HEAD：`a0a083dd6e65051ab60d6d9bb3e044f4b3984f7a`；生产 Tauri 依赖 pin 为 `5308d2b77174e687423c28c862fde42406408456`。
- Mobile HEAD：`3b0ff15feeb95e0d1fe89040d7e9b2ed96a3734f`；既有未跟踪 Helix bundle 与 `tests/.DS_Store` 不属于本 Gate。
- Strict Matrix SHA-256：`b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- 前序 G09：loop `event_seq=19`、`phase=closed`、R1-R4 全 PASS；gate report SHA-256 为 `92dd695adc64a3982b008af5a829eccf18afc7d77baef151365ec71c964b6430`。
- G10 admission：loop `event_seq=7`、`phase=implementing`、`IMPLEMENTATION_ADMITTED`。全局 SpecKit current-feature 仍指向 G09，因此 G10 全程必须显式传 `--feature specs/034-ifv2-g-10-timeline-window`，不得让默认脚本误写 G09。

## T003-T008 Helix Producer Revalidation

- Shared prerequisite：G10 复用 `G01ScenarioRunner::create_group_prerequisite` 的生产 `im_create_channel -> HTTP authority -> PersistFire(channel/member) -> message.left-dialog-list Frame`，未新增第二套建群 reducer。
- Production handler：`local_first.rs` 现有实现已满足 local scan、conditional remote read、cache persist barrier、query generation/epoch、complete snapshot/patch 与 failed Frame；本阶段未为测试修改生产 handler/projector。
- `cargo check -p helix-im`：PASS，退出码 0。
- 首轮 focused test：7/8 PASS；唯一失败揭示 cache persist failure 不建立 coverage，因此只读 remote Frame 的 `hasOlder=false`，不是完整 durable coverage。测试合同修正为明确区分 durable cache success 与 cache failure。
- 批量修正后 `cargo test -p helix-im --test g10_full_flow_test --test g10_timeline_window_interaction_test -- --nocapture`：PASS；full-flow 8/8、interaction 3/3、fail=0、skip=0。
- 六类场景：prerequisite、happy、business invalid input、HTTP failure、cache persist failure、late/duplicate race、local coverage recovery 均由公开 `ImModule::handle` 或公开 command parser 验证。

## T009-T012 Helix Canonical Fixture

- Producer revision：`e10719adbcf5767c2eab2eb288d86f3e2b210477`；producer chain 固定为 `G10ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode`。
- 显式执行 `UPDATE_G10_WDIO_FIXTURES=1 cargo test -p helix-im --test g10_wdio_fixture_contract_test -- --nocapture`：1/1 PASS。该次运行先生成 checked-in bytes，随后在同一测试中重新读取并逐字比较、typed decode 和 provenance 对账。
- inventory=6：fresh group、ready、failed、cache-degraded、race-current、recovery；每份都是完整 Frame，未把竞态阶段拼成固定帧序。
- raw SHA-256：group `f8826105...`、ready `aebfa041...`、failed `9d45acbd...`、cache-degraded `93589ced...`、race-current `a24b676f...`、recovery `667f3c32...`；最终 provenance raw SHA-256 `4d35068679462d7ee830e1511270e95c031992874d725edfff15863215ea0469`。
- surface inventory：`message.left-dialog-list` 1 份，`message.timeline` 5 份；schemaVersion 全部为 2。
- Helix Green：build、完整场景、字段合同、Golden/update policy 与 provenance 全 PASS，required fail=0、skip=0。

## T013-T022 PC Mock/Real/Report

- cses-client 提交为 `4105aefaf5c470a77c52a23318edae57cdff3b70`；生产 Tauri Helix pin 为 `e10719adbcf5767c2eab2eb288d86f3e2b210477`。
- G10 canonical mirror、typed loader、AcceptedCommand 与 Frame replay 分离、MessageV4 PageObject、Mock/Real reporter 和 production boundary 均已完成。WDIO Jest 为 `3 suites / 22 tests`，TypeScript、production candidate build 与边界检查均 PASS。
- Mock run `mock-20260726042101303-33737` 为 `4/4`，覆盖 ready、failed、cache-degraded 与 race/recovery，claim 仅为 `mock`。
- single-real run `single-real-20260726043407694-47657` 为 `1/1`；dual-real run `dual-real-20260726043758057-51242` 为 `1/1`；report run `report-20260726043833628-51883` 为 `1/1`。
- 真实链从 UI 创建 fresh group 开始，关联真实 Tauri invoke、Helix bus ingress、只读 SQLite、PG/Go 证据、FrameApplied 与 DOM。fresh group 会含一条真实入群系统消息；当本地 coverage 已完整时，G10 合法地以零 `getLatestPost` HTTP 完成。
- `message:wdio:g10:*` 运行输出过非致命 `tauri-driver not found` 诊断，但 embedded provider 实际启动并完成上述 suite；该诊断没有替代或掩盖 readiness/断言失败。
- production boundary 扫描 `2735` 个生产文件，Fixture/Mock/WDIO API production reachable=`false`。

## T023-T032 Mobile Contract

- Mobile 提交为 `0e9400c5044d19dd47f7b97899ff2a611ca57a92`；production Helix pin 为 `f221c924d4da606035ed040c5d4583ccb5a250dd`，bundle `artifacts/helix/helix-f221c92-g10.bundle` SHA-256 为 `b17f36b982b2eda2080ac2125bff6efa6672f259229dc28ca25bd9f21fad2bc5`。
- G10 mirror parity 为 `6/6`；source pin 证明 fixture producer `e10719ad...` 可达 production pin；legacy hard-cut 报告 production V1 producer/consumer/raw reducer 均为 `0`。
- `G10FrameContractTest.*` 为 `7/7`，覆盖完整 envelope、scope/window token/page/items/actions/revisions/correlation、failure/race、DomainEvent batch、SurfaceRuntime atomic apply 与 QuickJS typed view。
- `./scripts/run-uc01-16-gtest.sh --only UC10` 为 `9/9`，runId `mrc-mobile-uc-20260726044857-63057`；真实创建 fresh group、发送 22 条消息、query admission、最终 20-row typed window、最新 sent row 与 cleanup 全部完成。
- 本 Gate 未改变 Venus/QuickJS 接线能力，按总控合同复用共享 Venus 回归而不新增专属 Gate。生产链保持 `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch -> SurfaceRuntime -> QuickJS im.surface.view`。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 三份 provenance SHA-256 均为 `4d35068679462d7ee830e1511270e95c031992874d725edfff15863215ea0469`。
- inventory 均为 `group-created`、`timeline-ready`、`timeline-failed`、`timeline-cache-degraded`、`timeline-race-current`、`timeline-recovery`，schemaVersion 全为 `2`。
- 六份 raw SHA-256 在三仓逐文件相同：`f882610591c1d17d7e08833ac7145eb872505f92e8a3bf41fa4074f46dea9d20`、`aebfa041fe3c1430479643c3b1a70baf10b0ca562a98332e77518bb8fb9cf9c2`、`9d45acbd9df01f08cd0bf0d1fb7cab52cb07da92a65f6374d0d5f8eb5670f884`、`93589ced1b2d055efea213790fcbed96923b95458a54feff618e1881e881f9a5`、`a24b676f13eef747c1ba90e4150831ff6e78b22777c4ef33d0e3674eab0c2239`、`667f3c32a183c3520dcf2ee69ea4a6f8a7299b3f36f49749ea9d73f74ebc7e47`。
- provenance byte-exact 一致，因此 semantic SHA-256 与 surface inventory 同样一致；PC/Mobile 未补写 Helix 业务字段。

## T034-T036 Review

- **Behavior Green**：fresh group、local-complete、partial remote、HTTP failure、cache persist failure、迟到/重复 authority、local recovery 与 Actor/Observer 最终结果都由 Helix 同一 coverage/query-generation 模型决定。HTTP 是条件性 authority，不把零 HTTP 的完整本地窗口误判为缺链。
- **Code Boundary Green**：local-first coverage、条件 HTTP、cache persist barrier 与 timeline projector 只在 Helix；PC/Mobile 只做 production command admission、完整 Frame atomic apply 和平台渲染。Fixture、Mock 与测试注入默认 production build 不可达。
- **Frame V2 Green**：六份完整 envelope 的 schema/scope/surfaceEpoch/revision/baseRevision/window token/page/items/actions/directives/correlation 已字段级对齐；cache-degraded 不冒充 durable coverage，race-current 与 recovery 保持独立可观察终态。

## T037-T038 Gate Report And Quickstart

- 结构化汇总见 `evidence/gate-report.json`。required lanes fail=`0`、skip=`0`，final verdict=`Green`。
- Helix、PC Mock/Real/Report、Mobile parity/GTest/real runner、production boundary 与三仓 hash 对账均 exit `0`；三仓 `git diff --check` 通过。
- PC 初始失败真实暴露了三项 harness 假设错误：需显式选中 fresh group、群内存在系统入群消息、local-complete 允许零 HTTP；修正测试观察边界后独立重跑全绿，未向 Angular 添加业务 reducer。
- mandatory after_implement 与 convergence 只有在 R4 接受本证据、任务 fingerprint 不再变化时才允许最终 `CLOSED`。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-10` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
