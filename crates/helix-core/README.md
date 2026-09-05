# CSES Helix Core

`cses-helix-core` 是 Helix 的运行时无关确定性业务内核，提供 `Tick`、`Effect`、ports、
Module 宿主与 sans-IO 执行壳。它不直接依赖 Tokio、数据库、HTTP、Web 或 Tauri。

```toml
[dependencies]
cses-helix-core = "=0.1.9"
```

Rust crate 名保持为 `helix_core`：

```rust
use helix_core::{Effect, Tick};
```

本项目采用 `MIT OR Apache-2.0` 双许可证。
