# Helix temporary_id 所有权收口报告

- 基线：`9890a8e71e8693429ef64210dc6d92cb77087e4c`
- 完成提交：`77d5f534baacc0d49d9c0e56f1149ae5aeb4ed0d`
- 分支：`main`

## 领域判断

- `TemporaryId` 是发送聚合内部的值对象，不是 Flutter、QuickJS 或 C++ 的输入字段。
- `ImModule` 持有单调分配序号，结合 step 的 `now_ms` 确定性铸造 ID；单个 module 生命周期内每次分配 O(1) 时间、O(1) 额外空间且不重复。
- 公开 command payload 不参与 ID 选择。内部 `allocated_temporary_id` 只服务确定性 replay/test envelope，`client_api` 会删除外部伪造字段。

## 调用链证据

```text
helix_command / Web API
  -> client_api::build_command
  -> normalize_public_semantics
  -> Tick::Command(im_send_message)
  -> ImModule::handle_send_message
  -> TemporaryId::mint
  -> optimistic Persist(message.temporary_id)
  -> im:post:sending
  -> posts/create.temporaryId
  -> PendingSend
  -> WS echo reconcile
```

FFI 已删除自有的 alias/payload builder 复制实现，统一复用 `client_api::build_command`，避免 FFI 与 Web 的归一化规则再次漂移。

## 对外 receipt 语义

1. `helix_command` 返回 `FFI_OK` 只表示命令已入队，不包含 temporaryId，也不表示发送成功。
2. 移动端必须等待 `im:post:sending`，从该投影读取 Helix 生成的 `temporaryId`。
3. C++/QuickJS 再以该投影组装 `{temporaryId,status:"sending"}` optimistic receipt。
4. `im:post:received` 才完成 WS echo 对账；`im:post:send-failed` 与超时 `unsend` 仍由 Helix 状态机产生。

capabilities 已新增 `features.helixOwnedTemporaryId=true`。canonical 命令仍是 `im_send_message`，`im_send` 仅保留 ingress 名称兼容；两者都会丢弃端侧 temporaryId。

## TDD 与验证

- 测试先冻结：无 temporaryId 的 canonical 发送、端侧伪造 ID 被忽略、同一 ID 贯穿 Persist/投影/HTTP、同毫秒连续发送不重复、FFI alias 兼容与 canonical 路径。
- `cargo test -p helix-im --lib client_api::`：PASS，16 passed。
- `cargo test -p helix-im --test send_test`：PASS，14 passed。
- `cargo test -p helix-im --test strict_matrix_contract_test`：PASS，6 passed。首次把负向断言误锚到 G-03，修正为文本发送 G-01 后通过。
- `cargo test -p helix-driver-ffi --test mobile_contract_test`：PASS，6 passed。
- `git diff --check`：PASS。

## 边界

- 未修改 Go Server、移动端仓、SQLite schema 或 FFI C ABI。
- 未运行真实 Go/WS/C++ QuickJS/移动设备链路，不宣称 runtime-pass。
- 当前 ID 的不重复范围是单个 `ImModule` 生命周期；跨 engine session 的全局唯一命名空间应随未来正式 `engine_session_id / IdSource / AllocateId` 合同一起落地，不能靠 UI 恢复生成器。
- G-01 的 `PersistOk -> frame -> HTTP` 分阶段状态机仍是后续工作，本提交只收口 temporary_id 所有权。
- 仓库原有 `.gitignore` 修改与 `HELIX_PHASE0_REPORT.md` 均未修改、未暂存、未提交。
