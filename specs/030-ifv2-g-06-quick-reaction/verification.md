# Verification Procedure: G-06

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-06` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`714677ab1e5f6a5126fa45fea0a12da744cbae65`；前序 G05
  T001-T038 全部完成，R4 `PASS` 且最终 `CLOSED`。
- cses-client：`424d1c9e5e79351a74149c1f1c0088eb92f85352`，tracked tree
  clean；Tauri 四个 Helix production dependency 统一 pin
  `3798ffdd0bfd89b8922829dd252327bb0a2075ec`。
- Mobile：`1b6b77856d5d5118fce87336a85e42a959012693`，tracked tree
  clean；任务前已有八份 Helix bundle 与 `tests/.DS_Store` 保持未跟踪。
  production Helix pin 为
  `0147ee8856aa2b072c7a7537d3c968d5bbbfdd50`，bundle SHA-256 为
  `f01ead551d1eefd059119beb5c7146322a6973425fae9aec20a6211e475f8bbd`。
- Strict Matrix 机器真源
  `crates/helix-im/contracts/strict-matrix.v1.json` raw SHA-256 为
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`；
  人读生成文档 raw SHA-256 为
  `6f186cf291dc6336798b1178bf3d97493588eea8ccc467f026766654d3fe2354`。
- 当前已存在 canonical HTTP wire、ActionBinding pending authority、
  `post_update` atomic persist 与 timeline refresh；尚无 G06 完整 ScenarioRunner、
  canonical Fixture、PC 独立 lanes，且 Mobile 仍错误提交 `user_id`。因此仅满足
  implementation admission，不声明 Helix/PC/Mobile 任一 G06 lane Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用 G01 production prerequisite：`im_create_channel` 取得权威
  `channelId`，再在同一 fresh group 发送 source post；G06 只消费权威
  channel/post ID，不复制建群 handler。
- PC 继续使用 `g01-group-prerequisite.ts` 从真实 MessageV4 UI 创建群并选择
  444/678；Mock 只 replay Helix canonical group/timeline Frame。
- Mobile 继续使用 `UcJsContractFixture::RC.createChannel` 与 `sendMessage` 创建
  fresh group/source post；不使用固定旧 channel 作为 canonical real 前置。
- 三端共享能力当前足够，无需新增第二套 CreateGroup 生产逻辑；后续 G06
  Scenario/Real lane 必须证明每个 run 使用新 channel。

## T004-T008 Helix Production Flow

- `crates/helix-im/tests/g06_full_flow_test.rs` 通过生产
  `ImModule::handle` 覆盖 fresh group、opaque action happy path、客户端
  `user_id` 非 canonical 业务失败、HTTP 基础设施失败、重复 WS/迟到 HTTP
  竞态、PersistAtomic 失败重放与 durable recovery，共 `7/7` 通过。
- `crates/helix-im/tests/g06_reaction_interaction_test.rs` 对 direct command
  wire、ActionBinding、持久屏障、causation、重放和恢复执行字段级断言，共
  `5/5` 通过。
- 生产 handler 当前已经严格接收 `post_id + emoji + optional req_id`，HTTP
  body 仅 `{postId,emoji}`；WS `post_update` 经 message+cursor
  `PersistAtomic` 后触发 attached timeline readback。测试证明无需修改
  `quick_reply.rs`、`surface_composer.rs` 或 projector 生产代码，不以测试旁路
  制造 Frame。
- `cargo check -p helix-im`：exit `0`。
- `cargo test -p helix-im --test g06_full_flow_test --test
  g06_reaction_interaction_test -- --nocapture`：exit `0`，`12/12` 通过。

## T009-T012 Canonical Fixture And Helix Green

- producer：
  `G06ScenarioRunner -> ImModule::handle -> Effect::Emit ->
  InteractionFrameV2::decode`，source revision
  `4084a6f3b0f9f8928f9559b0cda0e5dd700a8fcd`。
- inventory：
  `group-created.frame-v2.json`、`reactable.frame-v2.json`、
  `reacted.frame-v2.json`、`recovered-reacted.frame-v2.json`；普通测试
  未设置更新开关时只比较 Golden，不写盘。
- raw SHA-256：
  `8b62d9f62ed579b932ff682eaac40214545844bee5ffe9e0e071397f3f8a1010`、
  `169063bc94e1834b09a2ccb79fe0cdefd1127a6755ae068269592bc58bafb7f2`、
  `1c44cad20b7a29703def1a3485b74bb868d7b87a13bf4ddd8a8bf2ab4eaa8f55`、
  `4df26140063c35d5a42fc77d529a0651df599871b776f281d2738b06cc1dbe37`；
  provenance raw SHA-256
  `049f0bc7a42158f60ae7ac79b29dd7afcedddd742836b01db6d08f9fa15cfeeb`。
