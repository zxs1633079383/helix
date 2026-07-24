# Handoff: G01 WDIO 前五阶段验收

## 1. 当前结论

- Feature：`019-g01-wdio-five-phases`
- 状态：`IMPLEMENTING / BLOCKED`，不是 Closed。
- Task：`53/70` 完成，剩余 `17`。
- Mandatory loop：R3 implementation admission 已 PASS；`after_implement` / R4
  未运行，也不满足运行条件。
- 当前唯一硬阻塞：staging `mattermost` 实际写入的 PostgreSQL 与
  `codex.cses-pre-postgres-ro` 指向的 CNPG 不是同一数据源。
- `single-real-chain` 与 `dual-real-chain` 必须保持 Red；UI、SQLite、Frame、
  Observer DOM 或截图通过不能替代远端 PG facet。

## 2. 两仓实时状态

### Helix

- 路径：`/System/Volumes/Data/workspace/rust/helix`
- 分支：`main`
- HEAD：`735cd6469b34d7f954a7d696054aa05404276a83`
- 已提交：`test(im): 固化 G01 Frame V2 真源夹具`
- fixture producer source commit：
  `32e3a0659e4f29add1284be2ba1972eb28f0f938`
- fixture provenance SHA-256：
  `929e59444ee79377034b8170071e5dfa0fb1d202ff39feefab07a7e1cb254242`
- 未提交但必须保留：
  - `SESSION.md`
  - `specs/019-g01-wdio-five-phases/review-ledger.json`
  - `specs/019-g01-wdio-five-phases/tasks.md`
  - `specs/019-g01-wdio-five-phases/verification.md`

### cses-client

- 路径：`/System/Volumes/Data/workspace/angular/cses-client`
- 分支：`feat/helix-reactor-im`
- HEAD：`2ba5a8de864836a1436568184d15809df72bf570`
- 状态：WDIO 实施工作树未提交；禁止 reset、checkout 或覆盖这些改动。
- Helix 四包 pin：
  `32e3a0659e4f29add1284be2ba1972eb28f0f938`
- 当前 candidate：
  `src-tauri/target/debug/cses-client`
- candidate SHA-256：
  `4e83ac8aa4a4eba28148e0c0e061cc0a5b95e871cc96d2dfd57b68c7a2d454cc`
- 当前 lock SHA-256：
  - `yarn.lock`：
    `ce326ebc2be765c4fcc635b5ff9a3b4b04f7640dded66a7b484cf5cddfcd77ad`
  - `src-tauri/Cargo.lock`：
    `15345448229ec9a2011c19d6d4e715e79e6bb53615f9c81940439b4638fb87a7`

## 3. 已完成到哪里

### L0 / 安全入口

- `wdio-e2e` additive Cargo feature 与 optional Rust plugins 已落地。
- 测试专用 Tauri capability/config 已落地。
- production Angular invoker 不含 Mock 入口；Mock-aware invoker 只通过
  `wdio-e2e` file replacement 进入测试 bundle。
- Rust boundary：4/4 PASS。
- Angular production-boundary contract：PASS。

### UI / Mock / Integration

- UI lane：PASS。
- Mock lane：3/3 PASS：
  - `sending -> sent`
  - `sending -> failed`
  - `ipc-rejected`
- Mock receipt 与 `im:__bus__` Frame replay 已分离。
- Integration lane：真实 Tauri invoke + 真实 Helix Frame + Actor DOM PASS。
- Report contract：PASS。

### 双账号与证据

- Actor：444；Observer：678。
- profile、auth、device、WebDriver port、日志和 artifact root 已隔离。
- 444 真实发送后，678 DOM 能看到相同 fresh marker。
- 双端截图在截图事务内重新定位 fresh marker，避免“DOM 存在但截图不可见”。
- 截图保存 viewport、时间和 SHA-256 index。
- Artifact redaction/schema scan：PASS。

## 4. 已验证命令

```bash
# Helix
cargo test -p helix-im --test g01_wdio_fixture_contract_test

# cses-client focused contracts
yarn tsc -p tests/e2e/wdio/tsconfig.json --noEmit
yarn jest -c tests/e2e/wdio/jest.config.js --runInBand
node scripts/message-wdio/build-candidate.mjs --check
cargo test --manifest-path src-tauri/Cargo.toml \
  --test wdio_plugin_boundary --features wdio-e2e
yarn ng build -c wdio-e2e --base-href /

# WDIO passing lanes
yarn message:wdio:g01:ui
yarn message:wdio:g01:mock
yarn message:wdio:g01:integration
yarn message:wdio:g01:report-contract
```

最近验证结果：

- Jest：17 suites / 57 tests PASS。
- Mock：3/3 PASS。
- Integration：真实 invoke PASS；coexistence PASS。
- Performance case 当前 skipped，不可声明 performance Green。

## 5. 当前失败签名

真实 single/dual lane 的终止错误：

```text
G01 marker is absent from remote PostgreSQL public.post
```

已确认事实：

