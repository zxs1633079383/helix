# Mobile InteractionFrame V2 全量替换

本文档集定义 Venus/QuickJS/C++ 如何消费 Helix `InteractionFrameV2`。范围为 25 个机器 Gate：

- [GS01](./GS01/README.md)
- [GS02](./GS02/README.md)
- [GS03](./GS03/README.md)
- [GS04](./GS04/README.md)
- [G01](./G01/README.md)
- [G02](./G02/README.md)
- [G04](./G04/README.md)
- [G04a](./G04a/README.md)
- [G05](./G05/README.md)
- [G06](./G06/README.md)
- [G07](./G07/README.md)
- [G08](./G08/README.md)
- [G09](./G09/README.md)
- [G10](./G10/README.md)
- [G10a](./G10a/README.md)
- [G11](./G11/README.md)
- [G12](./G12/README.md)
- [G13](./G13/README.md)
- [G14](./G14/README.md)
- [G14a](./G14a/README.md)
- [G15](./G15/README.md)
- [G15a](./G15a/README.md)
- [G16](./G16/README.md)
- [G16b](./G16b/README.md)
- [G16c](./G16c/README.md)

明确排除 `G03/G03a`。共享 Helix/Go/存储合同复用
[PC Frame V2 文档](../interaction-frame-v2-全量替换-按UC/README.md)，Mobile 文档只扩展
Venus、QuickJS、C++、FFI、SurfaceRuntime 与移动 UI 的平台责任。

## 完成顺序

`冻结 V1 -> producer/consumer 双切断 -> 首轮纯 V2 -> 审批删除 -> 物理删除 V1 -> 删除后二次 V2`

`schemaVersion` 字段保留且必须为 `2`。删除的是对 InteractionFrame V1 的生产、消费和兼容支持。
