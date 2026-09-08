# Helix IM Strict Matrix

> `RETIRED_HISTORICAL`：本目录及其生成矩阵只保留 InteractionFrame 时代的 UC 发布快照，
> 不参与当前 Gate 分母或 Green 判定。当前 MessageV3 producer 真源见
> [`specs/047-im-event-gate-inventory/`](../../../../specs/047-im-event-gate-inventory/)。

本目录是 Helix IM 的历史 UC 发布矩阵入口。它回答的不是“当前代码有没有写”，而是“当时的
一个 UC 是否已经具备可发布的全链路证据”。

## 文件职责

- [`UC全链路发布矩阵-v1.md`](./UC全链路发布矩阵-v1.md)：由机器合同生成的完整 `UC × UI × Helix × HTTP × WS × Storage × Native` 矩阵，禁止手工修改。
- [`Command接口-v1.md`](./Command接口-v1.md)：由同一 Matrix 生成的 canonical command 使用表；每个 Gate 的字段集独立保留，不能从 UI 壳层反推或合并。
- [`持久证据包白话说明.md`](./持久证据包白话说明.md)：解释“为什么测试跑过还不够”，以及证据包具体保存什么。
- [`运行环境与远端存储真源.md`](./运行环境与远端存储真源.md)：记录 Consul KV、imv2、Redis、Pulsar、ES 的职责边界和脱敏后的当前核验结果。
- [`../../contracts/strict-matrix.v1.json`](../../contracts/strict-matrix.v1.json)：机器可读的唯一合同真源。
- [`../../contracts/strict-matrix.schema.json`](../../contracts/strict-matrix.schema.json)：合同结构约束。
- [`../../contracts/command-api.v1.json`](../../contracts/command-api.v1.json)：从 Matrix 生成的下游消费 API；`matrix_sha256` 是正式 commit blob 与 PC/Mobile pin 的绑定值。
- [`../../contracts/NEED_COMMAND_API_RUNTIME_ALIGNMENT.md`](../../contracts/NEED_COMMAND_API_RUNTIME_ALIGNMENT.md)：Matrix 与当前 runtime 的已知字段/投影漂移；命中即 `CONTRACT_DRIFT`，不得伪造下游兼容成功。

## 判定公式

```text
UC Green
= contract_state == implemented
+ UI Green
+ Helix Green
+ HTTP Green 或有反向证据的 N/A
+ WS Green 或有反向证据的 N/A
+ Local Storage Green
+ Remote Storage Green
+ Native Green
+ 同一 run_id/gate_id/operation_id/req_id/channel_id
```

其中任一项缺失、过期、来自其他 run、只有 mock、只存在于 `/tmp`，或者无法通过 manifest 校验，整行都必须是 Red。`declared` 只表示合同已经写清，不能被展示成“功能完成”。

## 更新流程

1. 只修改 `contracts/strict-matrix.v1.json`，不要手改生成矩阵；每个 UC 必须写明 HTTP request/response、WS 事件和远端库表。
2. 运行 `node scripts/strict-matrix/check.mjs --write` 生成文档。
3. 运行 `node scripts/strict-matrix/check.mjs --check-generated` 检查合同与文档无漂移。
4. 运行 `cargo test -p helix-im --test strict_matrix_contract_test` 检查 Helix 命令面、所有权和证据合同。

下游只能从正式 Helix pin 读取合同，禁止从工作树猜测版本。示例：

```bash
node scripts/strict-matrix/check.mjs \
  --revision <helix-commit-sha> \
  --write-consumer-ts <downstream-generated.ts>
```

`matrix_sha256` 是 `strict-matrix.v1.json` 原始 blob 的 SHA-256，用于 release pin、manifest 与生成物校验。`matrix_semantic_sha256` 只用于稳定内容诊断，不能替代前者。生成的 command API 只描述合同，不表示真实 HTTP/WS/projection/E2E 已 Green。

新增 UC 时必须同时声明 Gate、命令、Effect、持久化、RenderNode、allowed actions、反向断言和七平面证据要求。缺任一项时，生成或测试必须失败，而不是生成一行“待补充”后继续发布。
