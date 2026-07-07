# Superpowers 文档入口

本目录保留 spec / plan 的设计过程材料。它们是 provenance，不是 runtime green。

## 读取顺序

1. 先看仓库根 `docs/README.md` 和模块 README，确定当前 canonical 文档。
2. 再到 `specs/` 找设计意图。
3. 最后到 `plans/` 看执行拆分。

## 当前仍有参考价值的 spec

| 文档 | 主题 |
|---|---|
| `specs/2026-07-05-multiplatform-otel-trace-design.md` | 四仓 OTel trace 设计 |
| `specs/2026-07-06-multitenant-local-db-real-validation-design.md` | 多租户本地库隔离验证 |
| `specs/2026-07-01-helix-im-ddd-package-layering-design.md` | helix-im DDD 分包设计 |
| `specs/2026-06-18-helix-flutter-ffi-demo-design.md` | Flutter / FFI demo 设计，引用前需核对 driver-ffi 当前契约 |

## 历史计划说明

- `plans/*.md` 是执行计划或旧任务拆分，不自动代表已完成。
- `.superpowers/sdd/*.md` 是 worker brief/report/progress，不进入当前事实链。
- 出现 `DONE`、`PASS`、`green` 字样时，必须回到 reviewer / collector / gate 证据验证。
