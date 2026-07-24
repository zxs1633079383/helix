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

InteractionFrame V2 与 InteractionProtocol/v1、ActionInvocationEnvelope/v1 是独立版本域。
