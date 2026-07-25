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
