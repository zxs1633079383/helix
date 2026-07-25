# Verification Procedure: G-03a

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-03a` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`a0e897c2164744dd5d88f666c904b8c8d13de63b`；进入 G03a 前
  G02 子 Feature 已 R4 `PASS` 并最终 `CLOSED`。
- cses-client：`39695bbd00d978d619505a79560de8af2e89631e`，clean；当前
  Tauri Cargo pin 为 G02 producer
  `ac15abd0b29ef62ea5d359fb22030800dd0dee2b`。
- Mobile：`6730b5be8d11e334ef632a986173598f18ec9e9a`；仅保留进入任务前已有
  bundle 与 `tests/.DS_Store` 未跟踪项。生产 source pin 为
  `1774d5a38e694b4a56e2b85ea9055c3e825f2222`。
- Strict Matrix raw SHA-256：
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- G03a 当前没有 Helix canonical Fixture、PC WDIO lane 或 Mobile V2 Frame contract；
  本状态只满足实施准入，不声明本 Gate Green。

## T003 Shared CreateGroupPrerequisite

- Helix 继续复用公开 `G01ScenarioRunner::create_group_prerequisite`，通过真实
  `Tick::Command -> HTTP PortReply -> Persist/Emit` 取得权威 channel。
- PC 继续复用 domain-neutral `prepareMockCreatedGroup` 与真实 MessageV4 建群操作；
  G03a helper 不复制 G01/G02 marker 或业务成功条件。
- Mobile 继续复用 `createFreshGroupPrerequisite`，调用生产
  `im.channels.create` 并等待权威 `im:channel:created`；不复用 G02 遗留群。
- 三端能力已在 G02 闭环中重新验证，本 Gate 只复用合同，无需修改共享生产代码。