1. fresh marker 在目标 CNPG `public.post` 的 before 查询为 `0`。
2. 444 UI 发送后，Actor/Observer DOM 和本地 SQLite 都出现终态消息。
3. 同一 marker 在目标 CNPG 的 after 查询仍为 `0`。
4. staging `mattermost` Deployment 为 `1/1 Ready`。
5. staging 应用实际 PG 连接与 keychain CNPG 不同源。
6. `postgres-exporter-pre` 用户可连接另一套 pre PG，但没有
   `public.post` 查询权限。
7. staging ConfigMap 中的应用凭据改连实际 PG 会返回认证失败，不能作为只读证据源。

因此这是数据库证据真源/路由问题，不是 WDIO selector、Frame codec 或 UI 渲染问题。

## 6. 最近证据

- Run：`dual-real-index-20260724211947`
- 目录：
  `/System/Volumes/Data/workspace/angular/cses-client/artifacts/message-wdio/dual-real-index-20260724211947`
- Actor screenshot：
  `screenshots/actor-terminal.png`
- Observer screenshot：
  `screenshots/observer-terminal.png`
- Screenshot index：
  `screenshots/index.json`
- viewport：`800x600`
- Actor SHA-256：
  `48836e8a83318e0a1bc9f3570693761b51811331fcaf1960ef8a7ea8d5da9d15`
- Observer SHA-256：
  `1c7c946a2d43df3f3e2a0bf049a473894dc4b85abf63704a2be14a2ad784432d`
- Artifact scan：
  `10 files / redactionVerdict=pass`
- Report exit：`1`，原因仅为远端 PG marker 缺失。

## 7. 当前端口状态

- `8066` staging IM port-forward：已停止。
- `55432` CNPG port-forward：已停止。
- 没有遗留的 port-forward session。

恢复 staging IM：

```bash
kubectl -n cses-staging port-forward svc/mattermost 8066:8065
```

CNPG 诊断连接可以临时恢复，但它当前不是 staging `mattermost` 的同源证据库：

```bash
kubectl -n postgres-cses \
  port-forward svc/postgresql-cses-pre-cnpg-rw 55432:5432
```

## 8. 下一步临界路径

### 第一步：解除 PG 真源阻塞

二选一：

1. 提供 staging `mattermost` 实际写库对应的只读 DSN，并确认可查询
   `public.post`。
2. 将 staging `mattermost` 的写入路由统一到当前 keychain CNPG。

禁止把错误 CNPG 上“marker 不存在”改成 warning；必须继续 fail-closed。

拿到同源只读 DSN 后：

```bash
cd /System/Volumes/Data/workspace/angular/cses-client

export CSES_WDIO_PG_READONLY_DSN='<same-source readonly DSN>'

yarn message:wdio:preflight --lane single-real
yarn message:wdio:g01:single-real
yarn message:wdio:g01:dual-real
```

验收要求：

- PG before=`0`，after=`1`。
- PG `id/channel_id/user_id/temporary_id` 与 SQLite、Frame、双 DOM 一致。
- Actor/Observer fresh screenshots 同 run、marker 可见。
- reducer 九面全部 PASS。

### 第二步：single-real Green 后迁移 G01 raw driver

只在 T038 PASS 后执行：

- T039：失败全量升级与首次失败附件。
- T043-T045：G01 代表链切换 WDIO adapter、G01 raw 调用归零。
- 其他 Gate 仍有引用时保留共享旧 driver。
- 不允许提前物理删除共享 raw driver。

### 第三步：完成 dual/reporter/performance

- T047/T048：evidence adapter 与 warm P95。
- T052/T053/T056：dual real、correlation、九面 reducer Green。
- T060：manifest 补齐真实 source pins、PID/process window 和 artifact index。
- T063/T064：performance reducer 与稳定 `message:wdio:all`。
- T066/T067/T069：全 lane、artifact scan、raw migration audit。
- T070：只有全部通过后才运行 mandatory `after_implement` 并申请 R4。

## 9. 剩余 Task

```text
T038 T039
T043 T044 T045
T047 T048
T052 T053 T056
T060 T063 T064
T066 T067 T069 T070
```

## 10. 禁止事项

- 不要运行 `after_implement` 或宣称 R4 Closed。
- 不要把 UI/SQLite/Frame PASS 写成 single/dual real-chain Green。
- 不要在 T038 前执行 T043/T044 的 raw cutover。
- 不要删除其他 Gate 仍使用的共享 desktop driver。
- 不要安装 standalone `tauri-driver`；当前 Harness 使用 embedded provider，
  standalone driver 会改变 provider 行为。控制台的缺失诊断是上游误报。
- 不要清理两仓 dirty worktree。
- 不要输出账号密码、auth 文件内容、DSN 或 Kubernetes Secret。

## 11. 恢复阅读顺序

1. 本文件。
2. `verification.md`。
3. `tasks.md` 中剩余 17 项及依赖。
4. cses-client
   `tests/e2e/wdio/harness/storage-evidence.ts`。
5. cses-client
   `tests/e2e/wdio/harness/real-evidence-collector.ts`。
6. cses-client
   `tests/e2e/wdio/specs/g01-single-real.e2e.ts`。
7. cses-client
   `tests/e2e/wdio/specs/g01-dual-real.e2e.ts`。
