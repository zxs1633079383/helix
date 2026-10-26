# G01 WDIO Acceptance Contract

## Lane Contract

| Lane | Topology | Mandatory assertions | Result level | Forbidden claim |
|---|---|---|---|---|
| `ui` | Actor WKWebView | Surface ready、Composer input/submit、selector/CSS/尺寸、失败截图 | `ui-only` | command/Helix/Go/DB |
| `mock` | Actor WKWebView | receipt payload/calls、V2 decode、sending→sent/failed、IPC reject 分离、DOM | `mock` | Rust command/Go/DB |
| `integration` | Actor WKWebView | Mock=0、真实 health/query/invoke、AcceptedCommand、Tauri/Helix、matching Frame/DOM | `integration` | Observer；缺 Go/PG 时不可称 real |
| `single-real` | Actor WKWebView + real backend | Actor UI、Go HTTP、SQLite、PG、Frame、FrameApplied、Actor DOM | `single-real-chain` | Observer consume |
| `dual-real` | Actor 444 + Observer 678 | Go、WS、SQLite、PG、Frame、FrameApplied、双 DOM、双 fresh screenshot | `dual-real-chain` | GS01-GS04 |

## Required G01 Cases

| Case ID | Lane | Input | Terminal assertion |
|---|---|---|---|
| `G01-UI-READY` | ui | unique marker | ready/input/submit/disabled contract |
| `G01-MOCK-SENT` | mock | receipt + sending/sent fixture | temporary identity 收敛为 sent |
| `G01-MOCK-FAILED` | mock | receipt + sending/failed fixture | failed 来自 matching Frame |
| `G01-IPC-REJECTED` | mock | invoke reject once | `ipc-rejected`，无业务 failed Frame |
| `G01-INTEGRATION` | integration | real health/query/send | Rust/Helix 执行且 Mock call=0 |
| `G01-SINGLE-REAL` | single-real | real Actor send | authority/storage/frame/Actor DOM |
| `G01-DUAL-REAL` | dual-real | Actor send/Observer receive | 同一 canonical post 双端呈现 |
| `G01-REPORT-PASS-FAIL-RETRY` | report | injected results | 报告、附件、退出码、flaky 一致 |

## Correlation Minimum

必须可从 `runId + gate + persona` 限定窗口，并以至少一条无歧义链连接：

```text
reqId -> trackId -> channelId + markerHash
      -> temporaryIdHash -> postId -> eventSeq
      -> transactionId -> FrameApplied -> DOM message identity
```

动态字段缺失允许由另一组键补齐，但不得只靠时间接近或消息正文匹配。

## Failure Escalation

成功路径只写 summary 与该 lane 必需证据。任何 assertion/latch/Frame/log/network 失败时：

1. 保存首次失败与 stage timing。
2. 采当前 persona screenshot、visible DOM summary、console 和 Tauri/Helix bounded window。
3. real lane 再采 Go/WS、SQLite/PG bounded evidence。
4. dual lane 保持健康 persona 现场，只恢复失败 persona。
5. 同签名最多自动修复两次；第三次 `NeedsHuman`。

## Timing

- 普通 DOM wait：5s。
- command/Frame wait：10s。
- 单端真实投递：15s。
- 双端完整场景：30s。
- 禁止固定 sleep；超时必须报告等待的 key、plane 和阶段。

## G01 Raw Migration

等价比较必须覆盖 selector、action、DOM identity/delivery、correlation 字段和 artifact schema。通过后 G01 scenario 的 raw client 引用计数必须为 0。共享 raw driver 仍被其他 Gate 使用时保留，禁止因本 Feature 物理删除。
