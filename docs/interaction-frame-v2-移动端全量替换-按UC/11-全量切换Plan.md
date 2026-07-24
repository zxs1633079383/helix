# Mobile Frame V2 全量切换 Plan

1. 固定四仓和 Android/iOS 制品候选。
2. 逐 Gate 复核 V1 baseline 与 PC shared contract。
3. 先完成公共 MobileImSdk/SurfaceRuntime/Venus 接线。
4. 按 GS01-GS04、G01-G02、G04-G16 顺序迁移 25 Gate。
5. 单 candidate 首轮纯 V2：25/25、V1/raw 为 0。
6. 审批并物理删除 producer/consumer V1。
7. 新 candidate 删除后二次 25/25。

实现时 C++/QuickJS 每 Gate只增加平台 adapter 和 renderer 映射，不复制 Helix 业务 reducer。
