# MessageV3 Gate Event 验证指南

## 前置

1. 当前分支为 `reactor/MessageV3-helix`。
2. Rust toolchain 为 `1.95.0`，安装 `wasm32-unknown-unknown`。
3. 不要求 cses-client 或 Mobile 仓库存在。
4. `.v1` 仍命中 `**/.v1/` Git ignore。

## 单 Gate 开发循环

以 G-01 为例：

```bash
cargo test -p helix-im --test message_v3_g01 -- --nocapture
```

预期先证明真实 `ImModule::handle` 剧本、PersistOk 屏障和 sender/receiver event 顺序。需要生成
JSON 时：

```bash
HELIX_GATE_ARTIFACT_DIR="$PWD/specs/047-im-event-gate-inventory/.v1/gates/G-01/generated" \
  cargo test -p helix-im --test message_v3_g01 -- --nocapture
```

预期生成：

```text
inbound.json
outbound.sender.json
outbound.receiver.json
provenance.json
```

G-16b 的机器 `domain` 为 `channel-member`，对应 Rust 测试包目录为
`tests/message_v3/channel_member/g16b/`；其余 domain 与测试包同名。

## 全 30 Gate 生成

```bash
node specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs
```

该命令只能调用 Rust Gate tests、生成索引并执行 schema/hash 校验。若脚本源码仍出现
`wsSample`、`eventSampleData` 或手写业务 payload factory，验证必须失败。

## 集合与结构验证

```bash
jq '.gate_ids | length' specs/047-im-event-gate-inventory/.v1/manifest.json
git check-ignore -v specs/047-im-event-gate-inventory/.v1/manifest.json
```

预期 Gate 数为 30，且 `.v1` 被忽略。manifest 必须报告
`missing=0/extra=0/duplicate=0`。

在任何生产编码前，`contracts/gate-migration.v1.json` 还必须已经为 30 Gate 填满生产 owner
和删除引用；空数组、`TODO` 或未知 owner 均应由 checker 拒绝。P0/P7 的共享
`IN_PROGRESS` 来自 `review-ledger.json`，单 Gate `IN_PROGRESS` 来自 migration manifest，
两者合并后最多一个。

## 单 Gate 旧链删除

按该 Gate 的 migration entry 执行聚焦 `rg`。只有 Gate 专属 V2/V1 引用为 0、生成物 hash
稳定且 focused test 通过，才能把状态改为 `MIGRATED`。共享 Frame/Surface 文件在最后一个
引用 Gate 前不删除，但已迁移 Gate 不得继续调用。

## 阶段验证

每个阶段只运行一次对应 crate check 和阶段 Gate tests；不做改一行跑一次 Cargo：

```bash
cargo check -p helix-im
cargo test -p helix-im --test <gate_target_1> --test <gate_target_2>
```

## 最终验证

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web
cargo test -p helix-core
cargo test -p helix-im
cargo build --target wasm32-unknown-unknown -p helix-driver-web
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
git diff --check
```

`cargo tree` 的 grep 必须为空。另需执行全仓 V2/V1 零引用门禁；历史归档允许项必须来自显式
allowlist，生产源码、当前测试和当前合同命中数必须为 0。

## 证据边界

本版本只证明 Helix coding、真实生产回放、canonical JSON 和 Rust 三端编译边界。未同步
PC/Mobile fixture，未运行 WDIO/GTest，也不声明跨仓或真实应用 Green。
