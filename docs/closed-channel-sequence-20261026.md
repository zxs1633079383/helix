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
- 实机验收阶段，开发 A/B 通过客户端临时 Cargo patch 使用修复源码，当时 registry 为 0.1.87；正式接入结果见下节。Go 不需为此缺陷重新发布。

## 0.1.88 发布

- 用户已授权 push、发布新版并接入前端，本轮明确不跑前端测试。
- 远端新增的 `1e9b006b` 仅修改发布记录；经确认在当前分支变基，修复提交变为 `48e5cbd8`，生产源码与原实机候选一致。
- EXPECT：按现有发布脚本完成四包 0.1.88、版本提交及注释 Tag，推送后核对远端 SHA；前端移除临时本地 patch，精确依赖 registry 四包。
- RELEASE PASS：`cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 0.1.88 已发布至 crates.io 并可解析。版本提交 `835f7952` 与注释 Tag `helix-v-0.1.88` 已推送，远端 Tag 解引用与发布提交一致。
- 前端当前 `dev-message` 分支的四依赖及锁文件已切到 registry 精确 0.1.88，临时 Cargo patch 已移除；四包 checksum 一致、373 份发布源码与修复工作区逐文件一致，其他锁文件条目未改变。前端已有改动保留，未替用户提交或 push。
- 发布检查：八 crate 编译、四包 dry-run/上传/解析、core 测试、WASM 编译、真实 SQLite 分页 26 项通过。IM 广域 1750 通过、133 失败、11 ignored；失败集合与上一版完全一致，无新增，不纳入本次修复。
- 本轮按用户要求未运行前端测试或 WDIO。此前四场景实机结果对应修复候选；发布包源码比对及编译不能替代新一轮交互验收。
- 发布日志、失败集合对比、来源及 registry 核对位于 `artifacts/closed-channel-sequence-20261026/release-0.1.88/`；客户端同名目录保留依赖接入与开发包构建日志。
