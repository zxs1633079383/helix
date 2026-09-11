# CSES Helix Core

`cses-helix-core` 是 Helix 的运行时无关确定性业务内核，提供 `Tick`、`Effect`、ports、
Module 宿主与 sans-IO 执行壳。它不直接依赖 Tokio、数据库、HTTP、Web 或 Tauri。

```toml
[dependencies]
cses-helix-core = "=0.1.18"
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
