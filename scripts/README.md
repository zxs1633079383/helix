# Helix Scripts

This directory keeps only durable entrypoints. One-off UC probes should live in
docs, workflow notes, or `host-cli-session.sh` command snippets, not as new
top-level scripts.

## Core Gates

- `gate-fast.sh` - scope-aware local/pre-push gate; scripts and low-risk crates run only relevant checks.
- `gate.sh` - full local/pre-push delivery gate for native + FFI；GitLab CI 主入口改为 MessageV3 real-chain。
- `branch-identity.sh` - branch identity report for SESSION handoff.
- `branch-sync-gate.sh` - branch/upstream drift guard.
- `ddd-structure-gate.sh` - DDD/package boundary structure gate.
- `trace-static-gate.sh` - correlation 与静态探针覆盖检查。
- `check-downstream-helix-pins.sh` - 下游 Helix source pin 对账。
- `coverage.sh` - Rust coverage entrypoint.
- `fullmap-coverage.sh` - IM migration coverage table check.

## Runtime And FFI

- `dev-loop.sh` - canonical local dev loop for host-cli against Go.
- `host-cli-session.sh` - canonical persistent host-cli session helper.
- `host-cli-FULL-uc-e2e.sh` - canonical full UC e2e sweep.
- `host-cli-dual-account-e2e.sh` - dual-account/broadcast e2e scenarios.
- `messagev3/phase1-real-chain.mjs` - Phase1 `Domain → Gate` three-repo runner; internally targets Go `127.0.0.1:8066`, invokes Angular real WDIO classes, and fails closed when HTTP/WS/Native/DOM/storage evidence is incomplete.
- `messagev3/phase1-5-real-chain.mjs` - Helix Phase1-5 的 83 个可执行 Gate 串行真链入口；支持 `--env`、`--phase`、`--domain`、`--gate`，并要求同轮 HTTP/WS、SQLite、EventSink 与清理证据才记 PASS。`NOT_APPLICABLE` 与 `NOT_IMPLEMENTED` 的 9 个 Gate 不进入执行分母。
- `messagev3/ci-diagnose.mjs` - PRE 的 DNS/TCP、Go health 与首账号 Java 登录限时诊断；任一探针失败即非零退出，不能代替业务 Gate 验收。
- `messagev3/real-chain-environment.mjs` - real-chain 环境 profile；默认 `pre`，内置 `local`，可用 `MESSAGEV3_GO_HTTP_BASE`、`MESSAGEV3_GO_WS_URL`、`MESSAGEV3_JAVA_API_BASE` 覆盖端点，未来环境可在变量齐全时直接扩展。
- `ffi-verify.sh` - Flutter/FFI export and boundary validation.
- `cpp-smoke.sh` - C++ FFI smoke check.

### MessageV3 real-chain 环境

`MESSAGEV3_ENV` 默认是 `pre`。CI 使用 pre 的 Go HTTP、Go WS 和 Java API；本地运行可切换
到同一配置表中的 `local`，或为未来环境同时提供三类端点：

```bash
# PRE（默认，CI 同款）
node scripts/messagev3/phase1-5-real-chain.mjs --all
# 本地 Go/WS/Java
MESSAGEV3_ENV=local node scripts/messagev3/phase1-5-real-chain.mjs --all
# 自定义环境（环境名未登记时必须同时提供三类端点）
MESSAGEV3_ENV=dev \
MESSAGEV3_GO_HTTP_BASE=https://api-im-dev.example/api/cses \
MESSAGEV3_GO_WS_URL=wss://ws-im-dev.example/api/v4/websocket \
MESSAGEV3_JAVA_API_BASE=https://api-dev.example \
node scripts/messagev3/phase1-5-real-chain.mjs --all
```

### MessageV3 CI 诊断与限时

当前 [`.gitlab-ci.yml`](../.gitlab-ci.yml) 默认运行完整 83 Gate；设置 `MESSAGEV3_DIAGNOSE=1`
时排除完整作业，改为 `messagev3-diagnose`：先执行 `ci-diagnose.mjs`，成功后仅运行
`phase1-5-real-chain.mjs --gate MV3-G07B --env pre`。前置探针失败会停止该作业，不执行后续 Gate。

- 两个 job 均为 `timeout: 6m`，`RUNNER_SCRIPT_TIMEOUT=5m`；这是仓库声明，Runner/项目运行态须另查。
- runner 的每个 Cargo 批次默认限时 `180000` ms；`MESSAGEV3_CARGO_TIMEOUT_MS` 可覆盖，解析值
  小于 `30000` ms 时回退默认。Unix 超时向本批进程组发 SIGTERM，5 秒后仍未关闭则发 SIGKILL，
  收口退出码为 124；Windows 使用子进程 kill。
- runner 实时透传 Cargo stdout/stderr，并输出前置检查、Phase/Gate 起止、证据校验和汇总进度；
  Cargo 日志保留在本轮 artifact。阶段日志或编译成功不代表业务 Gate PASS。
- 诊断逐项记录到 `artifacts/messagev3-diagnose/probes.jsonl`；单项探针限时 8 秒、TCP 5 秒、
  HTTP 7 秒，只记录状态/错误与 DNS/TCP 信息，不保存认证响应。诊断 job 的 artifact 保留 7 天，
  完整 job 的 artifact 保留 14 天。
- PRE 默认由 `real-chain-environment.mjs` 注入内置测试账号池；完整 runner 支持
  `CSES_WDIO_CREDENTIALS_JSON` 优先、File 覆盖其次，非 PRE 不注入默认池。
  `ci-diagnose.mjs` 的登录探针固定使用内置首账号，不读取该 JSON/File 覆盖。

## Setup And Shared Config

- `install-hooks.sh` - install repository hooks.
- `lib-config.sh` - shared shell config loader for scripts.

## Crates.io 发布

- `publish_crates.py --plan`：在内存中生成完整 workspace 候选版本图，检查所有公共 crate 的
  精确 path 依赖、双许可证、当前版本可解析和目标版本占用，不修改文件。
- `publish_crates.py --publish`：默认将四个公共 crate 的 patch 版本加一，同步内部精确依赖，
  编译后按 `cses-helix-core → helix-im → helix-driver-host → helix-driver-native` 顺序执行
  dry-run、上传与 registry 可解析确认。
- 版本事务同时覆盖 `helix-driver-web`、`helix-driver-ffi`、`helix-host-cli` 与 `helix-perf`：
  它们只同步对公共 crate 的精确依赖，自身 `package.version` 不递增，也不会上传 crates.io。
- 发布包集合与版本事务集合相互独立；workspace 新增未登记的公共 crate 精确 path 依赖时，
  `--plan` 会 fail-closed，避免进入真实发布后才由 Cargo 解析报错。
- 发布前同时要求当前四包均可解析、目标四包均未占用；若上一次处于 partial，必须先人工收敛，
  不会自动跳版本掩盖断层。
- `--bump minor|major` 可改变递增级别，`--version X.Y.Z` 可显式指定版本。真实发布始终要求
  tracked 工作树干净，且待发布 crate 内不存在未跟踪文件；脚本不执行 commit、tag 或 push。
- 首个 crate 上传前失败会恢复 8 个事务 manifest 与 `Cargo.lock`；已有 crate 上传后失败会保留
  新版本并输出 `PARTIAL`，因为 crates.io 已发布版本不可覆盖。

## Entropy Rule

Do not add a new `host-cli-uc*.sh` for a single bug or historical probe. Add the
scenario to `host-cli-FULL-uc-e2e.sh` if it is a durable regression, or document
the command sequence under `docs/harness/` / `docs/review/` if it is evidence.
