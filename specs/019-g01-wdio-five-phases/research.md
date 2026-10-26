# Research: G01 WDIO 前五阶段

## R-001 测试执行面

**Decision**: 在 cses-client 新建 `tests/e2e/wdio`，采用 WDIO + Mocha + expect-webdriverio；版本以安装时和当前 Node 20、Tauri 2 lockfile 解析出的同一兼容族为准并锁入 lockfile。

**Rationale**: 标准 runner 提供自动等待、hooks、reporter、multiremote 和统一元素 API，能替换 G01 对 raw W3C client 的直接调用而不重写业务 Harness。

**Alternatives considered**: 继续扩展 raw client 会保留固定轮询和自建 reporter；Playwright 不直接解决真实 Tauri WKWebView 与现有 Tauri plugin 接缝。

## R-002 测试能力隔离

**Decision**: 新增 additive `wdio-e2e` Cargo feature、专用 capability/config 和双条件插件注册；production build 通过静态/运行负向测试证明入口不可达。

**Rationale**: 当前 `desktop-e2e` 已按 profile 条件注册旧 WebDriver 插件。新 runner 必须复用这一安全思想，但不能扩大默认 capability 或把 Mock/execute bridge 编入生产。

**Alternatives considered**: 仅用环境变量隐藏插件不能提供编译期保证；复用默认 capability 会扩大生产权限。

## R-003 Mock 的两类对象

**Decision**: `im_send_message` Mock 只返回 `AcceptedCommand`；业务 `sending/sent/failed` 由完整 Frame V2 fixture 经 `im:__bus__` 注入。

**Rationale**: receipt 只证明 command 入队，Frame 才是 Angular delivery/identity/DOM 的权威来源。IPC reject 与业务 failed 必须分开。

**Alternatives considered**: 在 invoke 返回 ViewModel 会让 Angular 重建领域事实；Promise reject 伪装业务失败会破坏 Frame authority。

## R-004 Fixture 真源与同步

**Decision**: Helix 在 `crates/helix-im/tests/fixtures/g01-wdio` 保存 canonical fixture，由 Rust contract test 解码并验证 producer/source pin；client 只镜像、校验 semantic hash，不编辑内容。

**Rationale**: 当前 Helix 已有 `uc01-16-surface-runtime` manifest/contract 模式，可直接扩展。跨仓复制需要 provenance 与 hash 阻止漂移。

**Alternatives considered**: WDIO helper 手写 Frame 无法证明生产契约；只保存真实抓包而不加 serializer contract 容易随动态字段漂移。

## R-005 拓扑策略

**Decision**: suite 级复用 topology；Mock/UI/integration/single-real 启动一个 Actor，dual-real 才增加 Observer。persona 独立 profile/auth/device/port/artifact。

**Rationale**: 现有 `NativeTopology` 与 dual-profile runner 已处理进程和 profile，adapter 复用能避免每 case 重建应用。

**Alternatives considered**: 固定双实例让快速 lane 支付无关成本；完全重写 topology 会重复登录、进程恢复和日志能力。

## R-006 等待与 correlation

**Decision**: 以 run-scoped event emitter/latch 和 `Map<string, Event[]>` 索引 reqId、trackId、transactionId、postId、eventSeq；DOM 使用 WDIO 自动等待，禁止固定 sleep。

**Rationale**: 增量写入 `O(1)`，单键读取平均 `O(1)`，避免每次断言扫描完整 console/Tauri/Go 日志。

**Alternatives considered**: 高频轮询和正则全文件扫描会随 suite 时长线性恶化；固定 sleep 既慢又不可靠。

## R-007 证据采集级别

**Decision**: 成功路径写 summary、required final assertions 和 canonical 必需截图；失败升级采 DOM、console、Tauri/Helix、Go/WS、数据库和截图，保留第一次失败。

**Rationale**: 速度优化不能删除 canonical 证据，但普通 Mock/UI 无需采双端数据库与全量日志。

**Alternatives considered**: 每步截图/全量采集导致十分钟链；只保留最终 retry 会掩盖 flaky 根因。

## R-008 Report V1

**Decision**: Mocha Spec 为本地反馈，JUnit 为 CI 标准输入，自定义 reporter/adapter 生成 JSON/Markdown summary、run manifest 和附件索引；Allure 延后。

**Rationale**: 第一版必须可读、可机器解析并表达 evidence level/claim boundary；不需要先引入完整报告服务。

**Alternatives considered**: 仅 console 不可聚合；仅 JUnit 无法表达双 persona/证据面；Allure 会扩大首期依赖。

## R-009 日志与隐私

**Decision**: 所有 reporter/log adapter 使用字段 allowlist；正文只保留受控 marker hash，authorization/Cookie/token/password/raw payload 一律拒绝写入。

**Rationale**: 双账号与真实链最容易将凭据和 payload 带入失败附件。allowlist 比事后黑名单更可证伪。

**Alternatives considered**: 仅正则替换常漏嵌套 header/body；整文件复制再脱敏扩大暴露窗口。

## R-010 数据库边界

**Decision**: SQLite/PG 只执行 run/channel/post 范围的有界只读查询；测试不写库、不修库、不用 DB 结果驱动 UI。

**Rationale**: 数据库是证据面而非测试造数入口，直接写入会绕过 Go/Helix 链。

**Alternatives considered**: DB seed/repair 可制造假 Green；无界 dump 既慢又有隐私风险。

## R-011 G01 raw 迁移范围

**Decision**: 只要求 G01 scenario 不再直接调用 raw W3C client；共享旧 driver 在其他 Gate 仍依赖时保留并禁止新增 G01 调用。

**Rationale**: 本 Feature 的范围是 G01，提前删除共享 driver 会破坏未迁移 Gate。

**Alternatives considered**: 一次删除全部 raw driver 会把 G02/G04-G16 迁移偷偷并入本 Feature；双栈长期并行则无法验证 G01 切换。

## R-012 性能统计

**Decision**: 将 build/install/首次授权与 warm scenario 分开；case 记录 setup/action/ack/frame/dom/evidence/screenshot/report/total，按 lane 聚合 P95。

**Rationale**: 只有阶段化时延才能定位瓶颈，也避免把一次性编译时间伪装成 WebDriver 慢。

**Alternatives considered**: 单一 wall time 无法区分进程、网络、Frame 或报告成本。
