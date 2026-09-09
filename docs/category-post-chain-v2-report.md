# CATEGORY_POST_CHAIN_WIRE/v2 — Helix交付

传输选择规则已由[v3增量交付](category-post-chain-v3-report.md)覆盖；本文保留v2源码/编译证据，不代表v3真实链验收。

状态：**V2_BUILD_READY_WAITING_GO_FIVE_TABLES**。v2 wire/feature与新Native runner本地验证完成；用户随后明确复用原TEXT五表，真实链必须等Go新存储实现就绪。旧七表、独立category-chain URL的Native PASS只保留为历史证据。

权威合同：[Go v2合同](/Users/cses-11/Desktop/cses-im-server/docs/category-post-chain-wire-v2.md)。本轮不合并Native命令、Go/Helix表、消息type或WS事件；仅统一HTTP分发。分类内部DTO、commandKind、字符串版本、同人多类全量CAS/空数组取消、分页、权限和持久屏障保持。

| 分类Native语义 | HTTP（公共前缀/api/cses） | 派生字段 |
|---|---|---|
| capabilities | /post-chain/capabilities | mode=CATEGORY，body仅此字段 |
| create_draft | /post-chain/create | mode=CATEGORY |
| update_draft / publish | /post-chain/update-draft / publish | mode=CATEGORY |
| set_participation | /post-chain/append | mode=CATEGORY；原完整selections和CAS |
| get | /post-chain/get | mode=CATEGORY，view=card |
| get_mine | /post-chain/get | mode=CATEGORY，view=mine |
| entries | /post-chain/get | mode=CATEGORY，view=entries；原categoryId/cursor/limit |
| close / retract / reconcile | /post-chain/close / retract / reconcile | mode=CATEGORY |

Native从注册语义统一注入mode/view，不接受调用者覆盖。req_id留在本地。分类reconcile仍是CommonRes查询，retract仍整anchor且拒绝entryId；TEXT原九动作和mark-read不改。Go负责mode非法/null/缺失与view的HTTP边界错误；CATEGORY不发mark-read。客户端需同时检查canonical `category_chain_capabilities`注册、`module.features.postChainCategoryMode===true`和真实Go supported，旧command存在不能证明新transport。

正式0.1.1仅改wire.rs、capabilities.rs、4项新增公开seam测试。与客户端持久0.1.9匹配的补丁按该版本原文件生成，保留0.1.9普通消息/观测行为。源码租约不包含client持久副本，本任务未直接修改它；主任务已获得可合入patch。

- [0.1.9持久runtime补丁](category-post-chain-v2-runtime-0.1.9.patch)：在client根按`--directory=.cargo/category-chain-helix`应用；交接前只读`git apply --check`通过。
- [0.1.1本轮窄补丁](category-post-chain-v2-runtime-0.1.1.patch)：相对于既有分类dirty工作树，不是整功能从HEAD的补丁。
- [runner补丁](category-post-chain-v2-native-runner.patch)：相对于旧runner的harness目录，仅evidence/main；已应用在下面新临时候选中。
- [前后SHA与日志](category-post-chain-v2-readiness.json)。未提交、提版、发布或部署。

新独立候选：`/var/folders/xp/tcg1zrbd3tz557b6qvw977sr0000gn/T/helix-post-chain-mode-9uimdqmh/candidate`。

新binary：`/var/folders/xp/tcg1zrbd3tz557b6qvw977sr0000gn/T/helix-post-chain-mode-9uimdqmh/candidate/target/debug/category-native-smoke`。

SHA256：`e45f51bfeac8a09eb37f49642aeb5078d5c15a55b0bf621d4ad5aaaaab5dfefa`。

旧binary和既有run均未修改；新candidate没有runs目录，未调用live。

新runner在HTTP dispatch与匹配reply记录实际白名单path、mode、view、语义action和corr；仍要求11个语义动作完成，9个唯一HTTP路径和get三个view可逐项核对。`trace.categoryTransportValid`为PASS必需项；旧/category-chain URL、错误mode、缺失/错误view即使HTTP200也不能算语义成功，全部成功后再出现旧URL同样使最终INCOMPLETE。原WF、WS/EventSink/SQLite、幂等、错误脱敏和graceful cleanup约束保留。运行evidence新增`wireContract=CATEGORY_POST_CHAIN_WIRE/v2`。

验证：公开seam红测2失败/2通过（旧URL、缺feature）；正式树分类29/29、chain lib25/25、helix-im check；独立0.1.9 Native runner build、分类29/29、runner6/6；热路径审计与diffcheck通过。测试包含11动作URL/mode、三view精确body、嵌套camelCase、大字符串版本/整集合取消、非法transport覆盖拒绝、TEXT九动作缺mode的原请求体逐值比较。生产runtime的成功/失败/迟到/cancel/persist/旧revision测试一并通过。

下一步由主任务合入持久runtime patch并重编；Go五表实现就绪后协调一次新独占fixture，使用本binary准确路径/hash。Go tagged fixture需更新其固定binary路径/hash和v2路由证据，不能继续引用旧candidate或旧七表PASS。此处不启动服务/DDL/数据库连接；UI DOM、Web/FFI、跨Pod/Pulsar、故障恢复及日常部署均未验收。
