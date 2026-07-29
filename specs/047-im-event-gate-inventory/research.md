# MessageV3 Gate Event 重构研究

## R-001：唯一运行时边界

**Decision**: 保留 `helix-core::Tick/Effect/DomainEventBytes` 和三端 `EventSink`，不创建新的
runtime、Frame 或 Surface 抽象。MessageV3 是 `helix-im` 对 `{event,data}` 业务合同的版本名。

**Rationale**: native、FFI、Web 已能原样交付 `DomainEventBytes`；问题位于 `helix-im`
内部同时存在 Frame/Surface 和旧 raw compatibility，而不是 core/driver 缺能力。

**Alternatives considered**:

- InteractionFrame V3：被用户明确拒绝，会重建同类套壳。
- 新增 MessageV3 crate/runtime：违反最小变更和 runtime-agnostic 边界。
- 平台 reducer：会复制业务状态机。

## R-002：持久事实到 Emit

**Decision**: durable event 必须由 matching `PersistOk` 后的本地行或同一 atomic write
中已确认的 committed values 构造；字段 lineage 以 storage schema lock 为门禁。

**Rationale**: HTTP ack、Go WS wire 和 SQLite row 不等形。直接从 inbound wire 拼 terminal
event 会绕过持久屏障，并重新引入多层字段转换。

**Alternatives considered**:

- WS 收到即 emit：不能证明本地恢复一致。
- HTTP success 即 emit：缺 server identity 或 authority 时会产生假 sent。
- UI 侧查询/补字段：把业务计算泄漏到平台。

## R-003：V1 inbound 的删除边界

**Decision**: 真实 Go WS action 与 `Tick::Command/Inbound/PortReply` 不是旧 V1，必须保留；
删除对象是 `legacy_event.rs`、旧 command alias、旧 envelope normalize、只为旧 consumer
存在的 parser fallback，以及绕过 Gate handler 的旁路。

**Rationale**: 服务端 wire 是外部依赖，不能因“V1”命名误删。应删除的是兼容翻译层，而不是
Go 当前仍发送的 `post/posts_update/update_channel/...`。

**Alternatives considered**:

- 全删 WS parser：会切断真实 authority。
- 保留全部 alias：无法证明旧链归零。

## R-004：V2 删除边界

**Decision**: 删除 `interaction_frame.rs`、`surface_composer.rs`、`surface_runtime.rs`、
`dialog_frame.rs`、`dialog_patch.rs`、`projector/`、ActionBinding/UiDirective/FrameApplied
及专属 correlation/fixture；其中可复用的业务 shaping 和本地 query 移入 `event/` 或领域模块。

**Rationale**: 目录名不是唯一判断标准。持久写、窗口边界、viewer 授权是业务能力，应保留；
Frame schema、surface revision、actionRef、VM 包装是被废弃的呈现协议。

**Alternatives considered**:

- 整目录直接删除：会误删已验证的业务算法。
- 仅停止 export：留下死代码和测试假绿。

## R-005：测试产物生成

**Decision**: 每 Gate 一个独立目录和 integration test target。测试根固定为
`tests/message_v3/`，先按 `post/schedule/timeline/read/channel/sync` 领域分包，再使用
`<gate>/main.rs` 隔离 Gate；`Cargo.toml` 用显式 `[[test]]` 注册
`message_v3_<gate>`。公共代码只放在 `tests/message_v3/support/`。测试始终在内存中校验；
设置 `HELIX_GATE_ARTIFACT_DIR` 时额外原子写 JSON。Node 脚本只调用测试并校验结果。

**Rationale**: Rust 测试才能证明 payload 来自真实 `ImModule::handle`。环境变量让普通
`cargo test` 不污染工作区，同时允许 `.v1` 证据生成。

**Alternatives considered**:

- JavaScript 手拼：与生产结构漂移，已被否决。
- 每次普通测试都写仓库：并行和只读 CI 不稳定。
- 单个 30 Gate 巨型测试：失败定位差且违背逐 Gate 硬切。
- 30 个测试平铺在 `tests/` 根：无法按领域识别 owner，也不满足独立测试目录要求。

## R-006：本版本跨仓边界

**Decision**: 只生成 Helix `.v1` artifact 和 provenance，不复制到 PC/Mobile，不修改
WDIO/GTest。artifact schema 预留 consumer 和镜像 hash 字段，但当前状态为
`NOT_SYNCED_BY_SCOPE`。

**Rationale**: 用户要求先完成 Helix coding 和完整测试。把跨仓同步混入会扩大工作树与证据面。

**Alternatives considered**:

- 直接跨仓读取 `.v1`：依赖绝对路径且 `.v1` 被忽略。
- 本版本同步 mirror：超出确认范围。

## R-007：迁移顺序

**Decision**: shared → G-15a/G-01 → Post → Schedule → Timeline/Read → Channel →
Sync/Recovery → global purge。

**Rationale**: 建群和发送建立最小业务主链；sync 必须最后验证已经迁移的事实恢复，不能先为
尚不存在的 MessageV3 event 建假恢复层。

**Alternatives considered**:

- 截图顺序从 G-S1 开始：依赖倒置。
- 30 Gate 大爆炸：无法保持可审阅删除边界。