- semantic SHA-256：
  `445a9105088a86e7c5e83b894e86f827f11e8e8e7eab9732b26c8636d426042a`、
  `7c41f9268e071e43a8dd50300512a702a334e6de15d0f620999234eaf0a63a0e`、
  `d87f4567b9b4ec2e4b5d8bc0abe8fb103bce5b81a3ab74360740ae1fe158e204`、
  `dab2a986b7f7d6228809c92c66019e66e4928befe88980cb0f6f3d74289ad67f`。
- `UPDATE_G06_WDIO_FIXTURES=1 cargo test -p helix-im --test
  g06_wdio_fixture_contract_test -- --nocapture`：exit `0`，显式生成
  `1/1 PASS`。
- 普通 `cargo test -p helix-im --test g06_wdio_fixture_contract_test --
  --nocapture`：exit `0`，Golden/provenance `1/1 PASS`。
- 原始输出：
  `evidence/v001-helix-flow.log` 与 `evidence/v002-helix-fixture.log`。
  至此 G06 Helix producer 与 canonical Fixture Green；PC/Mobile 尚未 Green。

## T013-T022 PC Mock/Real/Report Green

- cses-client commits：
  `935cda7718e3b049a9a8c651265f4290d0d70ac4` 只读同步 G06
  canonical Fixture、typed loader、raw/semantic parity 与 TauriMockBridge
  receipt/frame 分离合同；
  `ab7833996be5156f4d0e0655c18e7f9c65488961` 完成 production
  reaction binding 可观测属性、Mock、Single Real、Dual Real 与报告 lane。
- `node scripts/message-wdio/sync-helix-fixtures.mjs --gate=g06 --check`：
  exit `0`，`G06_FIXTURE_PARITY_OK count=4`，Helix producer pin
  `4084a6f3b0f9f8928f9559b0cda0e5dd700a8fcd`。
- Fixture/Bridge/report Jest：exit `0`，`17/17`；WDIO TypeScript：
  exit `0`；MessageSurfaceRuntime reaction selector 单测包含于 `26/26` PASS；
  `message:wdio:build` exit `0`。
- `message:wdio:g06:boundary`：exit `0`，
  `checkedFiles=2735`、`mockProductionReachable=false`。Fixture、Mock bridge
  与 WDIO-only API 未进入 production bundle。
- Mock：exit `0`，`2/2`；报告 SHA-256
  `0cda3a7e1a1cdadb3712c3151e87a59f6c84ac8193a1b0b7576f9f8453f88bfa`，
  `evidenceLevel=mock`，未提升为真实链证据。
- Single Real：exit `0`，`1/1`；真实 MessageV4 UI fresh group →
  `im_invoke_action` → Helix → SQLite/PG → `/api/cses/posts/quickReply` →
  WS `post_update` → FrameApplied → Actor DOM 闭合。报告 SHA-256
  `dc07106f5b4752f9f2353d6e14864555de0bc66add2ff52da53502370e645a55`。
- Dual Real：exit `0`，`1/1`；Actor/Observer 在隔离 profile 对同一
  canonical post/reaction 收敛，两个 FrameApplied 与两个 DOM 均关联。
  报告 SHA-256
  `cb83410e0ea94287b77322969fa59a078b638e75da1380e61ae34258841aed3b`。
- 独立 report contract：exit `0`，`1/1`；报告 SHA-256
  `b1db72bb81c9bda659d7e6bf03cf05f201d526eb6a3fa8873164e506a46dd59a`。
  embedded provider 启动时会打印本机缺少外部 `tauri-driver` 的诊断，
  但四条 lane 均建立 WebKit session 并由真实退出码判定，不将该提示当作
  Green 或 blocker。

## T023-T032 Mobile Contract Green

- Mobile commits：生产 Helix 真源固定 `9fcc7fa`；G06 Fixture 镜像合同
  `9b65fca`；生产 SDK、QuickJS typed view、GTest 与 UC07 真实链
  `3bd7650a2545e4a8a2d56b552d2ad20ee1bbce8e`。Mobile production
  Helix pin 为 `6a2703971a2e452c191d6a3b6481a5a3c51998fc`，bundle
  SHA-256 为
  `deefc3ea88a6c01fd394ab618d4177843e56238dbeae71157ec302e20104a9ad`。
- `check-g06-fixture-parity.mjs`：exit `0`，
  `G06_MOBILE_FIXTURE_PARITY_OK count=4`；四份 Frame 与 provenance
  byte-exact，schema、surface inventory、raw/semantic hash 无 Mobile 补字段。
- `G06FrameContractTest.*`：`5/5` 通过。覆盖完整 envelope/runtime、
  timeline mutation/action、revision/causation、duplicate/schema/surface/revision
  fail-closed、durable recovery、生产 SDK admission、FFI DomainEvent batch、
  C ABI 与 QuickJS `im.surface.view`。
