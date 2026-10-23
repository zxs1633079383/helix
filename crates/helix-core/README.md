# CSES Helix Core

`cses-helix-core` 是 Helix 的运行时无关确定性业务内核，提供 `Tick`、`Effect`、ports、
Module 宿主与 sans-IO 执行壳。它不直接依赖 Tokio、数据库、HTTP、Web 或 Tauri。

```toml
[dependencies]
cses-helix-core = "=0.1.85"
```

Rust crate 名保持为 `helix_core`：

```rust
use helix_core::{Effect, Tick};
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## 存储规格入口

`UpsertSpec` 除表、行与冲突键外，还提供 `exclude_from_update`、`update_guard`
和 `version_column`；字段合同与默认值见 [`src/effect/storage.rs`](src/effect/storage.rs)。
SQLite 兑现见 [`host storage/write.rs`](../helix-driver-host/src/storage/write.rs)，
守卫回归见 [`host tests/upsert_guard.rs`](../helix-driver-host/tests/upsert_guard.rs)。

## 宿主注入的失败诊断

[`ExecutionShell::step_with_trace`](src/engine.rs) 接收可选 `TraceId`，普通 `step`
等价于传入 `None`；宿主在 step 后通过 `diagnostics()` 借用当前轮失败，下一轮清空复用。
[`diagnostics`](src/diagnostics.rs) 只定义纯数据与有界缓冲，不读取 ambient span 或依赖 OTel。
`TraceId::from_hex` 只接受 32 位小写十六进制非零标识；未注入合法 ID 时，
`StepDiagnostics::report` 不保存失败。每批最多 16 条、错误文本合计最多 4096 UTF-8 字节，
超额记录省略数；`FailureDiagnostic` 以 `Arc<str>` 共享已截断文本。

诊断单测入口：`cargo test -p cses-helix-core diagnostics::tests`。本次仅核对源码，未执行 Rust 测试。
