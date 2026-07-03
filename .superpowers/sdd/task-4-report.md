# Task 4 Report

## 实现内容

- 在 `crates/helix-im/src/mobile_query.rs` 新增移动端本地同步查询规划面：
  - `plan(name, payload)` 只接受 `im_query_dialog_list` / `im_query_messages_by_channel`
  - `MobileQueryPlan::Scan { spec, shape }` 把表名、过滤条件、排序和响应形状都留在 `helix-im`
  - `shape_json(shape, rows)` 负责把 `Storage::scan` 的 `Vec<Row>` 整形成移动端契约 JSON
- 在 `crates/helix-im/src/lib.rs` 挂载 `pub mod mobile_query;`
- 在 `crates/helix-driver-ffi/src/api.rs` 扩展 `helix_query`：
  - 继续保留 `helix_capabilities` / `helix_version`
  - 对其余 query 先尝试 `helix_im::mobile_query::plan(...)`
  - FFI 只执行 `Storage::scan(spec)`，然后把结果交回 `helix_im::mobile_query::shape_json(...)`
  - 未识别查询仍返回 `FFI_UNIMPLEMENTED` 并清空 out 参数
- 在 `crates/helix-driver-ffi/tests/mobile_contract_test.rs` 增加本地同步查询测试：
  - `im_query_dialog_list`
  - `im_query_messages_by_channel`
  - helper 会先准备一份带 `IM_SCHEMA` 的共享测试库，再让 FFI handle 指向它

## RED

- 命令：
  - `cargo test -p helix-im mobile_query -- --nocapture`
- 结果：
  - `mobile_query::tests::message_query_plan_keeps_schema_inside_helix_im ... FAILED`
  - 失败点：`plan: Parse("mobile query todo")`
  - 结论：最小模块挂载成功，测试真实命中 `mobile_query.rs`，实现尚未接通

## GREEN

- 命令：
  - `cargo test -p helix-im mobile_query -- --nocapture`
  - `cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture`
- 结果：
  - `helix-im` 过滤命中 `mobile_query`：`3 passed; 0 failed`
  - `mobile_contract_test`: `6 passed; 0 failed`

## 最终 cargo check

- 命令：
  - `cargo check -p helix-im -p helix-driver-ffi`
- 结果：
  - `Finished 'dev' profile [unoptimized + debuginfo] target(s) in 1.46s`

## 变更文件

- `crates/helix-im/src/mobile_query.rs`
- `crates/helix-im/src/lib.rs`
- `crates/helix-driver-ffi/src/api.rs`
- `crates/helix-driver-ffi/tests/mobile_contract_test.rs`
- `.superpowers/sdd/task-4-report.md`

## 自检结论

- 公开业务面仍是 `name + JSON payload`，没有为每个 IM 用例新增 C ABI 符号
- `helix-driver-ffi` 只执行 `helix-im` 给出的查询计划，没有把 `message/channel` schema 逻辑散落到 driver
- `helix-core` 侧没有新增运行时依赖
- 未触碰 `crates/helix-driver-ffi/helix.h` / `helix.hpp`
- 改动只落在任务允许的写入范围内

## 适配当前真实代码的说明

- brief 里的 `QueryDeps` 字段示例写的是 generic handle 语义；当前真实类型是 `query.runtime: tokio::runtime::Handle` + `query.storage: Arc<HostStorage>`，实现按当前签名适配
- brief 里的 `ScanSpec` / `Row` 片段与当前代码一致，可直接落到 `helix_core::effect::{ScanSpec, Row, SqlValue}`
- 当前 `HostStorage::open_sqlite_url()` 只迁移通用 `_helix_monotonic`，不会自动执行 `helix_im::IM_SCHEMA`；因此 FFI 集成测试需要先准备一份带 IM schema 的测试库，再让 handle 复用它。这一适配只放在测试 helper，没有把 migration 职责塞回 `helix-driver-ffi`

## 疑虑

- 生产路径如果在真正迁好 `IM_SCHEMA` 之前就发起 `im_query_*`，当前实现仍会按 storage 错误返回 `FFI_INVALID`；这次通过测试 helper 保证了真实查询面可验证，但“FFI 层是否要容忍未迁库并返回空投影”仍是后续产品语义选择
