# Verification: InteractionFrame V2 双仓硬切

## Phase 1 Source Lock

**Captured**: `2026-07-23T03:52:13-07:00`

| Repository | Source branch | Source HEAD | Source worktree state | Isolated branch | Isolated path |
|---|---|---|---|---|---|
| Helix | `main` | `3b430c6cb1cd4af615d487dde2c9788b7cfda257` | dirty: `SESSION.md`、WS parser/tests、G01 V2 流程图、loop ledger | `codex/018-interaction-frame-v2-cutover` | `/System/Volumes/Data/workspace/rust/helix/worktrees/018-interaction-frame-v2-cutover` |
| cses-client | `feat/helix-reactor-im` | `8d71085453c62b966d82b6c947f68a20cc638e3f` | dirty: message-v4 runtime view/content/SCSS 六项 | `codex/018-interaction-frame-v2-cutover` | `/System/Volumes/Data/workspace/angular/cses-client/worktrees/018-interaction-frame-v2-cutover` |
| cses-im-server | `main` | `6ddb5245364128732fea1c1ea48977001394a76f` | untracked `=`；本 Feature 默认只读 | 不创建 | `/System/Volumes/Data/workspace/golang/cses-im-server` |

两个隔离 worktree 创建后均为 clean；源工作树 branch/HEAD 前后未变化。所有实现、测试和提交只在隔离分支发生，除非最终 pin/验收步骤明确要求回写。

## Frozen Inputs

