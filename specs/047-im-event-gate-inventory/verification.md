# MessageV3 Gate 硬切验证

## 六级证据边界

| 等级 | 本轮结果 | 能证明 | 不能证明 |
|---|---|---|---|
| E1 deterministic | `PASS` | 同一 Gate 回放与重复生成收敛为稳定 JSON/hash | SQLite、Go、真实宿主 |
| E2 contract | `PASS` | 30 Gate、120 JSON、schema、字段禁集、迁移状态与旧链删除 | 真实网络和客户端消费 |
| E3 integration | `PASS`（Rust driver 接缝） | core/native/FFI/Web 编译，FFI 批量原样交付，Web wasm 构建 | 真 Go、认证、设备 |
| E4 runtime | `NOT_RUN` | 无 | 单账号产品链 |
| E5 dual-user | `NOT_RUN` | 无 | 真实 sender/receiver 双账号 |
| E6 canonical consumer | `NOT_RUN` | Helix producer 已生成 canonical，但消费者未同步 | PC/Mobile byte-exact 消费 |

PC 与 Mobile 本版本均为 `NOT_SYNCED_BY_SCOPE`。WDIO、Mobile GTest、真实宿主、真实 Go/WS、
双用户和跨端 canonical 消费没有执行，不得借用 E1-E3 的通过状态。

## 最终结果

| 检查 | 结果 | 证据 |
|---|---|---|
| `cargo check` core/im/native/FFI/Web | `PASS` | `evidence/p7-final-validation.txt` |
| `cargo test -p helix-im --quiet` | `PASS` | 194 个 lib 测试及全部 integration target 通过；11 个既有 ignored 用例未晋级为证据 |
| 30 个 MessageV3 Gate 独立 target | `PASS` | migration manifest 中 30/30 target 唯一注册并由全量测试执行 |
| `cargo test -p cses-helix-core` | `PASS` | 16 个 lib、6 个 integration、1 个 doctest 通过；1 个 doctest ignored |
| `cargo test -p helix-driver-ffi --quiet` | `PASS` | FFI lib 与全部 integration target 通过 |
| Web `wasm32-unknown-unknown` build | `PASS` | `helix-driver-web` 构建完成 |
| core runtime dependency scan | `PASS` | `tokio/sqlx/reqwest/web-sys/tauri` 命中为 0 |
| canonical 生成 | `PASS` | 30 Gate、120 JSON、tree SHA-256 `0969896be01dc28bcc63aafe62894cd31ce4ed02947dcbaeb888c756397ee0fc` |
| cutover checker + 负例自测 | `PASS` | missing=0、extra=0、duplicate=0、generated=`EXACT`、active=0 |
| `.v1` ignore | `PASS` | `.gitignore:21` 的 `**/.v1/` 命中 |
| `git diff --check` | `PASS` | 无空白错误 |

第一次最终证据命令在删除无用 re-export 后暴露了一个 `#[cfg(test)]` 作用域漂移，原始失败保存在
`evidence/p7-final-validation-failed.txt`，修复后完整重跑通过。失败证据未被成功摘要覆盖。

## 物理下线

- 已删除 `interaction_frame.rs`、`surface_composer.rs`、`surface_runtime.rs`、
  `dialog_frame.rs`、旧 `dialog_patch.rs`、`projector/` 与专属 action/capability 代码。
- 已删除旧 WS `legacy_event` 与 V1 inbound fallback；现役 Go wire parser 保留。
- FFI 已删除 `helix_invoke_action` 与 Surface resync 私有入口，三端继续只搬运
  `DomainEventBytes`。
- 旧 50 Gate Matrix 与其 command API 已标记 `RETIRED_HISTORICAL`；当前 30 Gate 机器真源为
  `contracts/gate-migration.v1.json`。
- 当前 Rust 生产、driver 与测试不存在旧 UI 模块 import、旧业务 event 或独立 action ABI。
  测试中的退役术语仅用于负断言；历史文档只能落入 checker 输出的显式 allowlist。

## 产物合同

每个 `.v1/gates/<Gate>/generated/` 固定包含：

- `inbound.json`
- `outbound.sender.json`
- `outbound.receiver.json`
- `provenance.json`

生成器只编排 30 个真实 Rust Gate target，不手写业务 `event/data`。G-01 sender 的
`im:channel:update.unreadCount=0`，receiver 使用各自本地已提交基线递增；两侧 `lastPost`
均包含同一条消息的 `id/userId/message/createAt`。

## Required Runs

- **V-001**: `cargo check -p cses-helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web`
- **V-002**: `cargo test -p helix-im --quiet`
- **V-003**: `cargo test -p cses-helix-core`
- **V-004**: `cargo test -p helix-driver-ffi --quiet`
- **V-005**: `cargo build --target wasm32-unknown-unknown -p helix-driver-web`
- **V-006**: `node specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs`
- **V-007**: `node specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs --require-generated --self-test`
- **V-008**: `git diff --check` 与 `.v1` `git check-ignore`

## Evidence Rules

原始命令输出保存在 `evidence/`，并通过
`loop_engine.py record-evidence` 登记 SHA-256。deterministic、contract、integration、
runtime、dual-user 与 canonical consumer 证据互不替代；失败、ignored、`NOT_RUN` 和
`NOT_SYNCED_BY_SCOPE` 必须保持显式。
