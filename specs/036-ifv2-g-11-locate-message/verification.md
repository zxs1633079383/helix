# Verification Procedure: G-11

## Source Lock

| Plane | Locked value | Evidence |
|---|---|---|
| Helix HEAD | `8c732a84cf0c0d29e65646e3aa5bfd7e87675856` | `git rev-parse HEAD` |
| cses-client HEAD | `f1c4ddc0fbbd4bb4547108627af83a028284a343` | `git -C .../cses-client rev-parse HEAD` |
| Mobile HEAD | `26c7616d805f0ba21406f0f9fa3579e212bdd090` | `git -C .../mobile-qucik-c++ rev-parse HEAD` |
| PC Helix pin | `d2b1bf6c1304155d6897a7238cfaefd2bae3d176` | `src-tauri/Cargo.toml` |
| Mobile Helix pin | `7e93be302d251fd4e63ca47c637dc0b3f289e1a1` | `contract/helix-source.lock.json` |
| Strict Matrix SHA-256 | `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac` | current Helix contract |
| G10a prerequisite | `phase=closed,event_seq=17` | `specs/035-ifv2-g-10a-load-older/review-ledger.json` |

`check-prerequisites.sh` 的持久 feature 指针原停在 G09；本轮已显式锁到
`specs/036-ifv2-g-11-locate-message`。before-implement 在已经 admitted 后重复 advance，使 ledger
提前进入 `verifying/missing evidence`；这不作为完成证据，必须由本轮真实 evidence 修复。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-11` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Helix Producer Run

- `cargo check -p helix-im`: exit `0`, `6.51s`。
- `cargo test -p helix-im --test g11_full_flow_test --test g11_locate_interaction_test --test read_relay_test -- --nocapture`:
  exit `0`；G11 full flow `7/7`、字段 helper `1/1`、read relay regression `11/11`。
- 生产结果：`posts/get` body 仅含 `postIds`；HTTP reply 必须与 attached target/channel 同一；
  success 只发布一个 `message.timeline` Frame，`targetMessageId`、`anchor.mode=locate` 与
  payload-free `revealMessage` directive 同 Frame；transport failure、absent target 与 duplicate reply
  均产生 `0` Effect。
- `UPDATE_G11_WDIO_FIXTURES=1 cargo test -p helix-im --test g11_wdio_fixture_contract_test -- --nocapture`:
  exit `0`，`1/1`；普通测试同样执行 actual-vs-Golden 比较。
- Canonical inventory `3`：`group-created`、`timeline-initial`、`timeline-located`。raw SHA-256
  分别为 `03119203...831a`、`f8bdc187...0214`、`40822b96...21a`；semantic SHA-256 分别为
  `951d07de...c960`、`b91a6b0d...ad27`、`56fe29d0...65de`。

## T013-T022 PC Mock/Real/Report

- cses-client 接受提交为 `509d10cae`；Tauri 四个 Helix production dependency
  统一 pin `5e3ffc026fc8b25f76a7cce162e75734bddb45ca`。Fixture 同步器、
  typed loader、AcceptedCommand/replay split、MessageV4 page object、G11
  reporter 与 production boundary 均完成，Angular 未新增 locate reducer。
- Fixture parity、focused Jest `24/24`、WDIO TypeScript、production candidate
  build 均 exit `0`。Mock runId
  `mock-20260726070428498-53337` 为 `3/3`，覆盖成功定位、command admission
  failure 与 duplicate Frame，claim 仅为 `mock`。
- single-real runId `single-real-20260726070534306-54241`、dual-real runId
  `dual-real-20260726070608200-54927`、report runId
  `report-20260726070501810-53776` 均为 `1/1`、exit `0`。真实链从
  MessageV4 UI 创建 fresh group，经真实 Tauri invoke、Helix HTTP、
  `im:__bus__` Frame ingress、SurfaceRuntime apply 与 DOM 自动等待闭合。
- dual-real 明确验证 locate 是 Actor-local 行为：Observer 不被伪造 locate
  target 或 Frame。production boundary 扫描 `2735` 个生产文件，
  Fixture、Mock bridge 与 WDIO-only API production reachable=`false`。
  embedded single lane 打印非致命 `tauri-driver not found` 诊断，但真实
  WebKit/Tauri session 已启动并通过，未将该诊断冒充或替代真实链证据。

## T023-T032 Mobile Contract

- Mobile 依次提交 `298d778`（production Helix pin）、`71d6cb6`
  （canonical mirror）与 `49b9430`（GTest/QuickJS/real chain）。
  production Helix pin 为 `60b2d5f579f209a144433db1f96bef8a63f6bd5c`，
  bundle SHA-256 为
  `127e60902595cddb6324a10c0fe68cb94fb6cf3968e63a813914fa28785673e6`。
- G11 mirror parity 为 `3/3`；source pin 证明 Fixture producer `5e3ffc...`
  可达 production pin；legacy hard-cut 报告 G01-G11 production V1
  producer/consumer/raw fallback 均为 `0`。
- `G11FrameContractTest.*` 为 `6/6`，覆盖完整 envelope、初始 snapshot、
  located patch、target/anchor/directive、duplicate、schema V1 fail-closed、
  SDK command admission、完整 DomainEvent batch、SurfaceRuntime atomic apply、
  QuickJS `im.surface.view` typed state 与 production C ABI。
- `./scripts/run-uc01-16-gtest.sh --only UC11` 为 `7/7`；真实链创建 fresh
  group 与目标消息，生产 `im.posts.locate -> im_get_posts` 最终产生
  `targetMessageId`、`anchor.mode=locate` 与 sent target row，随后完成 cleanup。
- 共享 `./scripts/run-uc01-16-gtest.sh` 实际执行 `150` tests、耗时
  `718161ms`：`141 PASS / 9 FAIL`。同一 runId 中 G11 为 `7/7 PASS`；
  失败归属为 GS02/GS03/GS04、UC01/UC02、UC12/UC13/UC14/UC15 的
  RealChain timeout，未被 G11 局部 Green 掩盖，分别留待其 owning Gate
  复核和收敛；本 Gate 未修改这些测试或业务路径。
- `MobileImSdk -> MobileFfiBridge/helix-driver-ffi -> DomainEvent batch ->
  SurfaceRuntime -> QuickJS typed view` 的生产接线已存在并由 production-linked
  GTest 证明，因此 T027-T030 未复制或修改业务逻辑。本 Gate 未改变 Venus
  接线能力，按总控合同复用 `run-venus-helix-im-smoke.sh`，结果 PASS。

## T033 Three-repo Fixture Parity

- Helix、PC、Mobile 的 `provenance.json` SHA-256 均为
  `420c6a7b749b407a6debfbf077fc9843ce997a3897940dd51d66a2ae6178b3fd`。
- 三仓 inventory 均为 `group-created`、`timeline-initial`、
  `timeline-located`，schemaVersion 全为 `2`，surface inventory 分别为
  `message.left-dialog-list`、`message.timeline`、`message.timeline`。
- 三份 Frame raw SHA-256 在三仓逐文件相同：
  `03119203...831a`、`f8bdc187...0214`、`40822b96...21a`；provenance
  byte-exact 一致，因此 semantic SHA-256 与动态字段白名单同样一致，
  PC/Mobile 未补业务字段。
- PC Mock 首次运行实际证伪旧 provenance：dialog patch 的允许动态字段错误
  指向 `mutation/view`。Helix producer 修正为 `mutation/patch/view` 后重新
  生成合同并提交 `60b2d5f`，两端只同步修正后的 canonical bytes，没有前端绕过。

## T034-T036 Review

- **Behavior Green**：fresh group、attached timeline、同 channel target、
  HTTP authority、missing/cross-channel/transport failure、duplicate/late reply
  与 detached recovery 均由 Helix 同一 locate model 决定。HTTP success
  不是 Frame 终态；Observer 不适用时不伪造跨账号定位传播。
- **Code Boundary Green**：业务 authority、target 选择与完整 Frame 只在
  Helix `query/dispatch + surface_composer/projector`。PC 与 Mobile 只做
  production command admission、完整 Frame atomic apply 与平台渲染；
  Fixture、Mock、test injection 默认 production build 不可达。
- **Frame V2 Green**：schema `2`、transaction/causation、timeline scope、
  surfaceEpoch、`baseRevision=1 -> revision=2`、完整 20-row patch、
  message actions、`targetMessageId`、`anchor.mode=locate` 与
  `revealMessage` directive 已字段级对齐；duplicate/unknown schema 均不部分 apply。

## T037-T038 Gate Report And Quickstart

- 结构化汇总见 `evidence/gate-report.json`，并由
  `loop_engine.py record-evidence` 写入 `evidence/evidence.json` 与 SHA-256；
  G11 required lanes fail=`0`、skip=`0`，final verdict=`Green`。
- Helix、PC Mock/Real/Report、Mobile parity/GTest/UC11 real、production
  boundary、共享 Venus 回归与三仓 byte parity 均完成；三仓
  `git diff --check` 通过。共享 Mobile 全量 runner 的 9 个其他 Gate
  RealChain timeout 已作为独立观察记录，不替代或污染 G11 required lane。
- mandatory after_implement 只有在 R4 接受本证据、tasks fingerprint 不再
  变化时才允许最终 `CLOSED`。
