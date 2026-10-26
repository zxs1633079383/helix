# 关闭群 seq 历史读取回归

## EXPECT

- 根因：seq 查询把 channel 的同步终态当作历史访问禁令；PRE 新旧历史接口均允许仍在群内的用户读取关闭群。
- 入口仍为 `im_query_messages_by_channel`、`im_load_older_context`、`im_load_newer_context`、`im_locate_context`；不改协议、数据库或 event 同步游标。
- 服务端每次查询的 metadata 校验继续决定访问权；客户端仅用频道生命周期代次拒绝跨关闭、离群、重入的迟到响应。
- 回归包括关闭群冷/热历史和定位、关闭后离群期间的迟到响应、服务端拒绝不回退、同步终态不被历史读取解除。
- 验证分开记录 sans-IO、真实 SQLite 和 A/B 主窗口/独立窗口；不将模拟 authority 当成 PRE 实机证据。

## 状态

- RED：sans-IO 与真实 SQLite 均复现 `SEQUENCE_PAGINATION_ACCESS_REVOKED`。
- GREEN：seq 10 项、lifecycle 12 项、startup scan 8 项、WS 12 项、真实 SQLite 26 项、库内 seq 5 项通过，共 73 项；`cargo check -p helix-im` 和 diff 检查通过。
- 两个旧 A/B 包均以原生点击复现关闭群空白和加载失败，后台新旧历史接口仍返回成功。
- 日志：`artifacts/closed-channel-sequence-20261026/`；客户端同名目录保留 baseline、构建、WDIO 脚本和后续结果。
- 当前正构建本地候选，桌面 GREEN 待验。未发布新版本、未 push；Go 不需为此缺陷重新发布。
