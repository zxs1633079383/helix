# Task 3 Report

## 实现内容

- 在 `crates/helix-driver-ffi/src/api.rs` 中实现 `helix_query` 的本地元信息查询：
  - `helix_query("helix_capabilities", "{}")` 返回 `helix_im::mobile_contract::capabilities_json()`
  - `helix_query("helix_version", "{}")` 返回 `{"abi":1}`
  - 未知查询继续返回 `FFI_UNIMPLEMENTED`，并保持 `out_ptr = null`、`out_len = 0`
- 新增 `write_query_bytes` 辅助函数，把 JSON 字节安全写回 FFI out 参数
- 在 `crates/helix-driver-ffi/tests/mobile_contract_test.rs` 中补齐移动端契约查询测试：
  - `helix_capabilities` 返回能力集 JSON
  - `helix_version` 返回 ABI 元信息
  - 未知查询仍 unimplemented 且清空 out 参数

## RED

- 命令：
  - `cargo test -p helix-driver-ffi --test mobile_contract_test helix_capabilities -- --nocapture`
- 结果：
  - `helix_capabilities_query_returns_mobile_contract_json ... FAILED`
  - 失败点：`assertion left == right failed; left: -5 right: 0`
  - 结论：`helix_query("helix_capabilities")` 仍走 `FFI_UNIMPLEMENTED`

## GREEN

- 命令：
  - `cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture`
  - `cargo test -p helix-driver-ffi query_returns_unimplemented`
- 结果：
  - `mobile_contract_test`: `4 passed; 0 failed`
  - `query_returns_unimplemented`: unit test、`api_smoke`、`mobile_contract_test` 过滤命中项均 `ok`

## 最终 cargo check

- 命令：
  - `cargo check -p helix-driver-ffi`
- 结果：
  - `Finished 'dev' profile [unoptimized + debuginfo] target(s) in 1.24s`

## 变更文件

- `crates/helix-driver-ffi/src/api.rs`
- `crates/helix-driver-ffi/tests/mobile_contract_test.rs`
- `.superpowers/sdd/task-3-report.md`

## 自检结论

- 仍保持公开业务面为 `name + JSON payload`，未新增每用例一个 C ABI 符号
- 只开放本地元信息查询，没有把 IM 表 / schema 语义塞进 `helix-driver-ffi`
- 未触碰移动端 C++ 桥接、QuickJS 封装、QuickJS 冒烟或移动端编译产物
- `im:post:send-failed` 未被加入移动端 P0 能力断言，未知查询契约仍保守
- 改动只落在指定写入范围内

## 疑虑

- `helix_version` 当前按 brief 返回固定 `{"abi":1}`；若后续 ABI 版本改由构建元数据驱动，需要同步补一个单点真源，避免常量漂移