| Input | Value |
|---|---|
| cses-client `helix-core` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-im` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-driver-host` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-driver-native` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| source `src-tauri/Cargo.lock` SHA-256 | `a789269b5fc2edd1a09c6bb3114b60a2658b7e76fb097017a4c7fea6a43ed6f9` |
| `strict-matrix.v1.json` SHA-256 | `d3461beb760255f6ae733e3b5f7ceccd32dca22faf8adf0c76c29e0d6786b345` |
| InteractionFrame fixture manifest SHA-256 | `71c281e5879ecf2d61f71363777f7b2b7915170403d33a0faff8964fcf81b1da` |

`Cargo.lock` 是 cses-client 源工作树的本地 ignored artifact，隔离 worktree 初始不存在；首次 Native 构建生成的新 lock 必须作为候选证据重新 hash，不能把上述 source hash 当作最终 pin。

## Phase Status

| Task | Status | Evidence |
|---|---|---|
| T001 | PASS | 三仓 branch/HEAD/dirty、四包 pin、lock/matrix/fixture hash 已记录 |
| T002 | PASS | [discovery.md](discovery.md) 已按当前锁定源码更新 V1/合法独立 v1 inventory |
| T003 | PASS | `python3 scripts/verify-interaction-frame-v2-index.py` 返回 19/19/19、25 Gate、T021-T077 完整 |
| T004 | PASS | [quickstart.md](quickstart.md) 固定当前可执行 focused、样式构建、Native、双库只读入口与 artifact root |
| T005 | PASS | Helix/cses-client 隔离 worktree 已创建且源 ref 前后不变 |

## Required Runs

后续 V-001 至 V-008 的原始输出必须写入 `evidence/`，并通过 `loop_engine.py record-evidence` 登记 hash。失败输出不得被摘要文本覆盖。

## Phase 2 Shared V2 Foundation

**Completed subset captured**: `2026-07-23T05:35:00-07:00`

| Task | Status | Evidence |
|---|---|---|
| T006 | PASS | Helix V2 contract + V1 fail-closed tests；`interaction_frame_contract_test` 17/17 PASS |
| T007 | PASS | client V2-only codec + V1/unknown rejection；focused Jest PASS |
| T008 | PASS | epoch/stale/atomic/directive/ack runtime tests；focused Jest PASS |
| T009 | PASS | 25-Gate profile、v1/v2/raw/ack counter 与 candidate/evidenceEpoch 隔离测试 PASS |
| T010 | PASS | Helix production writer 唯一输出 InteractionFrame V2；commit `ea415a800b329947ce0757b8595df19b715f7d55` |
| T011 | PASS | client production codec 仅接受 schemaVersion=2，无 V1 normalize/union |
| T012 | PASS | client slot state 纳入 surfaceEpoch，epoch/revision 整帧 staged swap |
| T013 | PASS | Angular → Tauri → Helix private `FrameApplied` typed command，content-free ack tests PASS |
| T014 | PASS | Helix matrix epoch 11、`InteractionFrame/v2`、fixture producer pin 与 client mirror/hash 对齐 |
| T015 | PASS | Native `interaction-frame-v2-cutover` profile 精确选择 25 Gate，排除 G03/G03a |
| T016 | PASS | 双 actor content-free Frame probe + reducer fail-closed counters 已接通 |
| T017 | PASS | Sass 编译、style-loaded、444/678 overflow/遮挡与双用户截图 gate 可执行 |
| T018 | PASS | 25 Gate PG/SQLite identity/delta/privacy 合同及机器校验 PASS |
| T019 | PASS | action/frame/ack correlation safe trace 已补齐；不记录 token、cookie、正文或 VM |
| T020 | PASS | V-001 至 V-004 全绿；SharedV2Ready 成立，详见下表 |

### Formal Revisions And Hashes

| Artifact | Value |
|---|---|
| Helix V2 producer commit | `ea415a800b329947ce0757b8595df19b715f7d55` |
| Helix formal pin (matrix + fixture manifest) | `d899a6033c90a153b5b009a62bee0e3a17dd9ad8` |
| cses-client V2 consumer commit | `11bd41b1b3a540ee26d6a95ab4b0468d929d98f5` |
| client generated `Cargo.lock` SHA-256 | `c6aceb0b8b5e6c9f91d2ca17f47b240417c0a5dab97d00925ec0c32b1f119f95` |
| client fixture manifest SHA-256 | `572914e80235227a690ffefeafcc2e5280f5e55e6807b26e3a824f6f010c4447` |
| client generated Strict Matrix TS SHA-256 | `f49fe63be8005d1bab7446f7a8315e6272cb0d703f57cec98fe62a67d5318582` |

### Verification Runs

| Command | Result | Wall Time / Notes |
|---|---|---|
| `cargo test -p helix-im --quiet` | PASS | 全量 lib + integration；约 26s；ignored tests 保持原状态 |
| focused Helix V2 contract/manifest/projection tests | PASS | 68/68；1.7s |
| client codec/runtime/action/parity/store/shell focused Jest | PASS | 7 suites、71 tests；13.1s |
| client runtime/shell post-ack focused Jest | PASS | 2 suites、33 tests；17.0s |
| `npx tsc -p src/tsconfig.app.json --noEmit` | PASS | isolated worktree 复用 ignored `node_modules` 与 `release.local.json` |
| `cargo check --manifest-path src-tauri/Cargo.toml` | PASS | 首轮因 isolated worktree 缺 ignored staged resources/dist 失败；补齐同源 ignored artifacts 后通过；既有 warnings |
| focused Native `frame_applied_queues_the_content_free_private_command` | PASS | 1/1 |
| `npm run test:message-v4:strict-contract` | PASS | epoch 11；Helix pin/hash 与 generated consumer 一致；Cargo.lock matched |

Phase 2 已完成，`SharedV2Ready` 成立；这只允许进入逐 Gate READY 工作，不等于
Native/PG/SQLite/DOM 全链路已经通过。

### SharedV2Ready Final Run

**Window**: `2026-07-23T04:58:00-07:00` 至 `2026-07-23T05:10:51-07:00`

| Validator | Command | Result | Wall Time / Artifact |
|---|---|---|---|
| V-001 | `cargo test -p helix-im --test interaction_frame_contract_test --no-default-features` | PASS，17/17 | 约 5.4s |
| V-002 | `cargo test -p helix-im` | PASS，全量 lib/integration | 约 27s |
| V-003 | `npm run message:command-api:gate && npm run message:projection-ownership && npm run message:surface-runtime:source-pin` | PASS，epoch 11，formal pin `d899a603...`，fixture 9/9 | 7.6s |
| V-004 | focused Jest + desktop E2E Jest + reducer Playwright + desktop TS + Sass | PASS，59 + 18 + 35 tests；TS clean；CSS hash 固定 | Jest 7.4s；其余见命令输出 |
| Native Rust | `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo check` | PASS，修正 safe-trace borrow 后一次完整复验 | 2m53s；既有 warnings |
| Index | `python3 scripts/verify-interaction-frame-v2-index.py` | PASS，19 Spec + 19 Plan + 19 Tasks + 25 Gate + T021-T077 | <1s |
| DB manifest | `python3 scripts/verify-interaction-frame-v2-evidence-manifest.py` | PASS，25 Gate contracts | <1s；manifest SHA-256 `18daacbf...` |

关键 artifact SHA-256：

- client `frame-evidence.ts`: `aa9765d2842ae6009ea777dd0ca9d5488c5653dbbda9a93597a10556a30ae04e`
- client `reduce-evidence.mjs`: `9a27accc9251998adaeb620197e03c52d8b04e8391c2909919968fcb4e73ba6a`
- compiled message-v4 CSS: `29b7ee8fe1066a1521eed0ed0e5b7c1dd78cc736dcce0318c6697777de07ed5f`
- evidence manifest: `18daacbf1736e01f24c4bdf5e1a5e9d13d10ba25e7b0ddbde07293e07eef60da`
- manifest verifier: `9bb871e31823f60c2df8b724fcde1690a98e8bd25a9362bceddb46d9f281221d`

`SharedV2Ready` 仅表示共享协议、工具和证据合同可进入逐 Gate 实现；尚未产生
25 Gate 的真实 PG/SQLite/双用户 DOM/截图证据，V1 删除仍被 T079-T080 硬阻塞。

## Phase 3 Focused Gate Readiness

**Captured**: `2026-07-23T05:18:00-07:00`

T021-T077 对应的生产实现与 RED/回放测试在本 Feature 创建前已经存在；本轮没有
复制状态机，而是用 V2-only writer/consumer、统一 evidence manifest 和下列 focused
回归重新收口。`READY_FOR_FINAL_E2E` 只表示可以进入同一 Native candidate，不能替代
真实 Go、PG、SQLite、FrameApplied、DOM 与截图证据。

| Dimension | Helix focused contract | client focused contract | Status |
|---|---|---|---|
| GS01 | `gs01_04_v2_sync_recovery_test` | `gs01-04-sync-recovery-dom.test.ts` | READY |
| GS02 | `gs01_04_v2_sync_recovery_test` | `gs01-04-sync-recovery-dom.test.ts` | READY |
| GS03 | `p4_sync_scheduler_test` + GS recovery | `gs01-04-sync-recovery-dom.test.ts` | READY |
| GS04 | `uc45_unknown_channel_post_test` + GS recovery | `gs01-04-sync-recovery-dom.test.ts` | READY |
| G01 | `send_test` + `f1_send_http_reconcile_test` | composer/runtime/view focused suites | READY |
| G02 | InteractionFrame codec/content contracts | typed document view test | READY |
| G04 | `g04_revoke_interaction_test` | runtime view/action contracts | READY |
| G05 | `g05_forward_interaction_test` | runtime view/forward contracts | READY |
| G06 | `g06_reaction_interaction_test` | runtime view/quick-action contracts | READY |
| G07 | `g07_urgent_interaction_test` | runtime view/urgent contracts | READY |
| G08 | `g08_schedule_create_interaction_test` | runtime view/schedule contracts | READY |
| G09 | `g09_schedule_cancel_interaction_test` | runtime view/schedule contracts | READY |
| G10 | `g10_timeline_window_interaction_test` | runtime view/anchor contracts | READY |
| G11 | `g11_locate_interaction_test` | runtime directive/view contracts | READY |
| G12 | `g12_reply_thread_interaction_test` | runtime view/thread contracts | READY |
| G13 | `g13_channel_read_interaction_test` | `g13-channel-read-interaction.test.ts` | READY |
| G14 | `g14_receipt_template_interaction_test` | `g14-receipt-template-interaction.test.ts` | READY |
| G15 | `g15_topic_interaction_test` | `g15-topic-interaction.test.ts` | READY |
| G16 | `g16_governance_interaction_test` | `g16-governance-interaction.test.ts` | READY |

### Focused Results

- Helix 单次 18-target Gate 回归：59 tests PASS，0 fail。
- client 单次 8-suite Gate/DOM 回归：首次暴露 G14 收据 Surface 未正式渲染、
  G16 close grant 未进入 menu 两个缺口；修复后 8/8 suites、38/38 tests PASS。
- G14 forbidden Frame 不再输出成员 identity，template received 与尚未接入的
  business confirmation 分开呈现。
- G16 只渲染 Helix enabled 的 close/leave grant；settings 保留现有只读 shell
  入口；disabled leave 不出现在 DOM。
- formal Helix pin 仍为 `d899a6033c90a153b5b009a62bee0e3a17dd9ad8`，
  Strict Matrix epoch 11，fixture/source-pin 9/9 一致。

**Verdict**: `19/19 READY_FOR_FINAL_E2E`。下一硬门是 T079-T080 的首次
`V2OnlyFirstRun`；在其 25/25 产品链证据全绿前，V1 deletion admission 仍为 DENY。

## Phase 4 V2OnlyFirstRun Attempt

**Window**: `2026-07-23T13:11:57.600Z` 至 `2026-07-23T13:13:08.516Z`

| Item | Result | Evidence |
|---|---|---|
| isolated Native origin | PASS | desktop-e2e 从 `http://127.0.0.1:14201` 启动，Tauri event ACL 可用，启动阶段已观察 `__helix_frame_applied` |
| correlation namespace | PASS | request id 从错误的 `<run>-444-444` 收口为 `<run>-444`；Go evidence 能按 exact reqId 命中 |
| shared channel setup | PASS | `im_create_channel` HTTP 200；`channel_created` 对 444/678 均为 verified delivery；PG readback 有 channel + 两成员 |
| G-S1 message transport | BLOCKED | `im_send_message` HTTP 200，但 Go body 等待 5s 后返回非 SUCCESS，未产生 `post` WS |
| Frame/SQLite/DOM continuation | NOT RUN | 缺 exact `post` WS，fail-closed 停在 Go broadcast boundary，未把后续面伪记为通过 |

