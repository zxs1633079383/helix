# CATEGORY_POST_CHAIN_WIRE/v3 — Helix交付

状态：**V3_BUILD_READY_WAITING_GO_FIVE_TABLES**。v3增量query/feature、精确持久runtime补丁和新runner已完成本地验证，未live。权威：[Go v3合同](/Users/cses-11/Desktop/cses-im-server/docs/category-post-chain-wire-v3.md)。v2九条HTTP路径、三个get view、11个分类语义命令、DTO/Event/字符串版本/commandKind保持不变。

所有分类URL统一追加`?mode=CATEGORY`，body仍有`mode:CATEGORY`。Native从语义命令生成query，调用方不提供或覆盖。capabilities亦使用query，模块同时报告`postChainCategoryMode:true`与`postChainCategoryWireVersion:3`；主任务客户端还需检查canonical `category_chain_capabilities`注册。旧v2模块不能凭单个boolean被误认为v3。

仅修改Helix分类wire URL、feature和已有4项公开seam回归（测试名升级v3）。TEXT九动作的URL/body与数字版本保持；不修改TEXT解析器、请求体限额、mark-read、runtime authority/持久屏障、普通消息或观测API。本轮不改Go表/DDL。

- [v2→v3持久0.1.9增量patch](category-post-chain-v3-runtime-0.1.9.patch)：client根按`--directory=.cargo/category-chain-helix`应用；发送主任务前已对其v2持久副本只读apply-check通过。
- [正式0.1.1本轮窄patch](category-post-chain-v3-runtime-0.1.1.patch)：仅本轮增量，不是整功能从HEAD补丁。
- [v2→v3 runner patch](category-post-chain-v3-native-runner.patch)：相对harness目录，保留既有v2证据字段和负向检查。
- [源码前后SHA/日志](category-post-chain-v3-readiness.json)。主任务拥有实际client合入/重编/运行，本任务未直接写其副本或配置。

新binary：`/var/folders/xp/tcg1zrbd3tz557b6qvw977sr0000gn/T/helix-post-chain-v3-qnrq328p/candidate/target/debug/category-native-smoke`

SHA256：`a0bdfe17e335e7efca3cc1420bb06128eabfd719a2bc16762b751357d444d4bd`

新候选：`/var/folders/xp/tcg1zrbd3tz557b6qvw977sr0000gn/T/helix-post-chain-v3-qnrq328p/candidate`。没有runs目录，未发起HTTP/WS/Go DB连接。旧v2 binary保持SHA `e45f51bfeac8a09eb37f49642aeb5078d5c15a55b0bf621d4ad5aaaaab5dfefa`；旧run与v2证据没有覆盖。

runner新增HTTP dispatch/匹配reply证据`queryMode/queryModeCount/querySyntaxValid/bodyModeSingle`，保留path/mode/view/action/corr及九路径/三view/11语义完整性。原始query不记录。query解码后重复（含编码key）、缺失、空、TEXT/未知、坏百分号/分号均不能成为成功语义；body.mode重复键（含escaped mode）、缺/null/错类型/异值也失败。允许合法编码和其他query参数。旧/category-chain URL带合法query/body仍会拒绝。`trace.successfulQueryModes`只应含CATEGORY，`categoryTransportValid`继续是最终PASS前置。输出`wireContract=CATEGORY_POST_CHAIN_WIRE/v3`，`evidenceSchema=v2-path-mode-view-plus-v3-query`。

验证：红测2 fail/2 pass；正式29分类、25 chain lib及check；独立0.1.9 29分类、最终7 runner测试及build；热路径审计和diffcheck均通过。Go读前262144-byte限制、最多262145-byte探针、错误零App/Store与旧TEXT大body/首JSON兼容仍须由Go独立测试证明，本地Native测试没有冒充这些证据。

Go原五表实现与服务预检完成后，由主任务协调新v3真实链。现有runner隔离身份/DB声明边界未改，固定18066和实际数据库绑定由主任务/Go提供与预检，本任务不猜测。旧七表/独立URL smoke及v2编译都不能代替v3五表验收；UI DOM、Web/FFI、跨Pod/Pulsar、故障恢复、日常部署仍未验证。无提交、提版、发布或部署。
