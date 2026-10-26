# Quickstart: G01 WDIO 前五阶段

> 以下是实现完成后的稳定入口；任务实现期间不得用临时脚本替代这些合同命令。

## 1. Source And Permission Preflight

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
yarn message:wdio:preflight
```

预期：输出 Node/Rust/macOS 架构、client/Helix pin、测试 build capability、端口、artifact root、Screen Recording 状态和 SQLite/PG readonly 可用性；不输出任何凭据。

## 2. Build Once

```bash
yarn message:wdio:build
```

预期：仅构建 `wdio-e2e` candidate。实现任务中间不反复 cargo；完整测试模块完成后一次 check/build，阶段结束一次测试。

## 3. Single-instance Fast Lanes

```bash
yarn message:wdio:g01:ui
yarn message:wdio:g01:mock
yarn message:wdio:g01:integration
```

- `ui` 只声称 `ui-only`。
- `mock` 覆盖 `sending -> sent`、`sending -> failed` 和 IPC reject，只声称 `mock`。
- `integration` 清理所有 Mock，执行真实 Tauri/Helix，只声称 `integration`；Go/PG 不完整时不得升级。

## 4. Single Real-chain

```bash
yarn message:wdio:g01:single-real
```

预期：Actor 真实 UI、Go HTTP、SQLite、PG、Frame、FrameApplied、Actor DOM 闭合；不声称 Observer 到达。

## 5. Dual-account Real-chain

凭据由隔离 profile 或安全环境注入，命令行不接受明文密码：

```bash
yarn message:wdio:g01:dual-real
```

预期：444 Actor 与 678 Observer 使用不同 profile/device/port/artifact；九面证据和双方 fresh screenshot 完整，预算 30 秒。

## 6. Report Contract

```bash
yarn message:wdio:report-contract
```

预期：注入 pass/fail/retry，校验 Spec 输出、JUnit、JSON/Markdown summary、manifest、附件索引、首次失败、flaky、退出码和脱敏。

## 7. Full G01 Five-phase Acceptance

```bash
yarn message:wdio:g01:all
```

预期顺序：安全入口 → single layered → G01 driver migration → dual real → Report V1。任一 mandatory case 失败立即保留现场；不会宣称 GS01-GS04 Green。

## Artifact Layout

```text
artifacts/message-wdio/<runId>/
├── manifest.json
├── summary.json
├── summary.md
├── junit.xml
├── actor/
├── observer/
└── evidence/
```

只允许写当前 run 目录。SQLite/PG 查询为只读且有界；禁止复制原始数据库、完整日志、authorization、Cookie、token、密码或消息正文。
