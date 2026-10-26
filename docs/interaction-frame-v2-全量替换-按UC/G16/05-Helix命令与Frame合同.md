# G16 Helix 命令与 Frame 合同

## 命令与 Effect

| Gate | command | Effect |
|---|---|---|
| `G-16` | `im_channel_close` | `http_write`、`storage_upsert`、`projection` |
| `G-16` | `im_channel_leave` | `http_write`、`storage_upsert`、`projection` |
| `G-16b` | `im_channel_leave` | `http_write`、`transport_broadcast`、`projection` |
| `G-16c` | `im_channel_change_display_name` | `http_write`、`projection` |
| `G-16c` | `im_channel_change_notice` | `http_write`、`projection` |
| `G-16c` | `im_channel_change_info` | `http_write`、`projection` |

## Frame

- producer：`ImModule::handle -> projector/SurfaceComposer -> Effect::Emit`
- schema：`InteractionFrameV2`，`schemaVersion=2`
- canonical Surface：`message.channel-summary`、`message.member-roster`
- 写路径只有在 matching `PersistOk` 或权威 readback 后才能发布终态 Frame。
- 一个 `Effect::Emit` 携带一个完整 Frame，driver 不逐 mutation 拆分。

## 源码与测试

- `crates/helix-im/src/interaction_frame.rs`
- `crates/helix-im/src/projector/mod.rs`
- `crates/helix-im/tests/g16_governance_interaction_test.rs`
