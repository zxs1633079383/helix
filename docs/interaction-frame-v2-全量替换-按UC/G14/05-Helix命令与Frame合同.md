# G14 Helix 命令与 Frame 合同

## 命令与 Effect

| Gate | command | Effect |
|---|---|---|
| `G-14` | `im_post_read` | `http_write`、`storage_upsert`、`projection` |
| `G-14a` | `im_template_received` | `http_write`、`projection` |

## Frame

- producer：`ImModule::handle -> projector/SurfaceComposer -> Effect::Emit`
- schema：`InteractionFrameV2`，`schemaVersion=2`
- canonical Surface：`message.participant-receipts`、`message.timeline`
- 写路径只有在 matching `PersistOk` 或权威 readback 后才能发布终态 Frame。
- 一个 `Effect::Emit` 携带一个完整 Frame，driver 不逐 mutation 拆分。

## 源码与测试

- `crates/helix-im/src/interaction_frame.rs`
- `crates/helix-im/src/projector/mod.rs`
- `crates/helix-im/tests/g14_receipt_template_interaction_test.rs`
