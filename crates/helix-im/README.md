# Helix IM

`helix-im` 是运行在 Helix Core 上的确定性 MessageV3 IM 业务模块，负责消息领域状态机、
持久化屏障、恢复与业务事件生成，不包含平台 UI 或具体 I/O 实现。

```toml
[dependencies]
helix-im = "=0.1.9"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。
