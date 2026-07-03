# Task 1 Report - 冻结移动端公开契约

## 实现内容

- 新增 `crates/helix-im/src/mobile_contract.rs`
  - 冻结移动端公开常量：`CONTRACT_VERSION=1`、`BUS_CHANNEL="im:__bus__"`
  - 增加 `im_send -> im_send_message` 别名映射
  - 增加 `normalize_command_payload(public, payload)`，在 Rust 边界把 `channelId`/`temporaryId` 归一成 `channel_id`/`temporary_id`
  - 增加 `capabilities_json()`，公开 ABI / bus / commands / events / features，并明确不宣传 `im:post:send-failed`
- 修改 `crates/helix-im/src/lib.rs`
  - 导出 `pub mod mobile_contract;`
- 修改 `crates/helix-im/src/outbound/registry.rs`
  - 新增 `outbound_command_names()`，稳定导出当前 outbound 命令闭集（排序后）
- 修改 `crates/helix-im/src/query/core.rs`
  - 抽出 `QUERY_COMMAND_NAMES`
  - 新增 `query_command_names()`
  - `is_query()` 改为复用同一闭集

## RED

- 命令：
  - `cargo test -p helix-im mobile_contract -- --nocapture`
- 结果：
  - RED 符合预期，编译失败
  - 关键失败摘要：
    - `cannot find function normalize_command_payload in this scope`
    - `cannot find function command_alias in this scope`
    - `cannot find function capabilities_json in this scope`

## GREEN

- 命令：
  - `cargo test -p helix-im mobile_contract -- --nocapture`
- 结果：
  - 通过
  - 摘要：
    - `2 passed; 0 failed`
    - `send_alias_normalizes_to_existing_core_command_and_snake_payload ... ok`
    - `capabilities_do_not_advertise_send_failed_as_p0 ... ok`

## 最终 cargo check

- 命令：
  - `cargo check -p helix-im`
- 结果：
  - 通过
  - 摘要：
    - `Finished 'dev' profile [unoptimized + debuginfo] target(s) in 1.70s`

## 变更文件

- `crates/helix-im/src/mobile_contract.rs`
- `crates/helix-im/src/lib.rs`
- `crates/helix-im/src/outbound/registry.rs`
- `crates/helix-im/src/query/core.rs`
- `.superpowers/sdd/task-1-report.md`

## 自检结论

- 满足 `name + JSON payload` 的公开面，没有为每个 IM 用例扩 ABI 符号
- camelCase 仅在 Rust 边界归一化，再交给既有 helix-im 命令处理器
- 未把 IM schema/查询整形下放到 `helix-driver-ffi`
- 未引入 `im:post:send-failed` 到移动端 P0 能力广告
- cargo 调用次数遵守约束：RED 1 次、GREEN 1 次、最终 check 1 次
- 仅修改白名单内文件

## 疑虑

- 当前写入白名单不包含 `crates/helix-im/src/outbound/mod.rs`，所以新加的命令名导出能力实际落点是 `helix_im::outbound::registry::outbound_command_names()`；如果后续要求公开路径严格平铺为 `helix_im::outbound::outbound_command_names()`，需要额外放开该文件做一次 re-export。

## 导出路径修复记录（2026-07-03）

- 修复点：
  - 将 `crates/helix-im/src/outbound/mod.rs` 的 `pub use registry::{...}` 补上 `outbound_command_names`
  - 将 `crates/helix-im/src/mobile_contract.rs` 中的能力清单来源切到 `crate::outbound::outbound_command_names()`
  - 增加一个最小测试，锚定公开路径与 registry 路径一致
- 命令：
  - `cargo test -p helix-im mobile_contract -- --nocapture`
  - `cargo check -p helix-im`
- 结果：
  - `cargo test -p helix-im mobile_contract -- --nocapture` 通过，`3 passed; 0 failed`
  - `cargo check -p helix-im` 通过，`Finished 'dev' profile [unoptimized + debuginfo] target(s) in 4.87s`
