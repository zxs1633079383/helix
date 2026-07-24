# Mobile Frame V2 全量切换 Spec

## 目标

将 25 个机器 Gate 从 Venus/QuickJS raw-event V1 消费切换为唯一
`InteractionFrameV2 -> MobileImSdk -> SurfaceRuntime -> readonly QuickJS Surface VM` 链路。

## 需求

- 逐 Gate 冻结 V1 当前类图、流程、数据和旁路。
- 复用 PC Gate 的 Helix/Go/存储合同，不在 Mobile 复制领域决策。
- 首轮 E2E 前同时切断 V1 producer 与 consumer。
- 首轮纯 V2 后才物理删除 V1，随后使用新 candidate 二次验收。
- 实际页面缺失或制品/合同漂移必须 fail-closed。
