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
