# Handoff: G01 WDIO 前五阶段验收

## 1. 当前结论

- Feature：`019-g01-wdio-five-phases`
- Task：T001-T070 全部完成。
- V-001 至 V-008：全部登记 PASS。
- Mandatory loop：R1/R2/R3 重新校验 PASS，R4 evidence gate
  `event_seq=29` PASS，convergence 无新增任务，`event_seq=31` 返回
  `CLOSED`。
- 被测服务是本地
  `/System/Volumes/Data/workspace/golang/cses-im-server`，监听
  `127.0.0.1:8066`；Telepresence 只提供 PG/Redis/Pulsar 依赖网络。

## 2. 已完成能力

- test-only WDIO/Tauri capability 与 production boundary 已隔离。
- UI-only、Helix 真源 V2 Mock、IPC reject、real invoke、single-real、
  dual-real 五阶段均通过。
- Actor 444 与 Observer 678 使用隔离 profile、auth、device、端口和日志目录。
- single/dual 真实链关联本地 Go、WS、Helix、SQLite、只读 PG、
  Frame/FrameApplied、DOM 和截图。
- G01 canonical 入口已改走 `WdioMessageUiDriver` 与共享
  `submitG01TextThroughUi` operation，G01 direct raw reference 为 `0`。
- 非 G01 仍有 15 个共享发送 helper 引用，因此共享 desktop driver 保留。
- Report V1、manifest、Spec/JUnit/JSON/Markdown、首次失败附件、退出码、
  性能 reducer 和 artifact scan 已闭合。

## 3. 最终证据

最终 real-chain run：

- Mock `mock-20260724235459401-50059`
- Integration `integration-20260724235617280-50859`
- Single real `single-real-20260724235902280-52084`
- Dual real `dual-real-20260725000022820-52638`
- Report `report-20260725000223162-54259`

Fresh warm performance：

- UI：P95 `1194ms`，预算 `10000ms`
- Mock：P95 `1667ms`，预算 `10000ms`
- Single real：P95 `3085ms`，预算 `15000ms`
- Dual real：P95 `19311ms`，预算 `30000ms`
- 四条 lane 均 `profileRestarts=0`。

最终 focused 验证：

- Helix G01 fixture contract：1/1 PASS。
- cses-client TypeScript：PASS。
- WDIO contracts：21 suites / 61 tests PASS。
- production boundary：PASS。
- 双仓 `git diff --check`：PASS。
- canonical、performance 与 fresh failure proof artifact scan：PASS。

## 4. 失败证据与修复

- V-004 首次因调用进程未注入 `CSES_WDIO_PG_READONLY_DSN` 失败；原始失败
  输出保留，retry 1 使用安全注入的只读连接后 PASS。
- V-008 扫描该失败报告时发现 WDIO 官方 JUnit 把绝对源码路径写入 XML。
  已增加 `junit-sanitizer.ts` 和合同测试，并用
  `g01-redaction-failure-proof-20260725` 重新证明失败报告也可通过脱敏扫描。
- embedded provider 仍会打印 standalone `tauri-driver not found` 诊断，
  但实际 session PASS；不要安装 standalone driver 改变 provider 行为。

## 5. 两仓状态与边界

- Helix：`main`，工作树同时存在其他文档任务改动；不得 reset 或覆盖。
- cses-client：`feat/helix-reactor-im`，本 Feature 改动尚未提交；不得 reset、
  checkout 或混入无关提交。
- 不输出账号密码、auth 文件、DSN 或 Kubernetes Secret。
- 本 Feature 只证明 G01 WDIO 前五阶段，不冒充 GS01-GS04 或 G02-G16
  全部验收完成。

## 6. 恢复入口

1. `verification.md`
2. `tasks.md`
3. `evidence/V-001-production-boundary.log` 至
   `evidence/V-008-redaction-retry-2.log`
4. `review-ledger.json`
