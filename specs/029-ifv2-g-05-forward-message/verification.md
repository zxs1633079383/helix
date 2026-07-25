# Verification Procedure: G-05

## Required Runs

- **V-001 Helix build**: plan 必须锁定受影响 crate 并执行对应 `cargo check`；期望退出码 0，且 helix-core 运行时依赖检查仍为空。
- **V-002 Helix scenario**: 执行本 Gate 的生产 `ImModule::handle` 回放测试；期望 prerequisites、happy、failure、race/idempotency 与 recovery 的 required case 全通过。
- **V-003 Canonical Fixture**: 普通测试只生成 actual Frame 并与 checked-in JSON 比较；期望 typed validation、字段断言、raw/semantic hash 与 surface inventory 全一致，未设置显式 update 开关时不得改写 Golden。
- **V-004 PC Mock**: 执行本 Gate 的 Fixture parity、Tauri Mock replay、SurfaceRuntime、DOM 断言与 reporter；期望 required fail=0/skip=0，且报告明确标记为 Mock。
- **V-005 PC Real**: 经真实 MessageV4 UI、Tauri invoke、Helix/SQLite/HTTP/WS/bus ingress 执行 single-real；涉及 observer 时追加 dual-real。期望 correlation、FrameApplied、DOM 与后端证据闭合，不用固定 sleep。
- **V-006 Mobile contract**: byte-exact 同步 Helix Fixture，执行链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 的 GTest；期望 typed view 与 Helix Frame 字段一致，默认 production build 不可达测试注入和 raw reducer。
- **V-007 Cross-repo review**: 执行行为、代码边界、Frame V2 与 evidence review；期望 `G-05` 为 `review-green`，否则保持 partial/blocked。

## Evidence Rules

- plan 阶段必须把每个 V-xxx 解析为当前仓真实存在的命令、测试文件、fixture 目录、report 路径和 timeout；无法定位时不得用猜测命令补位。
- 每次运行保留原始输出并通过 `loop_engine.py record-evidence` 登记；失败输出不得被说明文字替换。
- Mock、AcceptedCommand、HTTP 2xx、单端 replay 或 GTest 不能代替 PC Real 与跨仓 Review。
- 当前 Feature 只定义验收合同；在实现与运行证据出现前，本 Gate 状态保持 Draft。

## T001 Source Lock - 2026-07-25

- Helix：`8f3a14d81ffcb9d7d90224d2838fe3d623eeeb0e`；前序 G-04a
  T001-T038 全部完成，mandatory R4 `PASS` 且最终 `CLOSED`。
- cses-client：`f0cc3d187f4d84165a485d21abe91175264519f4`，tracked tree clean；
  Tauri 四个 Helix production dependency 当前统一 pin
  `c9c37ea8fcb398715cff872d4adbdc8a0e412c46`。
- Mobile：`883fca7f744feb6d38c48ff6b607fe7818c42bf3`；tracked tree clean，
  仅保留任务前已有 Helix bundle 与 `tests/.DS_Store` 未跟踪项。production
  Helix pin 为 `bde8d79fba6335f9d6d8ed3feafab05a80fcd151`。
- Strict Matrix 生成文档
  `crates/helix-im/docs/Strict Matrix/UC全链路发布矩阵-v1.md` raw SHA-256 为
  `6f186cf291dc6336798b1178bf3d97493588eea8ccc467f026766654d3fe2354`；
  机器真源 `crates/helix-im/contracts/strict-matrix.v1.json` raw SHA-256 为
  `b6625b033db6ada5a05ae9d7898f664df25cd097a5a0e18d42aaa3d3e00a02ac`。
- 当前已有生产转发装配、HTTP per-target admission、可信 WS correlation 与
  dialog Persist barrier；尚无 G05 完整 ScenarioRunner、canonical Fixture、PC
  独立 lane 或 Mobile G05 Frame contract，因此只满足实现准入，不声明 Green。

## T003 Shared CreateGroupPrerequisite

- Helix 复用 G01 已验证的生产 fresh-group prerequisite：先经
  `im_create_channel` 取得权威 `channelId` 并持久化 channel/member，再在 source
  group 发送待转发消息；target group 也必须由同一前置能力新建。
- PC 复用 `g01-group-prerequisite.ts` 从真实 UI 创建 source/target fresh group；
  Mock 只回放 Helix canonical Frame，不在 Angular 补建群或转发 reducer。
- Mobile 复用 `UcJsContractFixture::RC.createChannel`，每次 real-chain 新建 source
  与 target group；GTest 的 command fixture 不把固定测试 ID 冒充真实前置完成。
- 三端无需新增第二套 CreateGroup 业务逻辑；本 Gate 只消费其权威 channel ID，
  因此 T003 无生产代码修改。

## T004-T012 Helix Producer Green

- production behavior commit：
  `3798ffdd0bfd89b8922829dd252327bb0a2075ec`；Fixture commit：
  `b61d39bee004a4e195a18fbb0c00fe56301dc6f2`。
- `G05ScenarioRunner` 只通过 public `client_api::build_command`、
  `ImModule::handle`、匹配 `PortReply`、可信 `Tick::Inbound` 与
  `Effect::Emit` 回放；没有测试专用 reducer 或直接修改 module state。
- RED 回放证实并修复两个生产问题：createPosts HTTP admission 曾发布无事实变化的
  causation Frame 并提前完成 ActionRegistry；attached target timeline 曾先消费
  authority ledger，导致随后 left-dialog Frame 丢失 causation。当前模型明确为
  timeline reconciliation 与 dialog patch 两条独立投影，dialog attached 时由其
  durable readback 完成 final target；headless 无 dialog 时 timeline 才是唯一终态。
- `cargo check -p helix-im`：退出码 0；`cargo tree -p helix-core` 对
  `tokio|sqlx|reqwest|web-sys|tauri` 搜索为空。
- `cargo test -p helix-im --test g05_full_flow_test --test
  g05_forward_interaction_test -- --nocapture`：退出码 0，6/6 full-flow 与
  3/3 focused contract 全过。覆盖 fresh target、source authority、业务拒绝、
  transport failure、WS-first/late HTTP、duplicate inbound 与 dialog PersistErr
  replay。
- 显式更新命令 `UPDATE_G05_WDIO_FIXTURES=1 cargo test -p helix-im --test
  g05_wdio_fixture_contract_test -- --nocapture`：退出码 0；普通模式重跑 1/1
  通过，证明未设置开关时只比较 checked-in Golden。
- canonical inventory 为 4：`group-created`、`source-forwardable`、
  `target-forwarded`、`target-dialog`，schema 全为 2；surface inventory 为两个
  `message.left-dialog-list` 与两个 `message.timeline`。
- raw SHA-256：group-created
  `4f3a10813d059d7eb58cb2918f2944ceefd1f2bbf8253107283c1c20e7861123`；
  source-forwardable
  `cc0195a098bd739a3b3ae8cae3d2b147f2f68dce066f451aa5c6b34d5ae3c5c1`；
  target-forwarded
  `1406445940595964e560f0128c68eadb6c3527bfd4b0b1478028376f6935334b`；
  target-dialog
  `bca69ee5f98e2f08564dcd6d632f4d00a90993eed104f2322d107b37097ed641`；
  provenance
  `164b1b1b82afce5f945459e4acc37d2dd934bc8395a2e6610841e2808e1ed808`。

结论：G-05 Helix producer 与 canonical Fixture required lane 为 Green；PC 与
Mobile 尚未开始，Gate 整体仍为 `implementing`，不得进入 G06。
