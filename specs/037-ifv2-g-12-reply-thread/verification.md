# Verification Procedure: G-12

## T001-T003 Source Lock And Shared Prerequisite

- Source lock：Helix
  `533f1b654351f6708e34344aa509fcd58b81cf86`；cses-client
  `509d10cae61e66f6d0a90461daa8d1ad192ba1b8`；Mobile
  `49b94309710c511c1ffae1bf41f6a55f022fabe2`。
- Consumer pins：PC 四个 Helix Tauri dependency 当前均为
  `5e3ffc026fc8b25f76a7cce162e75734bddb45ca`；Mobile production source
  lock 为 `60b2d5f579f209a144433db1f96bef8a63f6bd5c`，bundle SHA-256 为
  `127e60902595cddb6324a10c0fe68cb94fb6cf3968e63a813914fa28785673e6`。
- Strict Matrix SHA-256 为
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
  前序 G11 ledger 为 `phase=closed`、R4=`pass`、`next_action=none`。
- 工作树边界：PC clean；Mobile 仅有既存未跟踪 bundle 与
  `tests/.DS_Store`，本 Gate 不覆盖；Helix 只有本次 Spec 指针、ledger、
  SESSION 与 G12 文档改动。
- 三端共享 fresh-group prerequisite 已存在：Helix
  `G01ScenarioRunner::create_group_prerequisite`、PC
  `prepareMockCreatedGroup`/真实建群 harness、Mobile
  `RC.createFreshGroupPrerequisite`。G12 每次 canonical/real run 必须调用该
  能力创建独立 channel/thread，不复制建群 reducer，也不复用 G11 数据。

## T004-T008 Helix Production Flow

- `cargo check -p helix-im` 最终 exit `0`。
- `g12_full_flow_test` 为 `8/8`，覆盖 fresh group、wire admission、snapshot、
  branch patch、HTTP failure、mixed authority、duplicate reply 与 detached
  recovery。
- `g12_reply_thread_interaction_test` 为 `3/3`，证明读-only G12 无伪造
  Persist barrier，terminal batch 只有完整 Frame，late/duplicate 与恢复均由
  Helix correlation/projector 决定。
- `read_relay_test` 为 `11/11`；原本期待 raw `im:read:result` /
  `im:channel:replies` 的 stale snapshot 回归已改为只接受
  `im:interaction:frame`，迟到旧 authority 为零输出。
- 生产链现为 `OutboundReplies -> normalize authority ->
  project_reply_thread_frame`：`im_get_replies` 首次产生 revision 1
  `threadDetailSnapshot`，attached `im_get_reply_branch` 产生
  baseRevision 1 / revision 2 的完整 `threadDetailPatch`。HTTP/error/mixed
  authority 不产生空成功 Frame；detached branch 保持 view-silent，后续
  snapshot 恢复。

## T009-T012 Helix Canonical Fixture

- `g12_wdio_fixture_contract_test` 从同一
  `G12ScenarioRunner -> ImModule::handle -> Effect::Emit` trace 导出三份完整
  Frame：`group-created.frame-v2.json`、`thread-snapshot.frame-v2.json` 与
  `thread-branch.frame-v2.json`。测试逐份执行 typed decode、schema 2 校验、
  thread 字段级语义断言以及 Golden byte 比较。
- 只有 `UPDATE_G12_WDIO_FIXTURES=1` 可创建或覆盖 Golden；普通测试只比较
  actual producer 与 checked-in bytes。显式更新与普通比较两次运行均为
  `1/1 PASS`、exit `0`。
- `provenance.json` 固定 producer commit
  `a2207d73f8e72da7374b916b2167ee3a25b9f6c3`、fixture id、schema、
  surface inventory、raw/semantic SHA-256、动态替换与 revision rebase 白名单。
  `jq` inventory/schema/hash 校验为 `true`。
- canonical raw SHA-256：group-created
  `5753a7e4e6bad20f2b46d345f6483b2600bd10f5cfccb16aa17200d54aaf0ab6`；
  thread-snapshot
  `434207276732e4076ddacb206731187e8031a79e25240ce3a3d66e300a2e7c37`；
  thread-branch
  `6a11b920460c031129c28b0bff18303fc4f3e56fa4653da28fe065646755c6e2`。
- Helix Green 命令：`cargo check -p helix-im`、`cargo test -p helix-im
  --test g12_full_flow_test -- --nocapture` 与 `cargo test -p helix-im --test
  g12_wdio_fixture_contract_test -- --nocapture` 均为 exit `0`。跨端 mirror
  尚未完成，因此这里只关闭 Helix producer/Fixture 阶段，不提前标记 G12
  review-green。

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-12` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。
