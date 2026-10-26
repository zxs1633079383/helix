# Helix Phase 0 合同收口报告

- 基线：`deca62769a82f7f3536a0864a3a5f300f83719eb`
- 完成提交：`9890a8e71e8693429ef64210dc6d92cb77087e4c`
- 分支：`main`

## 本次完成

1. 依据 `cses-im-server` 当前 main 的 channel increment、sync notify 与 pong 实现，冻结 G-S1、G-S2、G-S3 的 HTTP response ABI、WS 证据与远端只读边界。
2. 把 HTTP 受理、WS 传输、本地持久化、最终投影拆成独立验收面，并补充“单面成功不得晋升完成”的负向断言。
3. `helix_capabilities.commands` 只公开 canonical `im_send_message`；`im_send` 继续保留为 ingress 兼容别名，但不再广播。
4. capabilities 声明 runtime 已真实发布的 `im:post:send-failed`，同时保留 G10 的 `im:post:increment-failed`。
5. 更新 FFI 移动端合同测试和中文 Command-Query-Event SPEC，明确业务读操作走异步 command/event，`helix_query` 只承载元数据查询。

## 修改文件

- `crates/helix-im/contracts/strict-matrix.v1.json`
- `crates/helix-im/src/client_api/capabilities.rs`
- `crates/helix-im/tests/strict_matrix_contract_test.rs`
- `crates/helix-driver-ffi/tests/mobile_contract_test.rs`
- `crates/helix-driver-ffi/docs/移动端Command-Query-Event契约SPEC.md`

## 验证

- `cargo test -p helix-im --test strict_matrix_contract_test`：PASS，6 passed。
- `cargo test -p helix-driver-ffi --test mobile_contract_test`：PASS，5 passed。
- `git diff --check`：PASS。

## 边界与卡点

- 本提交冻结并校验 Helix 合同，不宣称真实 Go Server、WS、SQLite 或移动设备链路已经 runtime-pass。
- 仓库原有 `.gitignore` 未提交改动已保留，未修改、未暂存、未纳入本提交。
- 无阻塞项；后续 mobile-quickjs 可按本合同进行 repin 与 GS01-GS03 真实链路实现。
