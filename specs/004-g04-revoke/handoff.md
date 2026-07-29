# G04/G04a Handoff

## 完成状态

**FULL GREEN**。撤回与系统事件已完成 Helix -> Go -> Native/Tauri -> Angular Message-v4 的双账号真实链路，任务清单 12/12 完成，无 required fail/skip。

## 纵切合同

- ActionBinding：只暴露 viewer authority 允许的 live binding，actionRef/causationId 全程关联。
- Tick/Effect：UiEvent 经 ActionDispatch 进入确定性 Tick，HTTP/WS 与失败分支均有结构化证据。
- Persist：远端 PostgreSQL 与 444/678 本地 SQLite 同窗对账；业务终态只在 PersistOk/PersistAtomic 后发布。
- Surface/VM：完整 render-ready row 进入对应 Surface，无 raw event/legacy reducer 回退。
- InteractionFrame：scope/epoch/revision/causation 匹配后 whole-frame apply，PC 可稳定消费。
- Angular/UI：Message-v4 组件、DOM/ARIA、data-* 与 Index-Dual 交互状态已在 final Native 中验证。

## 最终证据

- Suite：`mrc-g04-g16-final-a2f835f-1784780000`，单拓扑 19 Gate，耗时 186808ms。
- Gate：`G-04, G-04a`，均为 `runtimeStatus=pass`、`canonicalStatus=green`。
- 结果：`/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/.artifacts/mrc-g04-g16-final-a2f835f-1784780000/scenarios/<gate>/result.json`。
- SHA-256：`f50581d3... / e70ad5cc...`（完整值见同目录 `result.sha256`）。

## 未决项

无。本 handoff 只声明 final Native 证据；FFI/Web 的平台专项验收不冒充本 Gate 结果。
