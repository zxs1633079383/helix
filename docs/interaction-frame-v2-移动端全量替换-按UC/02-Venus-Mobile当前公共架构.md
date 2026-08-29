# Venus/Mobile 当前公共架构

> ⚠️ `RETIRED_HISTORICAL`：本文冻结 2026-07 的 V2 迁移快照，外部 Mobile checkout
> 已变化；不得作为当前 MessageV3 实现或完成证据。

当前存在两条未完全收敛的实现面，另有一组尚未提交的迁移代码：

1. 生产 Venus：`globalThis.im -> VenusImApi -> HelixImDriverRuntime -> helix_command`，返回方向按
   raw DomainEvent 广播到每页 listener，由 JS 更新页面。
2. 已提交迁移地基：`MobileImSdk + SurfaceRuntime + quickjs/bind_mobile_im` 支持 attach/resync/ack、
   action ledger 和只读 `im.surface.view/pending`，但当前正式类型仍以 V1 frame shape 为主。
3. 当前 dirty 工作树：开始接受 `schemaVersion=2` 的 rich VM、把 attach/resync 映射为
   `__helix_request_surface_snapshot`、并在 hello/sync/query 完成后重发 snapshot 请求；这些改动
   尚未形成正式 V2 envelope consumer，也不是 immutable candidate。

证据：

- [Venus 全链说明](/System/Volumes/Data/workspace/c/mobile-qucik-c++/docs/Venus调用Helix全链路与优化指南.md)
- [进程级 driver](/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/HelixImDriverRuntime.cpp)
- [Surface 类型与双版本常量](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.hpp)
- [Frame decoder](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp)
- Mobile SDK frame/action/resync：历史路径
  `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp`，当前 checkout 不存在。
- QuickJS 只读 Surface：历史路径
  `/System/Volumes/Data/workspace/c/mobile-qucik-c++/quickjs/bind_mobile_im.cpp`，当前 checkout 不存在。

当前边界判定：`CURRENT_PARTIAL`。schema 数字为 2 不能证明正式 V2 envelope 已接通；Venus raw
event 广播、V1 decode、legacy reducer 或实际页面缺失任一存在，都不能进入纯 V2 candidate。
