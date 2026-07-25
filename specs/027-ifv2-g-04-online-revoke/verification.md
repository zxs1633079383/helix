# Verification Procedure: G-04

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-04` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`bc03fb0d13c7062ada71467fc6ae14ea861d31ef`；前序 G03a 子
  Feature 已完成 T001-T038，R1-R4 `PASS` 且最终 `CLOSED`。
- cses-client：`08c1843b7b7751029b6fb9b40ec54eb122f646d0`，clean；当前 Tauri
  Cargo pin 为 `41ef427500757ecd627b337a156cb45818dcfc84`。
- Mobile：`7bc78ef0b2510232a182b9999413bfe1a1bebe35`；仅保留进入任务前已有
  bundle 与 `tests/.DS_Store` 未跟踪项。生产 source pin 为
  `af3fee1e2e9735d9cd9574a525e05b9d36333d58`。
- Strict Matrix raw SHA-256：
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- G04 当前已有生产 `im_revoke`、WS `posts_update` PersistAtomic 和 timeline
  Recalled 投影，但没有本轮要求的完整 ScenarioRunner、canonical Fixture、PC
  独立 WDIO lane 或 Mobile Frame V2 contract；只满足实现准入，不声明 Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用公开 `G01ScenarioRunner::create_group_prerequisite`，先从生产
  `Tick::Command -> HTTP PortReply -> Persist/Emit` 取得权威 channel，再发送一条
  可撤回消息作为 G04 目标。
- PC 复用 domain-neutral `prepareMockCreatedGroup` 与真实 MessageV4 建群/发消息
  操作；G04 helper 不复制前序 Gate marker 或成功条件。
- Mobile 复用 `createFreshGroupPrerequisite`，调用生产 `im.channels.create` 并等待
  权威 `im:channel:created`；不复用 G03a 遗留群或失败消息。
- 三端 prerequisite 已由 G01-G03a 闭环持续验证，本 Gate 只复用合同，不修改共享
  生产业务逻辑。
