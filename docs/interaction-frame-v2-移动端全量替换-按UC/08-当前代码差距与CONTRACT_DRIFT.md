# 当前代码差距与 CONTRACT_DRIFT

> 审计时间：2026-07-24 PDT。CURRENT 包含 Mobile 未提交工作树；因此只能作为 Spec
> discovery，不能作为 candidate 完成证据。

| 优先级 | 差距 | CURRENT 源码事实 | TARGET |
|---|---|---|---|
| DONE upstream | Helix 018 producer 已 V2-only | `57d2ddab` 的唯一 `build_interaction_frame/emit_interaction_frame` 接收 `InteractionFrameV2`；schema 常量为 2；V1 type/constant/fixture 已删除 | Mobile pin 并消费该 immutable contract，不重复实现 producer |
| DONE reference | PC 018 consumer 已 V2-only | `846afd3fa` 的 Native ingress 与 Angular codec 仅接受 schema 2，四个 Helix crate pin `57d2ddab` | 仅作 Mobile consumer 行为对照 |
| P0 | Mobile source pin 漂移 | `contract/helix-source.lock.json`、command contract、UC manifest 仍指向 `477ca58` | 同一 `57d2ddab` 或后继 immutable Helix revision 生成全部合同与制品 |
| P0 | C++ 不是正式 V2 envelope consumer | dirty `SurfaceRuntime.cpp::decode` 虽识别 schema 2，但根仍只允许 `schemaVersion/transactionId/mutations`；没有 `runtime/directives`，mutation 没有 `surfaceEpoch` | 精确接受 Rust `InteractionFrameV2`，未知/缺失字段 fail-closed |
| P0 | Surface 集不完整 | `SurfaceRuntime.hpp::SurfaceId` 只有八项，缺 `message.composer` | 九个 Helix Surface 与封闭 VM/mutation 类型齐全 |
| P0 | Mobile V1 consumer 仍生产可达 | dirty header 明确保留 `kLegacyInteractionFrameSchemaVersion=1`，decoder 同时接受 1/2 | 首轮前切断 V1，首轮后物理删除 Mobile legacy |
| P0 | Venus raw 输入仍可达 | `HelixImDriverRuntime` 仍广播 DomainEvent，`VenusImApi` 仍暴露 event listener | migrated business event 只进入唯一 `MobileImSdk + SurfaceRuntime` |
| P1 | dirty snapshot 回路未完成正式合同 | `MobileImSdk` 已开始把 attach/resync 映射为 `__helix_request_surface_snapshot`，并在 hello/sync/query 后重发 | 以 private command + typed Frame/ack 闭环，并覆盖背压/重复/重连 |
| P1 | VM 类型边界仍是 JSON 容器 | C++ 有 distinct wrapper 和 rich 字段校验，但 VM 最终仍以 `std::string json` 保存 | 生成合同负责完整字段校验；renderer 不做业务解释 |
| P1 | QuickJS legacy store 仍存在 | `quickjs/bind_mobile_im.cpp` 同时暴露 `im.store.*` 与 `im.surface.*` | migrated Surface 只读 `im.surface.*` |
| BLOCKED | 真实产品页面缺失 | 当前仓只有 adapter/demo/test，无法定位 19 UC 的正式 Venus 页面 | Spec 实施前登记页面、renderer key、节点和 owner |
| P1 | 制品 manifest 漂移 | 旧文档指向的 `src/include/cses_im/manifest.json` 不存在 | 固定真实 Android/iOS 制品 manifest 与 hash |

关键结论：

1. `kInteractionFrameSchemaVersion = 2` 只证明版本分支存在，不证明 V2 wire 完成。
2. dirty rich-v2 校验属于进行中的 Mobile consumer 扩展，必须精确跟随 Helix `57d2ddab`
   的 `causationId/runtime/surfaceEpoch/directives`，不得降格。
3. 当前 Mobile 已有 ActionInvocation envelope staging 与 private snapshot request 地基，但公开
   `im.*` 业务 command、raw event 与 V1 decode 尚未完成物理清退。
4. 018 删除后二次 Native 回归 `T092-T096` 与 R4 仍未完成；这不推翻静态硬切事实，但禁止
   把 producer/PC consumer V2-only 写成完整产品验收。
5. 正式 Spec 必须先解决 Mobile P0 公共基座，再进入 19 UC 纵切；否则每个 UC 会重复修同一
   decoder、pin、Venus runtime 与证据工具问题。
