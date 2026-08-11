# MessageV3 Phase1 三仓真实测试矩阵

本测试包把 Phase1 按 `Phase1 → Domain → Gate` 分包。Gate 分母不是手写猜测：运行前同时读取 Helix
`message-v3-phase1-send.registry.json`、Angular `phase1.manifest.json` 和 Angular `domain-matrix.mjs`，
三者不是 33/33/33 就立即阻塞。

## 运行入口

在 Helix worktree 执行：

```bash
node scripts/messagev3/phase1-real-chain.mjs --dry-run
node scripts/messagev3/phase1-real-chain.mjs --domain send-core
node scripts/messagev3/phase1-real-chain.mjs --all
node --test tests/messagev3/phase1-real-chain-contract.test.mjs
```

无需配置环境变量。runner 内部固定使用 `http://127.0.0.1:8066`，生成临时的非生产
`release.local.json`，并把下列三个仓库路径传给 Angular Phase1 runner：

| 角色 | 默认路径 | 用途 |
|---|---|---|
| Helix | 当前 worktree | Gate registry、Rust producer/host 证据、运行器与汇总 |
| Go | `/System/Volumes/Data/workspace/golang/cses-im-server-MessageV3-helix` | 当前 Go server source pin；HTTP/WS 服务必须已在 8066 监听 |
| Angular | `/System/Volumes/Data/workspace/angular/visual/cses-client` | 真实 Tauri/WDIO receiver、Native trace、projection 与 DOM |

runner 只会检查 WDIO 端口 4544/4545/4546；端口被占用时不杀进程，结果为
`BLOCKED_PRECONDITION`。Go health 只作为前置探针，`HTTP 200` 或 `status=OK` 不能单独产生业务
`PASS`。

## Domain → Gate → 真实测试类

| Domain | WDIO lane/suite | Gate | Angular 真实类文件 |
|---|---|---|---|
| `channel-setup` | `single-real/single-real-channel-send` | G08a | `tests/e2e/wdio/phase1/specs/real-channel-send.e2e.ts` |
| `send-core` | `dual-real/dual-real-send` | G01a、G01b、G01f、G01k、G02a | `real-channel-send.e2e.ts` |
| `send-core` | `single-real/single-real-fault` | G02b、G02c | `real-send-fault.e2e.ts` |
| `send-core` | `multi-real/multi-real-visibility` | G01l | `real-directed-visibility.e2e.ts` |
| `media` | `dual-real/dual-real-media` | G01c、G01d、G01e、G01g | `real-media.e2e.ts` |
| `media` | `single-real/single-real-media-limit` | G02d | `real-media-limit.e2e.ts` |
| `forward` | `dual-real/dual-real-forward` | G03a、G03b、G03c | `real-forward.e2e.ts` |
| `schedule` | `dual-real/dual-real-schedule` | G04a、G04b、G04c、G04d、G04e | `real-schedule.e2e.ts` |
| `draft` | `dual-real/dual-real-draft` | G02e | `real-draft.e2e.ts` |
| `announcement` | `dual-real/dual-real-announcement` | G07b | `real-announcement.e2e.ts` |
| `post-actions` | `dual-real/dual-real-actions` | G05a、G05b、G05c、G05d | `real-post-actions.e2e.ts` |
| `post-actions` | `multi-real/multi-real-receipts` | G05e | `real-receipts.e2e.ts` |
| `zero-write-and-gaps` | `single-real/single-real-channel-send` | G05l、G01h、G01i、G01j | `real-channel-send.e2e.ts` |

Gate 级 test class 的判据是当前 Angular 源文件中存在对应 `MV3-Gxx` 测试标题/观察写入，且
`phase1-real-chain-contract.test.mjs` 会逐个读取并断言；因此 runner 不会把不存在的类或静态
占位文件当作覆盖。

## 单条发送的证据链

以 `send-core` 为例，真实类必须在同一 run 中观察：

```text
Angular intent
  -> Tauri im_send_message / im_retry_send
  -> Helix Effect::Persist / PersistOk / Effect::Send
  -> Go HTTP（当前 8066）
  -> Go WS post 回调
  -> Native trace projection / Effect::Emit
  -> sender sending -> sent 或 failed
  -> observer DOM/readback
```

runner 从 Angular 生成的同轮 `domains/*/evidence.json` 读取 `commandEvidence`、
`httpEvidence`、`wsEvidence`、`projectionEvidence`、`domEvidence`，缺一就标
`NOT_VERIFIED`。`storageEvidence=UNVERIFIED` 会标 `PARTIAL_STORAGE_READBACK`，不借 HTTP/WS
帧把持久化冒充完成；草稿类的 Host storage restore 可以独立提供 storage 证据。Helix 既有
SQLite/HostStorage 测试仍属于 `FOCUSED`/`integration`，不会伪装成三仓同轮真实链。

## 状态解释

| 状态 | 含义 |
|---|---|
| `OBSERVED` | 选定 Gate 的同轮 HTTP、WS、Native projection、DOM 和所需本地证据均观测到 |
| `PARTIAL_CANONICAL` | 真实链路已观测，但 canonical/跨端 byte-exact 仍未证明 |
| `PARTIAL_STORAGE_READBACK` | HTTP/WS/UI 已观测，持久化行级 readback 未在同一轮证明 |
| `GAP_BOUNDARY_OBSERVED` / `GAP_BLOCKED` | G01h/G01i/G01j 的已知外部能力缺口，不能计入 real Green |
| `NOT_VERIFIED` | 测试类未运行、同轮证据缺失或 child runner 失败 |
| `BLOCKED_PRECONDITION` | Go 8066、凭据、candidate、WDIO 端口或仓库前置不满足 |

每轮汇总写入 `artifacts/messagev3-phase1-real-chain/summary.json`，child WDIO 原始报告仍保留
在其 run 目录；artifact 不写凭据、消息正文或 token。远端群/消息清理保持禁用，只登记资源，
因此真实运行结束后仍需按项目清理合同处理远端残留。
