# InteractionFrame V2 按 UC 当前源码索引

> 2026-07-24 已按 Helix、cses-client MessageV4 当前源码重建。原 19 组 V1/V2 对照图与逐 UC `Spec/Plan/Tasks` 是切换期材料；生产链已 V2-only 后不再作为当前合同保留。历史实施任务仍由 [plan.md](plan.md)、[tasks.md](tasks.md) 与 [verification.md](verification.md) 保存。

当前实现合同入口：

- [源码核验与版本](../../docs/interaction-frame-v2-全量替换-按UC/00-源码核验与版本.md)
- [按 UC 当前代码矩阵](../../docs/interaction-frame-v2-全量替换-按UC/01-按UC当前代码矩阵.md)
- [V2 当前公共架构](../../docs/interaction-frame-v2-全量替换-按UC/02-V2当前公共架构.md)
- [协议版本边界](../../docs/interaction-frame-v2-全量替换-按UC/03-协议版本边界.md)
- [验收证据边界](../../docs/interaction-frame-v2-全量替换-按UC/04-验收证据边界.md)

<!-- GATE_MATRIX_START -->
| UC | 当前机器 Gate | 当前源码合同 | 历史全局 Task |
|---|---|---|---|
| GS01 | G-S1 | [GS01/README](../../docs/interaction-frame-v2-全量替换-按UC/GS01/README.md) | `T021-T023` |
| GS02 | G-S2 | [GS02/README](../../docs/interaction-frame-v2-全量替换-按UC/GS02/README.md) | `T024-T026` |
| GS03 | G-S3 | [GS03/README](../../docs/interaction-frame-v2-全量替换-按UC/GS03/README.md) | `T027-T029` |
| GS04 | G-S4 | [GS04/README](../../docs/interaction-frame-v2-全量替换-按UC/GS04/README.md) | `T030-T032` |
| G01 | G-01 | [G01/README](../../docs/interaction-frame-v2-全量替换-按UC/G01/README.md) | `T033-T035` |
| G02 | G-02 | [G02/README](../../docs/interaction-frame-v2-全量替换-按UC/G02/README.md) | `T036-T038` |
| G04 | G-04/G-04a | [G04/README](../../docs/interaction-frame-v2-全量替换-按UC/G04/README.md) | `T039-T041` |
| G05 | G-05 | [G05/README](../../docs/interaction-frame-v2-全量替换-按UC/G05/README.md) | `T042-T044` |
| G06 | G-06 | [G06/README](../../docs/interaction-frame-v2-全量替换-按UC/G06/README.md) | `T045-T047` |
| G07 | G-07 | [G07/README](../../docs/interaction-frame-v2-全量替换-按UC/G07/README.md) | `T048-T050` |
| G08 | G-08 | [G08/README](../../docs/interaction-frame-v2-全量替换-按UC/G08/README.md) | `T051-T053` |
| G09 | G-09 | [G09/README](../../docs/interaction-frame-v2-全量替换-按UC/G09/README.md) | `T054-T056` |
| G10 | G-10/G-10a | [G10/README](../../docs/interaction-frame-v2-全量替换-按UC/G10/README.md) | `T057-T059` |
| G11 | G-11 | [G11/README](../../docs/interaction-frame-v2-全量替换-按UC/G11/README.md) | `T060-T062` |
| G12 | G-12 | [G12/README](../../docs/interaction-frame-v2-全量替换-按UC/G12/README.md) | `T063-T065` |
| G13 | G-13 | [G13/README](../../docs/interaction-frame-v2-全量替换-按UC/G13/README.md) | `T066-T068` |
| G14 | G-14/G-14a | [G14/README](../../docs/interaction-frame-v2-全量替换-按UC/G14/README.md) | `T069-T071` |
| G15 | G-15/G-15a | [G15/README](../../docs/interaction-frame-v2-全量替换-按UC/G15/README.md) | `T072-T074` |
| G16 | G-16/G-16b/G-16c | [G16/README](../../docs/interaction-frame-v2-全量替换-按UC/G16/README.md) | `T075-T077` |
<!-- GATE_MATRIX_END -->

## 机器校验

```bash
node scripts/refresh_interaction_frame_v2_docs.mjs
python3 scripts/verify-interaction-frame-v2-index.py
```

通过条件：

- 19 个 UC 目录各只保留一份当前源码 `README.md`。
- 19 行索引精确覆盖 25 个当前机器 Gate。
- `G-03/G-03a` 不属于本次 19 UC 切换范围；旧目录 `G17` 不得冒充当前机器 Gate `G-17`。
- 全局历史任务仍连续且唯一覆盖 `T021-T077`。