原始输出：[V-005-native-first-run-blocked.json](evidence/V-005-native-first-run-blocked.json)，
SHA-256 `11bb53b8a38fab14be7b3e79c86b847415c6d7b062d2be7850afe073078136e9`。
双端 host 日志位于同一 candidate artifact root：
`tests/e2e/.artifacts/mrc-frame-prereq-1784812304909/{444,678}/host.stdout.log`。

Go 日志在同一时间窗给出：

- `posts/create`：HTTP 200、146 bytes、`5.00072325s`，exact track id
  `mrc-frame-prereq-1784812304909-444:im_send_message:3`。
- Helix consumer：`posts/create reply status != SUCCESS, not advancing`。
- 环境：`telepresence status` 为 User/Root daemon not running、Traffic Manager not connected；
  x9/gRPC trace 报 `name resolver error: produced zero addresses`。

已按 HX-C016 尝试 `telepresence connect`；本机 sudo 凭证不接受历史记录的四空格，
因此未修改用户 GoLand 进程，也未继续执行会破坏证据边界的 25 Gate。cses-client
修复提交为 `b37bc0eb2`，focused Jest 20/20、SurfaceRuntime 21/21、desktop TypeScript
compile 和 Rust origin 单测 1/1 均通过。

**Task verdict**:

