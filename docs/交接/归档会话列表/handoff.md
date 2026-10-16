# 归档会话列表交接

更新时间：2026-09-24 Asia/Shanghai

## 目标与边界

- Go 关群只写频道 `delete_at`；`close=false` 可与归档并存。Helix 查询/同步保留归档频道，客户端“已归档”按 `deleteAt>0` 展示，viewer `is_remove` 独立控制移除态。
- Helix 已合并、推送并发布 crate；客户端已提交归档修复、锁定 `0.1.39` 并推送。未操作业务数据或部署桌面客户端。保留各仓无关脏文件。

## 当前代码身份

- Helix：`reactor/MessageV3-helix`，HEAD/远端 `ec587a27e06aefd8411b039cf53341ab800c2c65`，ahead/behind `0/0`；注释 Tag `helix-v-0.1.39` 已推送且指向同一提交。修复提交 `f2636259`、`bc0325b3`、`7b3133ac`，合并远端 0.1.37/0.1.38 后补测试 `d7c5730a`，版本提交 `ec587a27`。
- Go：`pre-paas2.5`，HEAD `23925b74a80e75441091abd86f18b9819ef85be2`；本任务无 Go 代码 diff。
- 客户端：`dev-message`，HEAD/远端 `9cd48cbc80c8d03924913fe48d9c5ead08e96c82`，ahead/behind `0/0`。归档修复提交 `918a5d2f53`、四个 Helix crate 锁至 `=0.1.39` 的提交 `3ce4d7b0ec`；合并同期远端三提交后推送。仅这五个文件由本任务改动并提交。

## 已完成与验证

| 范围 | 身份 | 结果与证据 |
|---|---|---|
| Go 关闭语义 | Go HEAD | `ChannelCloseInTx` 仅写 `channel.delete_at`；现有定向事务测试通过。 |
| Helix 合并 | `d7c5730a` | 五 crate check、core 测试、Web wasm、归档查询 7 项和写入 13 项通过；`helix-im` 库测试 452/452 通过。日志 `/tmp/helix-archive-merged-check.log`、`/tmp/helix-archive-release-*.log`。 |
| Helix 发布 | `ec587a27` | `0.1.39` 的 core、im、driver-host、driver-native 四包 dry-run、上传与 registry 回读通过；版本提交及注释 Tag 原子推送。日志 `/tmp/helix-archive-release-039-publish.log`。 |
| 客户端列表 | `918a5d2f53` | 归档同步/分组以 `deleteAt>0` 判定；服务测试 25/25、Host 会话/同步合同 18/18、包边界检查通过。合并远端后相关 Jest 59/59、`cses-message:build`、App `tsc --noEmit` 通过。 |
| 客户端 Helix pin | `3ce4d7b0ec` | `Cargo.toml` 与 lockfile 四包精确 `0.1.39`；`cargo check --locked --manifest-path src-tauri/Cargo.toml -p cses-client` 通过。远端分支 SHA 与本地相同。 |

## 阻断与未验证

- 完整 `cargo test -p helix-im` 在库测试 452/452 通过后，因客户端 tracked Phase 1 参考生成物 `MV3-G03a/inbound-contract.json` 多出 schema 未接受的 `go` 字段而失败；日志 `/tmp/helix-archive-merged-im-test-final.log`。不能称整包全绿。
- 客户端 `cses-client:build:dev` 本轮仍被未改动的依赖项目 `cses-orientation:build:production` 的 `file.referencedFiles[index]` 解构错误挡住；日志 `/tmp/cses-archive-app-build-current.log`。消息包构建独立通过。
- 未有同轮 Go WS → Helix SQLite/EventSink → Tauri 页面证据；crate 发布不等于客户端已更新。`im_query_dialog_list` 单次扫描上限 2000，超限时仍需正式频道同步页。

## 下一步

- 如需运行态验收，在授权环境做归档快照、Helix 查询和“已归档”页面同轮对账；另行处理 `cses-orientation` 构建故障。无需重跑已绑定相同代码与环境的通过检查。
