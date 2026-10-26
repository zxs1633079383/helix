# Helix Native Driver

`helix-driver-native` 是 Helix 的 Tokio Native 平台驱动，用于 PC/Tauri 等原生宿主。

```toml
[dependencies]
helix-driver-native = "=0.1.87"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## 宿主注入的离线事实
候选 NativeHttp/SharedHttpClient/上传器可选择注入 request guard，默认不启用，其他宿主行为保持原合同。NetworkUnavailable 只表示 I/O 前明确拒绝，普通 Network/Timeout 保持结果未知；不取消已发请求、不重放写操作。NativeTransport 从同一网络 watch 获取事实，重连生命周期继续首次尝试、三次快速重试后低频恢复。命令能力与完整别名清单由 helix-im client_api 动态提供，本地/缓存流程不被接口 Offline 一刀切阻止。
