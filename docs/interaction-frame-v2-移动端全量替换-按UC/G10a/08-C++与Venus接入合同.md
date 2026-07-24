# G10a C++ 与 Venus 接入合同

```text
Venus page lifecycle
  -> one MobileImSdk + SurfaceRuntime binding per page/context
  -> process CoreBridge/Helix driver
  -> `im_query_messages_by_channel` / `im_load_older_context`
  -> on_domain_events_batch
  -> MobileImSdk::pollBridgeEvents
  -> SurfaceRuntime::ingest
  -> typed VM
  -> QuickJS readonly surface
```

V2 canonical runtime 是 `MobileImSdk + SurfaceRuntime + quickjs/bind_mobile_im`。Venus 只保留：

- 创建/销毁页面 binding；
- owning-thread pump；
- 有界 batch queue；
- 动态库与 header 生命周期；
- renderer registry 和具体移动 UI。

证据入口：

- [cpp/MobileImSdk.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/MobileImSdk.cpp)
- [cpp/SurfaceRuntime.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp)
- [quickjs/bind_mobile_im.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/quickjs/bind_mobile_im.cpp)
- [venus/HelixImDriverRuntime.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/HelixImDriverRuntime.cpp)
- [venus/api/ImPostsApi.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/venus/api/ImPostsApi.cpp)
