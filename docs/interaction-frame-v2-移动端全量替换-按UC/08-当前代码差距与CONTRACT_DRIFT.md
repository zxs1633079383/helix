# 当前代码差距与 CONTRACT_DRIFT

| 差距 | 当前证据 | 目标 |
|---|---|---|
| Venus 仍直接广播 raw DomainEvent | `venus/HelixImDriverRuntime.cpp:151-160` | batch 先进入唯一 SurfaceRuntime |
| VenusImApi 仍向 JS 暴露任意 event listener | `venus/VenusImApi.cpp:79-135` | migrated business event 不进入 UI |
| MobileImSdk 同时保留 legacy reducer | `cpp/MobileImSdk.cpp:2300` | typed surface mode 物理删除 fallback |
| C++ 把 schema 2 标在 V1 形状上 | `cpp/SurfaceRuntime.cpp:1695-1709` 只接受根 `schemaVersion/transactionId/mutations`，mutation 无 `surfaceEpoch` | 接受 Helix `InteractionFrameV2` 的 `causationId/runtime/surfaceEpoch/directives` 并整帧校验 |
| C++ Surface 集不完整 | `cpp/SurfaceRuntime.hpp:15-29` 缺 `message.composer` | 九个 Helix Surface 全部有封闭 C++ 类型 |
| Surface VM 仍以 typed wrapper 包 opaque JSON | `cpp/SurfaceRuntime.hpp:48-67` | 生成合同校验完整字段 |
| QuickJS 仍有 `im.store.*` | `quickjs/bind_mobile_im.cpp:2019-2023` | migrated Surface 只读 `im.surface.*` |
| Venus 与 MobileImSdk 两套注入面 | 两套独立源码目录 | Venus 复用唯一 MobileImSdk runtime |
| Android/iOS Helix revision 漂移 | Venus 全链文档 §10 | 同源制品与 manifest gate |
| 实际产品页面源码不在本仓 | 只有 demo/test JS | `SOURCE_MISSING`，实施前绑定真实页面 |

上面两项意味着当前 Mobile 的 `kInteractionFrameSchemaVersion = 2` 不能被当作 V2 consumer
完成证据：版本数字相同，但 wire shape 仍漂移。修复方向是升级 C++ consumer 对齐 Helix V2，
不是让 Helix producer 删除正式字段。
