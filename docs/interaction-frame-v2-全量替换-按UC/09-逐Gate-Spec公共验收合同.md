# 逐 Gate Spec 公共验收合同

> 本文是 GS01-GS04、G01-G02、G04-G16 各 `11-V2切换Spec.md` 的公共真源。Gate Spec 只记录自己的业务目标、数据面、组件和专属断言，不复制本文件的切换纪律。

## 固定范围

- 机器 Gate：GS01-GS04、G01-G02、G04/G04a、G05-G09、G10/G10a、G11-G13、G14/G14a、G15/G15a、G16/G16b/G16c，共 25 Gate。
- 排除 G03/G03a。目录 `G17` 是未纳入本轮的提案，不属于完成口径。
- 父规格：[`specs/018-interaction-frame-v2-cutover/spec.md`](../../specs/018-interaction-frame-v2-cutover/spec.md)。

## 前置与执行顺序

1. 冻结同一个 Helix immutable revision，并让 cses-client 四个 Helix 包和 `Cargo.lock` 指向它。
2. Helix 生产可达路径只生成 `InteractionFrameV2`；V1 writer 命中立即失败。
3. cses-client 生产可达路径只接受 V2；V1 decode、union、normalize 和 raw business fallback 均不可达。
4. 在 V1 代码尚可物理保留但运行不可达的候选上完成首轮 25 Gate 真链验收。
5. 首轮全绿后物理删除 V1 writer、reader、normalize、fixture 和兼容测试。
6. 创建新的 candidate、artifact 目录和 evidence epoch，重新执行全部验收；不得复用删除前截图或日志。

## 每个 Gate 的共同证据面

| 证据面 | 必须证明 |
|---|---|
| 输入 | ActionBinding 或 HostObservation/Inbound/Timer 的 actor、scope、surfaceEpoch、observedRevision 与 correlation 可定位 |
| Helix | Tick、Effect、权威 PortReply、PersistAtomic write-set、PersistOk 和 V2 compose 顺序完整；PersistErr 不发布成功 Frame |
| Go | matching HTTP request/response、WS 或 sync authority、track/correlation 和 actor/observer 可见性均可定位 |
| 远端 PG | 使用本 Gate 的 identity、主键、actor/observer 和运行前后 delta 只读查询；旧行不算本轮证据 |
| 本地 SQLite | 使用同一 identity 只读证明预期 write-set 已提交；cursor、canonical identity 和 terminal 状态与 Frame 一致 |
| Frame | `schemaVersion=2`、transactionId、causationId、surfaceEpoch、baseRevision/revision、完整 typed VM 和 directive 顺序正确 |
| Angular | V2 codec 整帧校验、SurfaceRuntime 原子提交、生产 `FrameApplied` 回传；无 V1 normalize/raw reducer |
| UI | 目标组件 DOM/ARIA/data-* 正确，444/678 双用户截图无空白、遮挡、溢出或错误权限 |
| 纯 V2 | candidate 聚合 `v1Frames=0`、`rawBusinessFallbacks=0`，每个 required Gate fail=0、skip=0 |

## 可执行工具

### Helix focused

```bash
cargo test -p helix-im --test interaction_frame_contract_test --no-default-features
cargo test -p helix-im
```

Gate 实现阶段应优先运行对应 feature spec/task 指定的 focused test；完整模块完成前只运行一次聚合 Cargo 验证。

### cses-client 静态与合同

```bash
npm run message:command-api:gate
npm run message:projection-ownership
npm run message:surface-runtime:source-pin
npm run message:ui:authority
npm run message:ui:selector-contract
npm run message:real-chain:doctor
```

### Native 与证据聚合

```bash
npm run message:desktop:frame-prerequisites
npm run message:desktop:scenario:all
npm run message:real-chain:collect
npm run message:real-chain:generate-matrix
npm run message:real-chain:reduce
```

`message:desktop:scenario:all` 当前仍需新增显式 25-Gate V2 profile，以及 `v1Frames`、`v2Frames`、`rawBusinessFallbacks` 和 `FrameApplied` 采集；能力未补齐前只能判 BLOCKED，不能判 V2 PASS。

### SCSS/CSS 与视觉

当前没有独立的 SCSS/CSS 验收脚本。实施计划必须新增可执行 gate，至少覆盖：

1. Sass/CSS 编译无错误；
2. 本 Gate 目标组件样式实际载入；
3. 444/678 两个窗口的关键容器无 overflow、遮挡和布局跳变；
4. actor/observer 截图与 DOM/ARIA 断言同一运行产生。

## 数据安全边界

- PG 与 SQLite 验收只读，不允许手工 INSERT/UPDATE/DELETE 制造结果。
- 所有写入必须来自 Action/Observation → Effect → Go/Storage 正常产品链。
- 查询日志不得输出 cookie、authorization、消息正文或无界原始 body。
- 运行环境数据库/schema 必须先用只读元数据确认；不能把配置默认值当运行态事实。

## Gate 完成定义

单 Gate 只有同时满足以下条件才可 PASS：

1. Gate 专属 Spec 的所有验收项有本轮证据；
2. 公共证据面无缺失，actor/observer、PG、SQLite、Frame、DOM 与截图关联到同一 causation；
3. V1 producer/consumer 和 raw fallback 在生产路径零命中；
4. 首轮纯 V2 candidate 与物理删除 V1 后的新 candidate 均通过；
5. 不以 HTTP 2xx、WS 入队、旧数据库行、单张截图或旧 017 Green 单独代替完成。