- T079：`BLOCKED_EXTERNAL`，尚未完成。
- T080-T096：`PENDING`，不得标 `[X]`。
- V1 deletion admission：`DENY`；T081-T084 不得提前执行。
- after_implement R4：`BLOCKED`，原因 `evidence manifest has no PASS run`。

## Phase 4 G-S3 Resume Attempt

**Window**: `2026-07-23T09:28:08-07:00` 至约 `2026-07-23T09:30:24-07:00`

| Item | Result | Evidence |
|---|---|---|
| Helix sync dialog readback | PASS | `2300059` 将每批全表 channel Scan 收敛为目标 channel Get；`cargo test -p helix-im` 全量退出码 0 |
| Native candidate | PASS | cses-client pin `2300059`；`npm run message:desktop:build` 通过，耗时 1m25s |
| G-S3 baseline command/Go | PASS | exact reqId `mrc-6e8aa0e7bb6f8073-g-s3-444:im_send_message:8`；Go post `iis1a6dsm7f8xgf474yrb5qjoh` |
| remote PG readback | PASS | `public.post` 按 id/channel/marker 精确回读 1 行，`create_by=444` |
| observer SQLite readback | PASS | channel `hn95kg8jybyg5r46c9yp6tn81r` 有 seq=2 baseline message；cursor=2；channel last_post 为同一 marker |
| V2 authority counters | PARTIAL | `v1Frames=0`、`rawBusinessFallbacks=0`；`v2Frames=192`、`appliedV2Frames=181` |
| observer DOM | BLOCKED | 失败截图显示 dialog preview 已是 baseline，但已选中内容区仍为“等待权威快照”，timeline message 未形成可断言 DOM |
| heartbeat target phase | NOT RUN | baseline DOM setup fail-closed；场景未暂停 observer，也未发送目标 marker |

