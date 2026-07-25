# G01 Helix 命令与 Frame 合同

## 命令与 Effect

| Gate | command | Effect |
|---|---|---|
| `G-01` | `im_send_message` | `storage_upsert`、`http_write`、`timer`、`projection` |

## Frame

- producer：`ImModule::handle -> projector/SurfaceComposer -> Effect::Emit`
- schema：`InteractionFrameV2`，`schemaVersion=2`
- HTTP canonical Surface：`message.timeline`
- WS canonical Surface：独立 `message.timeline` 结算，以及已 attach 时条件性
  `message.left-dialog-list` patch；两者使用不同 correlation，不保证顺序。
- 写路径只有在 matching `PersistOk` 或权威 readback 后才能发布终态 Frame。
- 一个 `Effect::Emit` 携带一个完整 Frame，driver 不逐 mutation 拆分。

## Canonical Fixture

唯一 producer 是共享 `G01ScenarioRunner`：

```text
Tick/PortReply/Inbound
-> ImModule::handle
-> Effect::Emit
-> InteractionFrameV2::decode
-> typed validate
-> 字段级语义断言
-> canonical JSON
```

库存固定为：

- `sending.frame-v2.json`
- `sent.frame-v2.json`
- `failed.frame-v2.json`
- `ws-timeline-sent.frame-v2.json`
- `ws-left-dialog-list.frame-v2.json`

普通测试只比较 actual 与 Golden。只有显式设置
`UPDATE_G01_WDIO_FIXTURES=1` 才能覆盖 canonical 文件。

## 源码与测试

- `crates/helix-im/src/interaction_frame.rs`
- `crates/helix-im/src/projector/mod.rs`
- `crates/helix-im/tests/p1_send_build_test.rs`
- `crates/helix-im/tests/g01_full_flow_test.rs`
- `crates/helix-im/tests/g01_wdio_fixture_contract_test.rs`
