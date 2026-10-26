# Mobile QuickJS/C++ 协议与术语

| 名称 | 定义 |
|---|---|
| Venus page binding | 每页 QuickJS context 的 host adapter，不拥有业务状态 |
| MobileImSdk | typed Mobile façade；V2 下只转发 action、frame、ack/resync |
| CoreBridge | 唯一直接持有 Helix handle 的 C++ bridge |
| SurfaceRuntime | 校验 epoch/revision/causation 并原子提交 typed VM |
| accepted | 命令进入有界 ingress，不是业务完成 |
| action receipt | pending/拒绝回执，不替代 matching causation Frame |
| FrameApplied | whole-frame + directive 提交后的确认 |
| raw business fallback | C++/JS 从 raw `im:*` 重建业务 UI，V2 candidate 必须为 0 |
| snapshot request | attach/resync 发给 Helix 的私有 `__helix_request_surface_snapshot`，不是业务 command |
| CURRENT_PARTIAL | 部分类型/fixture/adapter 已存在，但生产链仍含 V1 或 raw fallback |
| SOURCE_MISSING | 当前仓无法定位真实产品页面/renderer，不能推断 UI 已完成 |

InteractionFrame V2 与 InteractionProtocol/v1、ActionInvocationEnvelope/v1 是独立版本域。
