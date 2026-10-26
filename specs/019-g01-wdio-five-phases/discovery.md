# Source Discovery

**Feature**: `specs/019-g01-wdio-five-phases`
**Source Lock**: `943ab99b99604322765a901471f02b5e3d207683`
**Worktree State**: `M .specify/feature.json
 M SESSION.md
?? specs/019-g01-wdio-five-phases/`

## Facts *(mandatory)*

- **F-001**: 当前 Helix wire 合同只接受 `InteractionFrameV2`，schema version 为 2；command accepted 只代表 Helix 接管 request，业务终态仍需 matching-causation Frame。 | **Evidence**: `crates/helix-im/src/interaction_frame.rs:11`
- **F-002**: G01 当前 Angular 上行从 active channel 组装 `channel_id/type/text/props?/replied?`，再直接调用 `im_send_message`；尚不是 Composer `ActionBinding(message.create)` 上行。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/shell.service.ts:409`
- **F-003**: G01 先在 SQLite `message` 行持久化 `sending`，当前 strict matrix 的两阶段 producer 都声明为 `message.timeline`；DialogLeft、Unread、Composer 是目标影响面，不是已证明的当前 G01 mutation。 | **Evidence**: `docs/interaction-frame-v2-全量替换-按UC/G01/README.md`
- **F-004**: `PendingSend` 是 Helix 内存聚合，记录 temporary ID、发送状态、timeout、persist correlation、connection ID、request ID、body 和 single-flight/upload 状态，不是 SQLite 表。 | **Evidence**: `crates/helix-im/src/send/pending.rs:23`
- **F-005**: 现有原生桌面测试路线要求第一阶段建立测试专用依赖和 capability，第二阶段分开报告 DOM、Mock、real invoke，第三阶段替换 raw W3C 通用驱动，第四阶段使用隔离双实例，第五阶段复用现有恢复 Harness。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/06-实施路线与验收标准.md:15`
- **F-006**: 当前 cses-client 生产依赖与配置尚未出现 `tauri-plugin-wdio`、`tauri-plugin-wdio-webdriver`、`@wdio/tauri-service`、WDIO runner/reporter 或 `wdio.conf.ts`；文档也把这些能力标记为未安装。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/01-苹果英特尔环境与现状盘点.md:21`
- **F-007**: G01 Mock 必须验证 UI/IPC 行为但不能声称 Rust/Helix/Go；真实 invoke resolve 也只是 command transport acknowledgement，完整发送仍需 Frame、DOM 和后端证据。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/04-消息四版真实链路应用.md:75`
- **F-008**: 第一版报告的既定选择是本地 Spec Reporter、CI JUnit、现有 Evidence Collector 和失败截图；Allure 与视觉差异属于后续增强。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/05-测试框架断言与报告选型.md:1`
- **F-009**: 当前十分钟瓶颈来自固定双 profile、轮询、每 Gate 双端截图/诊断和重复全量证据扫描，不是单个 WebDriver API；目标执行模型是按需 lane、常驻拓扑、事件 latch、增量证据索引和失败升级采集。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/07-十分钟瓶颈与性能目标.md:1`
- **F-010**: 已有性能目标为单个 Mock/UI 10 秒内、普通 Gate P95 15 秒内、suite topology 只启动一次、普通 Gate 不重启 profile，且必须分开记录 build/startup/scenario/report 时间。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/07-十分钟瓶颈与性能目标.md:158`
- **F-011**: Actor 444 与 Observer 678 的现有 NativeTopology 已定义独立 WebDriver 端口、profile root、auth、device ID、stop/restart/pause/resume 和同 run artifact 目录。 | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/docs/原生桌面测试体系/04-消息四版真实链路应用.md:130`
- **F-012**: 当前 source lock 是 Helix `943ab99b99604322765a901471f02b5e3d207683` 和 cses-client `2ba5a8de864836a1436568184d15809df72bf570`；后续 plan 必须重新核验 source/pin，不能把文档快照当运行态。 | **Evidence**: `SESSION.md:1`

## Unknowns

- **U-001**: 当前固定 Tauri/Wry/Mac Intel 组合是否能成功编译和运行 WDIO 两个 Rust 插件，只有 Phase 1 PoC 能证实。 | **Resolution**: plan 中建立 fail-fast build/session task；失败时保持 L0 blocked，不降级为 raw wrapper。
- **U-002**: `@wdio/tauri-service` 是否能直接表达两个实例的 per-profile 环境和独立端口，需要 Phase 4 PoC。 | **Resolution**: 若不能，保留轻量 Topology Controller 只负责进程和身份，WDIO 仍独占 UI/WebDriver 操作。
- **U-003**: Tauri Embedded WKWebView 的视觉差异服务兼容性尚未验证。 | **Resolution**: 不进入 Report V1 完成条件；本 Feature 只要求确定性截图和人工可读附件。

## Decisions

- **D-001**: Feature 目录固定为 `specs/019-g01-wdio-five-phases`，只产出一个 G01 测试体系 Feature，不混入 G02-G16 或 GS01-GS04 Green。
- **D-002**: Mock 使用 command receipt 加 Helix producer/serializer 导出或真实链捕获的 Frame V2 fixture；IPC reject 与业务 `failed` Frame 分开。
- **D-003**: 结果层级固定为 `ui-only/mock/integration/single-real-chain/dual-real-chain`，低层通过不得升级为高层 Green。
- **D-004**: 前五阶段沿用 06 路线的安全边界、单实例、raw 迁移、双实例和 Harness 接入；Phase 5 只用 G01 证明接入，不声明 GS Gate。
- **D-005**: 第一版报告固定为本地可读结果、JUnit、紧凑 summary、run manifest 与 evidence 引用；Allure、视觉基线和 CI 发布另开 Feature。
- **D-006**: 运行效率来自按需实例、常驻拓扑、事件驱动等待、O(1) correlation 索引和成功/失败两级采集；不得通过删除必要证据换取速度。
- **D-007**: 444/678 的密码不进入 Spec、日志或报告，由受控 profile/环境注入；SQLite/PG 仅只读取证。
