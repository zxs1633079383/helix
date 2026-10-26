# Helix GS01-04 持久化确认屏障报告

- 基线：`main@77d5f534baacc0d49d9c0e56f1149ae5aeb4ed0d`
- 落地提交：`9879572cb659de666c7fe075de167d9e06b3eefd`
- 范围：Helix `helix-im` sync / increment / domain tests / Strict Matrix
- 未触碰：mobile、Go server、用户 `.gitignore`、既有未提交报告

## 结论

GS01-04 的最终业务投影现在统一由 Helix 的相关持久化回执解锁。HTTP SUCCESS、
`increment_channel_end`、Persist 未回执或 Persist Err 都不能单独晋升最终投影。

| Gate | 提交屏障 | 成功后投影 | 失败/未回执 |
| --- | --- | --- | --- |
| G-S1 | `channel -> channel_member -> channel_event_cursor` 同批 Persist | `im:channel:increment`、`im:channels:loaded` | 不发布 |
| G-S2 | message/channel 派生写与 cursor 同批 Persist | sync mutation、`im:channel:update-by-post` | 不发布 |
| G-S3 | 连续 message/cursor 同批 Persist | `im:sync:state{state=committed,recovery}` | 不发布 |
| G-S4 | 单频道 channel/member Persist | `im:channel:increment`、即时 dialog projection | 不发布 |

sync scheduler 与 todo 查询仍保留各自 HTTP 回执状态机；它们不是 G-S1 最终投影完成证据，
也没有把完成判断下放给 driver 或 UI。

## TDD 证据

新增 `crates/helix-im/tests/gs_persist_barrier_test.rs`，先形成 6/6 RED，再实现到 6/6 GREEN：

1. G-S1 HTTP SUCCESS / global end 不提前发布。
2. G-S1 Persist Err 保持静默。
3. G-S1 Persist Ok 按批释放 increment / loaded。
4. G-S2/G-S3 message 先于 cursor，Err 不推进 cursor、不发投影。
5. G-S2/G-S3 Ok 后发布 mutation / dialog / sync-state。
6. G-S4 hydration 只由 Persist Ok 解锁。

验证结果：

- barrier + increment + UC4 + S4 + Strict Matrix：28 tests PASS。
- WS registry：13 tests PASS。
- increment heal / golden replay / scheduler / WS / S1 / S6 / todo 定向回归：67 tests PASS。
- `cargo check -p helix-im` PASS。
- `node scripts/strict-matrix/check.mjs --check-generated` PASS。

## 合同状态

Strict Matrix 已升到 epoch 5，G-S1 至 G-S4 的 response、Persist 语义、反向断言和测试引用
已经写入权威 JSON，并重生 Markdown/hash。代码屏障已实现；合同仍保持 `declared`，因为真实
Tauri/FFI、DOM 与八面关联证据不在本次 Helix 代码刀范围内，未用单元测试伪装整行 Green。
