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
- A/B 真实 Tauri + PRE 四场景通过：A 主/独立窗口各核对全部 335 条可见消息，B 主/独立窗口各核对 187 条，未混入其他 148 条。完整上翻到首条、回到最新、切群重入和未读清零均通过。
- 核对真实 postId 集合与 DOM 可见性；独立窗口无 latestPostsStatus，系统通知无 message-body，不能套用主窗口/普通正文的采集假设。自动化前置错误与修正记录保留在客户端证据目录。
- 本地候选来源为 `2b9c943a`，二进制 SHA 见 provenance.json；主窗口通过结果在相同代码、配置、候选、环境下复用，后续完成其余三场景。
- 开发 A/B 暂通过客户端本地 Cargo patch 使用修复源码，避免下一次开发编译回到旧内核。该机器配置与锁文件调整未提交；registry 仍为 0.1.87。未发布新版本、未 push；Go 不需为此缺陷重新发布。
