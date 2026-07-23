# Quickstart: InteractionFrame V2 Cutover Validation

## 1. Preconditions

1. 记录 Helix、cses-client、Go HEAD/branch/status，不覆盖现有脏改动。
2. 冻结 Helix revision，并确认 cses-client 四包 pin 与 `Cargo.lock` 一致。
3. 生成并记录 Strict Matrix/fixture semantic hash。
4. 准备两个隔离测试账号、目标频道和只读 PG/SQLite 查询能力。
5. 禁止手工写数据库；所有业务数据由正常产品链生成。

## 2. Focused Shared Contract

```bash
cd /System/Volumes/Data/workspace/rust/helix
cargo test -p helix-im --test interaction_frame_contract_test --no-default-features
cargo test -p helix-im

cd /System/Volumes/Data/workspace/angular/cses-client
npm run message:command-api:gate
npm run message:projection-ownership
npm run message:surface-runtime:source-pin
npm run message:ui:authority
npm run message:ui:selector-contract
npm run message:real-chain:doctor
```

预期：V1 输入 fail-closed；V2 epoch/revision/atomic apply/FrameApplied 全部通过；source pin/hash 一致。

## 3. Per-Gate Focused Validation

按 [per-gate-spec-index.md](per-gate-spec-index.md) 顺序执行。每 Gate 必须先写/运行失败测试，再实现，再运行：

1. Gate Rust replay/contract test；
2. Gate Angular codec/runtime/component test；
3. Gate fixture parity；
4. Gate PG/SQLite readonly query shape dry-run；
5. Gate evidence facet completeness check。

单 Gate focused PASS 只标记 `READY_FOR_FINAL_E2E`，不是产品完成。

## 4. SCSS/CSS Gate

实施任务必须在 `package.json` 增加正式脚本。预期组合：

```text
Sass/CSS build
target component style-loaded assertion
444/678 DOM bounding-box and overflow assertion
actor/observer screenshot capture
```

脚本不存在或任一组件没有实际样式证据时，最终 Gate 为 BLOCKED。

## 5. First V2-only Native Run

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
npm run message:desktop:frame-prerequisites
npm run message:desktop:build
npm run message:desktop:scenario:all -- --profile interaction-frame-v2-cutover
npm run message:real-chain:collect
npm run message:real-chain:generate-matrix
npm run message:real-chain:reduce -- --profile interaction-frame-v2-cutover
```

预期：

- 25/25 Gate；
- required fail=0、skip=0；
- v1Frames=0、rawBusinessFallbacks=0；
- committed V2 transaction 与 FrameApplied success 数一致；
- 每 Gate 双用户、Go、PG、SQLite、Frame、DOM 和 444/678 截图 facet 完整。

当前工具尚未实现所有参数/counters；在对应任务完成前不要直接执行并宣称 PASS。

## 6. V1 Deletion Admission

只有第 5 节全绿才执行：

1. 审查 deletion inventory；
2. 删除 InteractionFrame/RenderNode V1 writer、reader、normalize、fixture、兼容测试；
3. 保留独立 API/文档 schema 的合法 `v1`；
4. 运行静态扫描和 cross-repo fixture parity。

## 7. Post-deletion Run

生成新的 candidateId、runId、evidenceEpoch 和 artifact root，重新执行第 2、4、5 节。任何删除前 artifact 被 reducer 引用都必须失败。

## 8. Result Reporting

每条命令记录：

```text
command
startedAt
endedAt
wallTimeMs
result
retry/cacheWait/timeout/block durations
artifact paths and hashes
```

缺证据面报告 `BLOCKED/INCONCLUSIVE`，不从旧 017 或首次运行继承 Green。
