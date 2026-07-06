# Task 4 Report

## Changed Files

- `crates/helix-im/src/send/retry_upload.rs`
- `crates/helix-im/src/send/mod.rs`
- `crates/helix-im/src/module.rs`
- `crates/helix-im/tests/rich_file_upload_contract_test.rs`
- `crates/helix-im/tests/common.rs`
- `SESSION.md`

## Behavior

- 新增命令 `im_retry_upload`，支持：
  - `{ "temporary_id": "...", "target": { "kind": "rich_image", "index": n } }`
  - `{ "temporary_id": "...", "target": { "kind": "file" } }`
- 从 `pending_sends[temporary_id].body.props` 读取缓存上传配置，只为请求目标重建一个 `FileUploadRequest`。
- retry 前要求目标当前 `upload.status == "failed"`；否则返回 `ImError::Parse`，不发副作用。
- retry step 只插入一个新的 `pending_uploads[corr]`，只发一条 `Effect::UploadFile`，不直接发 `posts/create`。
- retry 时把目标的内存态 `upload.status` 回拨到 `uploading`，并按 props 中剩余失败项重算 `PendingSend.upload_failed`，让后续 upload reply 继续走既有成功/失败回流；当最后一个失败目标重试成功时，既有 reply 处理会补发 `posts/create`。

## Tests

- `cargo test -p helix-im retry_failed -- --nocapture`
  - 结果：3 passed / 0 failed
- `cargo test -p helix-im --test rich_file_upload_contract_test -- --nocapture`
  - 结果：9 passed / 0 failed

## Concerns

- 当前 retry 只更新 `PendingSend.body` 里的内存 props，不会在 retry 发起瞬间把数据库里的 `upload.status` 从 `failed` 持久化回 `uploading`；直到 retry 成功/失败回包时，才由既有 upload reply 路径持久化最终状态。这符合本任务“retry step 只发一条 UploadFile”的约束。
