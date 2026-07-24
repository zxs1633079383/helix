# Venus/Mobile 当前公共架构

当前存在两条未完全收敛的实现面：

1. 生产 Venus：`globalThis.im -> VenusImApi -> HelixImDriverRuntime -> helix_command`，返回方向按
   raw DomainEvent 广播到每页 listener，由 JS 更新页面。
2. 迁移地基：`MobileImSdk + SurfaceRuntime + quickjs/bind_mobile_im` 已支持 V2 Frame、
   attach/resync/ack、action ledger 和只读 `im.surface.view/pending`，但尚未成为 Venus 唯一生产 runtime。

证据：

- [Venus 全链说明](/System/Volumes/Data/workspace/c/mobile-qucik-c++/docs/Venus调用Helix全链路与优化指南.md:10)
- [进程级 driver](/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/HelixImDriverRuntime.cpp:89)
- [V2 Surface 类型](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.hpp:15)
- [V2 Frame ingest](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp:2127)
- [QuickJS 只读 Surface](/System/Volumes/Data/workspace/c/mobile-qucik-c++/quickjs/bind_mobile_im.cpp:2028)