本次 raw artifact：
`tests/e2e/.artifacts/mrc-message-v4-1784824070406/scenarios/g-s3/result.json`。
observer 失败截图：
`tests/e2e/.artifacts/mrc-message-v4-1784824070406/678/scenarios/g-s3/failure-678.png`。

失败账号的 observer SQLite 当前有 `1274` 个 channel。启动恢复仍产生大量逐 channel
dialog V2 patch；虽已消除逐批全表 Scan，但 SurfaceRuntime 只应用 `181/192` 个采样
Frame，最终 timeline authority snapshot 没有到达 DOM。G-S3 场景同时已修正为记录
baseline reqId，并要求目标 `im_send_message` 排除该 reqId，避免再次把 baseline 当成
目标发送证据。

**Task verdict**:

- T079：仍为 `BLOCKED_RUNTIME`，首个 Red Gate 为 G-S3 `dom-projection`。
- T080-T096：保持 `PENDING`；不得生成 25/25 或删除 V1。
- V1 deletion admission：继续 `DENY`。
- 下一步：按 loop engine `event_seq=20` 进入 `speckit-converge`，把恢复批次 Frame
  洪峰与 timeline authority 未消费补成新的 append-only convergence tasks。

## Phase 6 Post-deletion Final Run

**Window**: `2026-07-24T11:10:51.410-07:00` 至 `2026-07-24T11:16:17.298-07:00`

| Item | Result | Evidence |
|---|---|---|
| candidate identity | PASS | Helix `57d2ddab8c8095877a769988751ebefff61bddfb`；cses-client `846afd3fa`；Go `3f43d28`；candidate `frame-v2-post-v1-delete-57d2ddab-846afd3fa-go-3f43d28` |
| evidence epoch / isolation | PASS | epoch `post-v1-delete`；run `mrc-frame-v2-final-25-1784891421`；reducer `artifactIsolation.pass=true` |
| Helix V1 physical deletion | PASS | V1 contract、producer、writer 与兼容入口已物理删除；`cargo test -p helix-im` 全量通过 |
| cses-client V1 physical deletion | PASS | V1 consumer、normalizer 与兼容分支已物理删除；Native transport 4/4、focused Jest、build 与 sync contract 通过 |
| focused regression | PASS | desktop UC 3 suites / 22 tests；SCSS/CSS style gate；strict contract 与四仓 pin lock 全部通过 |
| second Native run | PASS | GS1-4、G1-2、G4-16 共 25 个场景，runtime 25/25 PASS，canonical 25/25 Green，fail=0，skip=0 |
| V2-only protocol | PASS | `v1Frames=0`、`rawBusinessFallbacks=0`、`v2Frames=478`、`appliedV2Frames=478`、`frameAppliedAcks=478` |
| ordered evidence digest | PASS | 25 个 `result.json` 有序 hash-list SHA256 `3c37df02f80072d4b9769a9e5c0b0167686b0251a52cfcced1e70f435c88c136` |
| dual-account visual evidence | PASS | G01 账号 444 截图 SHA256 `c165e5a0d140fac939c0e4ce9bf02cc932a7e19b8d93cc7d46abb4d7357e3517`；账号 678 截图 SHA256 `4e93cb9b59831bb66189fed1649f85b1342938c99b2913818b920dc435c073e2` |
| dual-end/storage evidence | PASS | 场景原生断言逐项关联 Go/Helix/client 日志、真实 profile SQLite 与远端 PG；25 个 `result.json` 全部判定 PASS |
| strict facet reducer | DIAGNOSTIC | 7 Green / 10 Partial / 8 Blocked；`frameProtocol.pass=true`。依用户明确验收口径，该统计只诊断 facet schema/correlation，不覆盖场景原生断言 verdict |

