# Verification Procedure: G-04a

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-04a` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`394839c76be083947bbbb2993286e4582055a50b`；前序 G04 子 Feature
  T001-T038 全部完成，mandatory R4 `PASS` 且最终 `CLOSED`。
- cses-client：`c91cafc8359495daff2969ca4ed0db729027829f`，tracked tree clean；
  Tauri 四个 Helix dependency 当前统一 pin
  `43008acfc464a17711613fe5f37a56bb1146d22e`。
- Mobile：`094a15149f448b7ce952b71688e5bae9fe278769`；tracked tree clean，
  保留任务前已有 bundle 与 `tests/.DS_Store` 未跟踪项。production Helix pin 为
  `ee9b8f4492fd82aa846141be6f01f5729fc64bf8`。
- Strict Matrix
  `crates/helix-im/docs/Strict Matrix/UC全链路发布矩阵-v1.md` raw SHA-256 为
  `6f186cf291dc6336798b1178bf3d97493588eea8ccc467f026766654d3fe2354`。
- 当前已有 explicit sync、viewer-filtered batch、PersistAtomic 与 recalled
  projector，但没有 G04a 完整 ScenarioRunner、canonical Fixture、PC 独立 lane
  或 Mobile G04a typed contract；只满足实现准入，不声明 Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用 G01/G04 已验证的生产 fresh-group prerequisite，取得权威 channel，
  再发送并持久化一条具备 revoke capability 的消息；不复用 G04 post/cursor。
- PC 复用 domain-neutral fresh group helper；G04a 只新增离线/重连/sync 操作，
  不复制群创建 reducer。
- Mobile command contract 先调用生产 `im.channels.create`；canonical Frame
  prerequisite 仍由 Helix producer 提供，GTest 不手写 group VM。
- 三端本 Gate 每次验收均创建新群；共享 prerequisite 当前无需生产代码修改。

## T004-T012 Helix Producer Green

- producer commit：`c9c37ea8fcb398715cff872d4adbdc8a0e412c46`。共享
  `G04AScenarioRunner` 只经公开 `im_sync_channels`、`ImModule::handle`、
  `Effect::PersistAtomic`、匹配 `PortReply` 和 `Effect::Emit` 运行；未新增测试旁路。
- 当前生产链已满足目标，无需修改 `sync/increment.rs`、`surface_composer.rs` 或
  projector：平台 `eventSeq=999999` 未进入请求，fresh channel 实际
  `fromSeq=0`；type3 authority fact 只在原子持久成功后产生
  `message.timeline` recalled Frame。提交后 cursor 为 1，重复 sync 与迟到 WS
  均无业务 Effect。
- `cargo check -p helix-im`：退出码 0。
- `cargo test -p helix-im --test g04a_full_flow_test --test
  g04_revoke_interaction_test -- --nocapture`：退出码 0；G04a 7/7、focused
  interaction 5/5。
- 显式更新命令 `UPDATE_G04A_WDIO_FIXTURES=1 cargo test -p helix-im --test
  g04a_wdio_fixture_contract_test -- --nocapture`：退出码 0；随后不带更新开关
  重跑 1/1 通过，证明普通测试只比较 Golden。
- canonical inventory 为 4：`group-created`、`revocable`、
  `offline-recalled`、`recovered-recalled`，schema 全为 2；surface inventory
  分别为一个 `message.left-dialog-list` 与三个 `message.timeline`。
- raw SHA-256：
  `56b191ad60c644427cdea784de5cc867eeae018efd697bf3fe7deb63d1c05325`、
  `ac33de58a0055b3ca96fd39aabd40b861a4784795ff8639778e6259108a5baed`、
  `bb5a126f0581a1db8b09e943f108c0d6d1c6b3bc149fb4ed9d5e178268874a95`、
  `ad2d4712c24e07b336d858a0272a09877aa255567a4f59993e46b8c579d3bd11`；
  provenance raw SHA-256 为
  `ac7ef7c8c301da9e7ee76b1815038e75f4313d549fcc2330836b75074f5b435e`。

## T013-T022 PC Mock/Real/Report Green

- cses-client commits：Fixture mirror `e6d3edfed063de22d0a40849bab028eb091f7a95`；
  production Helix pin `2283c3600`；Mock/Real/report `4b8721fec`；补齐 failure 与
  duplicate Frame `f0cc3d187`。Tauri 四个生产 dependency 统一 pin
  `c9c37ea8fcb398715cff872d4adbdc8a0e412c46`。
- `node scripts/message-wdio/sync-helix-fixtures.mjs --gate=g04a --check`：
  退出码 0，`G04A_FIXTURE_PARITY_OK count=4`；PC mirror 与 Helix raw bytes、
  schema、surface inventory、raw/semantic SHA-256 一致。
- `yarn message:wdio:g04a:mock`：退出码 0，3/3 通过。覆盖 AcceptedCommand 与
  Frame 分离、authority 前 DOM 不变、admission failure 不伪造 recalled、
  duplicate terminal Frame 不推进 revision，以及 cold recovered snapshot。
- `yarn message:wdio:g04a:single-real`：退出码 0，run
  `single-real-20260725183605324-12955`。真实 UI 建立 fresh 双用户群，真实
  Tauri invoke 发起撤回与 explicit sync；run-scoped Go fault 只丢 Actor
  `posts_update`。最终 sync HTTP 200、FrameApplied、recalled DOM、Actor SQLite
  与 PostgreSQL `revoke=true` 闭合。report SHA-256
  `f4d4cdc794f6157bda6673e87eb7b5465a6ad6dbb2cd55d65d39d1d568391f75`，
  real evidence SHA-256
  `5a0c3c261964e77277694708b0290b21f6b874499c25174642689ae360d6cca6`。
- `yarn message:wdio:g04a:dual-real`：退出码 0，run
  `dual-real-20260725183627257-13264`。Actor 在线收到撤回；Observer 的
  `posts_update` 被当前运行故障窗抑制，Observer 仅经真实
  `im_sync_channels` 收敛；双端 recalled DOM、SQLite 与 PG 关联同一 post。
  report SHA-256
  `e53ad426bb805c0f39a6cece91b4de76c20eeb40e8c221ba5e992288d109190d`，
  real evidence SHA-256
  `713db0bdded7ff821956ca6da89a181ee1477a058b2136d01d4205b7021cdf6b`。
- 证据分级保持真实：`/posts/revoke` 使用 validated `mrc-*` track 并为
  verified；`/channel/sync/notify` 使用生产 Helix `helix-sync:<corr>`，
  Go 正确记录为 current-run observed、`verified=false`。报告保留
  `source=fault-sync-notify` 与该布尔值，不将内部 sync track 冒充用户操作
  verified；真实 command admission、HTTP 200、Frame、DOM 与 storage 共同闭环。
- `yarn message:wdio:contracts`：33 suites、109 tests 全过；`yarn
  message:wdio:g04a:report` 1/1；`yarn message:wdio:g04a:boundary` 输出
  `checkedFiles=2735, mockProductionReachable=false`；candidate boundary check
  退出码 0。single/dual artifact redaction 各扫描 6 文件并通过。
- `yarn message:wdio:build` 完成 Angular candidate 与 Rust/Tauri 构建，产物
  `src-tauri/target/debug/cses-client`。启动日志中 `tauri-driver not found`
  仅为外部 service 探测噪声；四条 embedded WebDriver lane 实际均完成执行，
  不作为 Green 替代证据。

## T023-T031 Mobile Contract Implementation

- Mobile commits：生产 pin `e246cbf`、Fixture mirror `0725f65`、typed view
  contract `e6cafba`、真实链 typed timeline `883fca7`。production Helix pin
  固定为 `bde8d79fba6335f9d6d8ed3feafab05a80fcd151`，正式 bundle
  `artifacts/helix/helix-bde8d79-g04a.bundle` SHA-256 为
  `ab47694baf9b50160c87644beb47f14b2379a2d0d938a674f39b93a60a0480d8`。
- `sync-g04a-helix-fixtures.mjs --check` 与
  `check-g04a-fixture-parity.mjs` 均退出 0；四份 Frame 与 provenance
  byte-exact 镜像 Helix，校验覆盖 pin、schema、inventory、raw/semantic
  SHA-256、surface inventory 与禁止补写业务字段。
- `G04AFrameContractTest.*`：5/5 PASS。测试链接生产 `MobileImSdk`、
  `MobileFfiBridge`、`SurfaceRuntime`、QuickJS binding 与 C ABI，覆盖 fresh
  group、revocable snapshot、offline recalled patch、cold recovered snapshot、
  duplicate/revision/schema/surface 原子拒绝、DomainEvent batch 和只读
  `im.surface.view`。
- `Uc05RevokeTest.*-Uc05RevokeTest.RealChain`：8/8 PASS；随后
  `run-uc01-16-gtest.sh --only UC05`：9/9 PASS。真实链每次创建 fresh 双用户群，
  Observer 在线建立 typed timeline baseline；Observer 离线期间 Actor 撤回；
  Observer 以同一 view DB 重启后调用生产 `im_sync_channels`，最终证据为
  `frameApplied=true`、`typedView=true`、`replay=true`，且 recalled row 恰好
  一条、actions 为空。
- T027-T030 未制造 Gate 特判：当前生产 `MobileImSdk::syncChannels` 已提供
  command admission 和 Frame event pump；`MobileFfiBridge` 保持整批
  DomainEvent 边界；`SurfaceRuntime` 原子 apply 并对 unknown
  schema/surface/revision fail-closed；QuickJS 只暴露 `im.surface.view` typed
  view。实现已满足合同，因此本 Gate 只增加 source-grounded 测试与真实链，
  不复制第二套 reducer。
- `check-surface-runtime-source-pin.mjs` PASS，输出 G02=6、G03a=8、G04=4、
  G04a=4；`check-surface-runtime-legacy-hard-cut.mjs` PASS，输出 production
  V1 producers=0、consumers=0 且 G01/G02/G03a/G04/G04a typed-only。
- 共享全量 `run-uc01-16-gtest.sh` 完整运行 780410ms：150 tests，136 PASS、
  14 FAIL，进程退出 1。G-04a 对应 `Uc05RevokeTest.RealChain` 在该同一 session
  中再次 PASS（7893ms），失败均来自尚未进入本轮纵向闭环的 GS02/GS03/GS04、
  UC01/UC02、UC06-UC08、UC10-UC15 真实链。该全量结果作为后续 Gate 的真实
  blocker 清单保留，不拿来替代或否定 G-04a required Mobile Contract；本 Gate
  自身 required filter 为 9/9 PASS、skip=0、fail=0。

结论：G-04a Mobile Contract required lane 为 Green；共享全量回归仍为 Red，
其 14 个失败必须在各自顺序 Gate 内关闭，不在 G-04a 恢复 raw query、V1 consumer
或前端业务 reducer 来制造总绿。

## T033-T037 Cross-repo Review Green

- 三仓 inventory 均为 4 Frame + 1 provenance，逐文件 byte-exact SHA-256
  一致：`group-created=56b191ad...`、`revocable=ac33de58...`、
  `offline-recalled=bb5a126f...`、`recovered-recalled=ad2d4712...`、
  `provenance=ac7ef7c8...`。PC 与 Mobile parity gate 同时复核
  schemaVersion=2、semantic hash 与 surface inventory，consumer 没有补写业务字段。
- 行为 Review 为 Green：每次 canonical/real 验收创建 fresh 双用户群；只有 Helix
  签发 revoke capability 的 Actor 可执行撤回；Observer 离线期间不靠本地猜测改变
  timeline。Actor 权威撤回完成后，Observer 通过真实 `im_sync_channels` 收敛；
  sync/admission/persist 失败不伪造 recalled；duplicate sync、重复 terminal Frame
  与迟到 inbound 不重复推进 revision；cold recovery 直接恢复 recalled snapshot，
  不复活 action。PC single/dual real 与 Mobile UC05 对 Actor/Observer 最终用户结果
  给出独立真实链证据。
- 代码边界 Review 为 Green：离线 authority、cursor、PersistAtomic barrier、终态、
  幂等和 recovery reducer 只在 Helix。PC production bundle 扫描 2735 文件，
  `mockProductionReachable=false`；Mobile production SDK/Bridge/SurfaceRuntime/
  QuickJS 仅做 command admission、整批 DomainEvent transport、原子 apply 与 typed
  view，生产 V1 producer=0、consumer=0，未注册测试注入或 raw reducer。
- Frame V2 Review 为 Green：`group-created` 为
  `message.left-dialog-list` patch `base=1 -> revision=2`；`revocable` 为
  `message.timeline` snapshot revision 1，文本 sent 且包含 Helix 签发 actions；
  `offline-recalled` 为 timeline patch `base=1 -> revision=2`，
  `content/delivery=recalled` 且 actions/quickActions 为空；`recovered-recalled`
  为无 baseRevision 的 recalled snapshot revision 1。四份 Frame 均
  `surfaceEpoch=1`、runtime=`online/ready/idle`、directives 为空。G-04a
  authority 来自 durable sync fact，envelope/mutation 的 `causationId` 缺省是生产
  合同，不伪造 request correlation；transactionId、scope channel 与 revision
  共同完成关联。
- required lanes 为 Helix、Fixture、PC Mock、PC Real、Mobile Contract 与
  production boundary，全部 Pass；required failure=0、required skip=0。共享 Mobile
  26 Gate 全量回归的 14 个失败作为未来顺序 Gate blocker 留档，不计入 G-04a
  required pass rate，也不被本 Gate 掩盖或修补。
- 机器汇总见 `evidence/gate-report.json`，当前 `finalVerdict=Green`；仍须 T038
  quickstart/diff check、evidence manifest 与 mandatory after_implement R4 决定
  是否真正 `CLOSED`。

## T038 Final Review

- quickstart 中 Helix、PC 与 Mobile 的 G-04a required commands 已按前述证据完成；
  共享 Mobile 全量 runner 也完整结束并保留 14 个未来 Gate blocker，没有提前中止
  或把失败改写为通过。
- 三仓 `git diff --check` 均退出 0；PC tracked tree clean，Mobile tracked tree
  clean 且只保留任务前已有的 bundle 与 `tests/.DS_Store` 未跟踪项。
- `evidence/gate-report.json` 满足本 Feature 的
  `cross-repo-report.schema.json` 必填字段、枚举与 Gate 常量；
  `evidence/evidence.json` 登记 V-001 至 V-007 七个 hashed PASS run。
- mandatory after_implement、targeted converge 与 after_converge 由
  `speckit-helix-loop-advance` 执行；只有 ledger 最终返回 `CLOSED`，总控才可把
  G-04a 标记 `review-green` 并解锁 G-05。