- Mobile 生产 command 仅提交 `post_id + emoji`；actor 由认证会话确定，
  QuickJS 返回 admission，但 reaction 终态只接受 Helix Frame。不存在
  `user_id`、本地 reaction patch、`im.store` 或 raw reducer。
- focused SDK/QuickJS/UC07（排除真实链）`9/9` 通过；共享
  `run-uc01-16-gtest.sh --only UC07` 最终 `8/8` 通过，真实 run
  `mrc-mobile-uc-20260725221735-56208` 从 fresh group/source post 开始，经
  `im_quick_reply -> MobileFfiBridge -> Helix/Go -> DomainEvent batch ->
  SurfaceRuntime atomic apply -> QuickJS typed view` 收敛。
- 首次真实链尝试错误等待已被 V2 consumer 隐藏的 `im:post:updated`
  lifecycle event，真实失败并保留；修复为按权威 query/Frame latch 等待 typed
  view，不以 fixed sleep 或 V1 event consumer 冒充成功。
- `check-surface-runtime-source-pin.mjs` 与
  `check-surface-runtime-legacy-hard-cut.mjs` 均 exit `0`：
  `production V1 producers=0 consumers=0`，G01-G06 typed-only；
  `sync-helix-command-api.mjs --check` 与 diff check 同时通过。

结论：G-06 Mobile canonical mirror、生产 SDK/Bridge、SurfaceRuntime、
QuickJS typed view 与 focused real-chain required lane 为 Green。

## T033-T037 Cross-repo Review

### Fixture Parity

- Helix、cses-client 与 Mobile 三目录 inventory 均为四份 Frame 加一份
  provenance，逐文件 SHA-256 完全一致。raw hash 依次为
  `8b62d9f6...`、`169063bc...`、`1c44cad2...`、`4df26140...`，
  provenance 为 `049f0bc7...`。
- semantic SHA-256 依次为 `445a9105...`、`7c41f926...`、
  `d87f4567...`、`dab2a986...`；三端 checker 使用同一 reaction 语义规则，
  未发现平台补写 actor、reaction aggregate 或 ViewModel 字段。

### Behavior Review

- 每次 canonical/real 验收都先创建 fresh group 与 source post，不依赖前序
  Gate 遗留 channel。客户端只能提交 post/emoji 或 opaque react action，
  authenticated actor、绝对 reaction aggregate 与 eventSeq 均来自权威链。
- Actor/Observer 的 HTTP terminal、WS reconciliation 与 timeline Frame
  不被错误排列为固定帧序；duplicate inbound、WS-first/late HTTP、
  PersistAtomic failure 与 recovery 均由 Helix scenario 覆盖。
- PC single/dual real 和 Mobile focused real 最终都在 typed timeline 观察
  emoji count=1；失败链不会由 shell 本地补 reaction。

### Code Boundary Review

- reaction authority、幂等、persist barrier、投影与 Frame composition 只在
  Helix production `ImModule::handle` 链。PC 仅 opaque action/invoke + atomic
  Frame apply；Mobile 仅 SDK admission、whole DomainEvent batch、atomic apply
  与 typed view。
- PC production boundary 扫描 2735 文件，
  `mockProductionReachable=false`；Mobile production V1 producer/consumer
  与 raw reducer 均为 0。Fixture、Mock bridge、WDIO/GTest 注入 API 在 production
  build 不可达。

### Frame V2 Review

- 四份 Frame 均 `schemaVersion=2`、runtime `online/ready/idle`、directive
  为空。reactable timeline snapshot revision 1 无 causation；reacted patch
  `baseRevision=1 -> revision=2`，frame/mutation causation 均为 requestId。
- recovered-reacted 为可独立恢复的 revision 1 snapshot；reaction row 保留
  sent delivery、eventSeq、emoji/count 与 opaque `react` ActionBinding。
- scope、surfaceEpoch、transactionId、revision 与 actionRef 均由三端 typed
  contract 校验；PC/Mobile 不解释 actionRef 或重算 reaction。

结论：行为、代码边界、Frame V2 与证据 Review 均为 Green；等待 mandatory
after_implement loop 的 R4 evidence 判定后才可关闭 T038。

## T038 Mandatory Loop Closure

- `evidence/evidence.json` 已登记 `V-007`，绑定
  `gate-report.json` SHA-256
  `2806103c88b510472b6b38a05ea7068847c44dbe4aefb94730b83b138c8153f2`。
- mandatory after_implement：R4 evidence `PASS`；targeted convergence 检查
  FR 18、SC 6、用户故事 3、计划链与 38 项任务，未发现
  missing/partial/contradicts/unrequested gap，未追加任务。
- mandatory after_converge：`CLOSED`。G-06 现为 `review-green`，允许总控按
  顺序进入 G-07；首次 Mobile 真实链的 V1 lifecycle 等待失败已明确保留为
  修复证据，不被最终成功报告覆盖。