最终命令使用 `CSES_E2E_PROFILE_SEED_ROOT=<post-v1-synced-profile-root>` 作为真实 SQLite
启动输入；它不继承旧 run 的 `result.json`、evidence verdict 或 reducer 结果。删除后 run
使用独立 artifact root：

`/System/Volumes/Data/workspace/angular/cses-client/worktrees/018-interaction-frame-v2-cutover/tests/e2e/.artifacts/mrc-frame-v2-final-25-1784891421`

本轮修复了三处 Native harness 问题：重启后的 profile/correlation 顺序、初始 sync
必须同时具备 ready Frame 与非空 dialog、G02 actor DOM 使用服务端权威 message id。
同时确认旧 Go 进程仍要求 legacy `users`，当前 Go `3f43d28` 的 canonical create
请求使用 `userIds`；重编当前源码后 targeted G-S1/G02/G08/G09 与全量 25 场景均通过。

远端链路未依赖 Telepresence：Go、PG、Pulsar、ES 与 Paas 使用 staging NodePort，
本地独立 Redis 仅承载当前候选运行态。账号口令未写入文档或 artifact 索引。

**Final verdict**: `CLOSED`。T092-T096 完成；V1 已下线且删除后 Native 25/25、
V2-only counters 与后续 focused regression 均通过。

## Phase 7 Final Capability Residual Removal

Phase 6 后的 convergence 生产源码扫描发现 `helix_capabilities.features` 仍宣告
`leftDialogListFrameV1=true`。它没有消费方，但仍属于生产 ABI 的 V1 残留，因此在
Helix `32e3a0659e4f29add1284be2ba1972eb28f0f938` 中物理删除，并把 cses-client
四个 Helix crate pin、Cargo lock 与生成合同统一更新到该 revision。

**Window**: `2026-07-24T11:33:53.072-07:00` 至 `2026-07-24T11:39:13.901-07:00`

| Item | Result | Evidence |
|---|---|---|
| final candidate | PASS | `frame-v2-post-all-v1-delete-32e3a065-client-pin`；epoch `post-all-v1-delete` |
| pinned source identity | PASS | Helix `32e3a0659e4f29add1284be2ba1972eb28f0f938`；cses-client `a4770d6d62a1124e29b2ea08d10ebb3017bd8889`；Go `3f43d28` |
| final suite | PASS | `mrc-frame-v2-final-all-v1-deleted-1784892804`；单一 topology 连续执行 25 场景 |
| scene-native verdict | PASS | runtime 25/25 PASS；canonical 25/25 Green；fail=0；skip=0 |
| final V2-only protocol | PASS | `v1Frames=0`、`rawBusinessFallbacks=0`、`v2Frames=482`、`appliedV2Frames=482`、`frameAppliedAcks=482` |
| artifact isolation | PASS | reducer `artifactIsolation.pass=true`，failure=0 |
| strict facet diagnostic | DIAGNOSTIC | 7 Green / 10 Partial / 8 Blocked；不覆盖场景原生 verdict |
| final G01 screenshots | PASS | 444 SHA256 `292451a56b029a20037a437f572f5935b6927fd9e425b5a40339102473fdab7a`；678 SHA256 `d3584782ca6aa67db79e31157509ad9fed1a263d1fa67f2b05e10260f7e8ad30` |
| final pin/contracts | PASS | Native build、strict contract、surface-runtime source pin 与 Cargo lock 均匹配 `32e3a065` |
| final V1 static scan | PASS | 生产源码 `InteractionFrameV1`、`FrameV1`、`leftDialogListFrameV1`、`normalizeLegacyFrame`、`rawBusinessFallback` 均为 0 命中 |

最终 artifact root：

`/System/Volumes/Data/workspace/angular/cses-client/worktrees/018-interaction-frame-v2-cutover/tests/e2e/.artifacts/mrc-frame-v2-final-all-v1-deleted-1784892804`

首次尝试因漏传 desktop-e2e 专用 `CSES_E2E_IM_HOST`，Native 按编译期地址连接并
fail-closed；补齐该真实 Go host 后先跑 G-S1 smoke，再完成上表 25/25。失败尝试没有
进入最终 evidence manifest。

**Final verdict**: `CLOSED`。所有 InteractionFrame V1 生产/消费/能力声明均已物理删除，
最终 pin 候选删除后再次通过 Native 25/25 与 V2-only counters。
