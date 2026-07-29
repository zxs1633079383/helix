# Verification Procedure: G-15

## Source Lock

- Helix baseline HEAD：`836898c8ed4d09eea39f365defef53c062df5723`。
- cses-client baseline HEAD：`01ee73c430c7ab39ab14411b4d0eae7f8b361950`；production Tauri Helix pin：`157daca3760e9c94d7b6bc9f38798e199628b1aa`。
- Mobile baseline HEAD：`afa41cd8d2dd6567a42d258c3ba504a0210f1125`；`contract/helix-source.lock.json` 是 production source pin 真源，G-15 实现阶段只在 canonical producer commit 后统一 repin。
- Strict Matrix `UC全链路发布矩阵-v1.md` SHA-256：`84b9cc6ce2bd3c7ffe4595371354eb609517b89902228bd27cfb93e85b80c770`。
- 前序 G-15a：`specs/041-ifv2-g-15a-create-channel/review-ledger.json` 为 `phase=closed`，R1-R4 全 PASS，event sequence 32。
- 工作树边界：PC clean；Mobile 仅有用户既有未跟踪 bundle 与 `tests/.DS_Store`；Helix 仅有本 Gate 的 feature/session/loop 状态文件。
- 本节锁定的是实现前 baseline，不冒充最终 producer/consumer pin；T012/T022/T032 将分别登记最终提交与验证证据。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-15` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## Helix Producer Green

- producer commit：`064f814a0f4fc4df8b9299652d0e615f5857b875`。
- `cargo check -p helix-im`：退出码 0。
- `g15_full_flow_test`：6/6 PASS；覆盖业务失败、基础设施失败、relation 持久失败、迟到回包、happy path 与 summary recovery。
- `g15_topic_interaction_test + uc15_16_projection_test`：13/13 PASS；证明 terminal 不再发布 raw `im:channel:replies`，且 typed thread 只能在 root durable readback 后产生。
- `g15_wdio_fixture_contract_test`：1/1 PASS；普通模式只比较 Golden，显式 `UPDATE_G15_WDIO_FIXTURES=1` 才生成两份 canonical Frame 与 provenance。
- `topic-thread-created.frame-v2.json`：raw SHA-256 `c6f9a96ee2ded44462b11c68760ed75a59fafcfea371b5d70edda2cff7eeb734`；semantic SHA-256 `4074e60b2d81de7c653b2abaf969c76399391129a3b0e6d255899b4175f9cc79`。
- `channel-summary-recovery.frame-v2.json`：raw SHA-256 `13ba2758d5c9d237376d3450f46b70379426518d298995cfc8ae1200ca1edf84`；semantic SHA-256 `fa5a165019a7eac4b8180aba1364d5d9c213e7c83ca1228479a2214204e88184`。
- `ws-topic-dialog.frame-v2.json`：raw SHA-256 `3ac61c8b5b552f8a4be69e1b881efbc0b5b7cf4d8bf4f755b16b06e2bed88454`；semantic SHA-256 `f7cff76dcec9decb1dae5a6c75c41e975a3abddb370d66995d197c301baa15c4`。
- `ws-topic-created.frame-v2.json`：raw SHA-256 `3924220e80418b39b04f2599f0db067f0ba114a6fd269cb904bfbf729ac01421`；semantic SHA-256 `dd03e4fd1d179dd57bfb075afe25c98938dfef7a5d2147160da65eca9f4659c1`。
- 两条 correlation 独立：HTTP/Persist/readback 发布 `message.thread-detail`；`channel_created` 发布 dialog 与 topic summary/roster，storage recovery 独立发布 `message.channel-summary`。测试不假设固定第三帧。

## PC Mock / Real / Report Green

- consumer commit：`88bdcd22169c151a321343cea73209952ec16dab`；生产 Tauri Helix pin：`064f814a0f4fc4df8b9299652d0e615f5857b875`。
- Mock run `mock-20260726195759933-47689`：3/3 PASS、exit 0、retry 0；分别覆盖 HTTP thread 早到后的 scope recovery、WS dialog 先到竞态与 admission failure 可重试。
- single-real run `single-real-20260726200838621-56221`：1/1 PASS、exit 0、retry 0；从 MessageV4 创建 fresh group、发送 root、创建话题，经真实 Tauri invoke、Helix/SQLite、`/api/cses/posts/makeTopic`、WS、FrameApplied 与 DOM 闭合。
- dual-real run `dual-real-20260726201156870-59140`：1/1 PASS、exit 0、retry 0；Actor 与 Observer 最终得到同一 topic channelId，Actor thread Frame 的 causationId 与 admission reqId 一致。
- report run `report-20260726201234835-59708`：1/1 PASS、exit 0、redaction PASS。
- G15 focused Jest：47 PASS；Angular shell/topic tests：14 PASS；TypeScript、production boundary 与 WDIO build 均 PASS。
- Fixture 不是 command 同步返回值；`mockAccepted()` 只返回 AcceptedCommand，`replay()` 才把完整 Frame 注入 `im:__bus__`。Angular 只在 dialog Frame 揭示 topic scope 后发起 thread recovery query，不计算 topic channel、root relation、权限或终态。

## Mobile Contract Green

- consumer commits：`477e156d6f4b85caeebaf6fd80bd50204c4f0792`（mirror/source pin）与 `c5896e16b3ce3a050b5d15b925c48415c39501b0`（生产 SDK/QuickJS/typed view）。
- production Helix pin：`02dd062b85fceb6489e8d7f9659af60e55ed86ec`，其 ancestry 包含 canonical producer `064f814a0f4fc4df8b9299652d0e615f5857b875`；增量 bundle SHA-256 `44a98e073fbb0ee1500ae9d485e94dbcbe496b6feb8f4fe2da4ab7e5630f9886`。
- `G15FrameContractTest`：6/6 PASS；覆盖完整 envelope、HTTP/WS 双 correlation 任意顺序、summary recovery、schema/surface/revision fail-closed、生产 DomainEvent batch、C ABI 与 QuickJS typed view。
- G15/SDK/QuickJS focused：12/12 PASS；`im_make_topic` 只进入 `root_id,user_ids,display_name?,req_id`，旧 `picture/channel_id/topic/team_id/self_id/member_ids` 不进入生产 admission。
- `run-uc01-16-gtest.sh --only UC15`：10/10 PASS；证明共享 fresh-group prerequisite 与 UC15 生产链接可用。该 lane 的真实操作覆盖 G15a 前置，不冒充 Mobile G15 real；Mobile G15 的 required 证据边界是 production-linked Contract GTest。
- source-pin、G15 fixture parity、legacy hard-cut 与 Strict Matrix：全部 PASS；production V1 producer=0、consumer=0，`im.store`、raw reducer、测试注入 ABI 默认构建不可达。
- QuickJS 接线被修改，因此执行共享 `run-venus-g01-frame-gate.sh`：PASS；未新增 G15 专属 Venus reducer 或 Gate。

## Three-repo Fixture Parity

| Artifact | Helix / PC / Mobile raw SHA-256 | Count |
|---|---|---:|
| `channel-summary-recovery.frame-v2.json` | `13ba2758d5c9d237376d3450f46b70379426518d298995cfc8ae1200ca1edf84` | 3 |
| `topic-thread-created.frame-v2.json` | `c6f9a96ee2ded44462b11c68760ed75a59fafcfea371b5d70edda2cff7eeb734` | 3 |
| `ws-topic-created.frame-v2.json` | `3924220e80418b39b04f2599f0db067f0ba114a6fd269cb904bfbf729ac01421` | 3 |
| `ws-topic-dialog.frame-v2.json` | `3ac61c8b5b552f8a4be69e1b881efbc0b5b7cf4d8bf4f755b16b06e2bed88454` | 3 |
| `provenance.json` | `be1cbec9a648e2b9ed5700a4a9c211009ba08cc5d378ea8e32f4885b4962e90f` | 3 |

- inventory、schemaVersion=2、surface inventory、raw SHA 与 semantic SHA 三端一致率 100%。
- 动态替换只允许 provenance 声明的 identity/correlation 字段与 runtime revision rebase；PC/Mobile 未补业务字段。

## R4 Review

### Behavior

- PASS：权限与 actor 身份由 Helix command/authority 校验；平台不传 owner、role、picture 或 topic relation。
- PASS：HTTP terminal 仅在 relation Persist Ok 与 readback 后发布 thread；业务/基础设施/persist failure 均不发布伪成功 Frame。
- PASS：HTTP thread 与 WS dialog/channel-created 是独立 correlation，可任意竞态；迟到或重复结果幂等，不要求固定第三帧。
- PASS：cold summary recovery 使用独立 snapshot；Actor/Observer PC real 最终收敛到相同 topic channelId。

### Code Boundary

- PASS：业务 reducer 只存在于 `helix-im` 的生产 `ImModule::handle -> Effect -> PortReply/Inbound -> Effect::Emit` 链。
- PASS：PC Fixture/Mock/WDIO API 未进入 production bundle；Angular 仅 attach/recovery/apply/render。
- PASS：Mobile GTest 链接生产 SDK、Bridge、SurfaceRuntime 与 QuickJS binding；FFI 保持 batch crossing，未逐 mutation callback。
- PASS：Mobile 默认 production build 不注册 `im.store`、raw business reducer、test injection 或 characterization-only ABI。

### Frame V2

- PASS：四份 Frame 均含 `schemaVersion=2`、非空 `transactionId`、完整 runtime 与 directives 数组。
- PASS：thread scope 冻结 `topic channelId + rootMessageId`，detailContext 冻结 parent/topic/root/title；dialog row 冻结 `channelKind=topic + rootMessageId`。
- PASS：WS dialog patch 为 revision 2/baseRevision 1；其余 snapshot 为 revision 1/baseRevision null；unknown schema/surface/gap 原子拒绝。
- PASS：HTTP terminal causationId=`req_id`；WS dialog 可无 causation；channel-created 保留 req correlation；actions/directives 为空是生产事实而非平台补省略。

### Evidence

- PASS：required lanes Helix、Fixture、PC Mock、PC Real、Mobile Contract、production boundary 与三项 Review 均有独立证据；fail=0、skip=0。
- 非阻断提示：`g15_topic_interaction_test` 的独立 test binary 对共享 ScenarioRunner 的 WS helper 报 dead-code warning；该 helper 在 `g15_full_flow_test` 中真实执行，不影响行为或合同。
