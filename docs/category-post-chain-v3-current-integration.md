# 分类接龙 v3 当前联调交付（2026-09-08）

**五表 Native 真链 PASS；当前 monorepo 客户端命令桥接已补齐，UI/DOM 待验收。** 本轮依据当前源码重新执行，不沿用旧七表或“等待五表”的历史结论。机器证据见 [current-integration.json](category-post-chain-v3-current-integration.json)。

## 当前实现与调用

客户端 `/Users/cses-11/Desktop/cses-client`，分支 `monorepo-message`，HEAD `8d465d287fde43f8a1bfb7b3d5ffc735404d288c`。已有23个 Git dirty UI 文件逐字 hash 不变；另有忽略的历史说明，未改写。新增桥接为未提交改动。使用现有 `MESSAGE_V3_IM_GATE` 的11个 `categoryChain*` 方法，完整调用说明见 [客户端接入文档](/Users/cses-11/Desktop/cses-client/packages/message/docs/category-chain-command-bridge.md)。

- 已补 Tauri 五读六写入口、boot 注册及 correlation 白名单。五个读命令复用既有 `await_query_result`，30秒有界等待；reconcile 是查询，不当 mutation admission。
- Angular Port、Helix、Mock、Pending 方法齐全。Mock/Pending 明确不支持；真实 Host 核验加载模块的 mode feature、wireVersion=3、canonical command，再实际查远端 capabilities。每次入队/返回前复验最初捕获的账号代次和 renderer 生命周期。
- CATEGORY payload 仅转换顶层键；categories/audience/selections 内 camelCase 原样，revision 精确字符串原样。TEXT adapter 行为不变。
- `im:category_chain:operation` / `projection` 经既有单总线映射为 `categoryChainOperation` / `categoryChainProjection`，完整 canonical data 原样交付，无新 reducer/listener。读结果仅严格剥 CommonRes；业务 DTO 字段仍需由消费者校验。
- UI 保存/创建保持禁用，未恢复旧 Facade/Host UI 源码，未向核心消息分发加 CATEGORY 分支。extension 继续承担分类消息展示。

## 本轮真实请求与结果

所有列出路径均为 `POST /api/cses/post-chain/<route>?mode=CATEGORY`，body.mode=`CATEGORY`。下表路由/mode/view/HTTP状态来自同轮 Native dispatch/reply 探针；行为结果由 runner 断言及 Go 独立 SQL 读回共同验证。没有在公开报告保存正文、身份、cookie或原始 query。

| Helix semantic / 客户端方法 | route / view | 实际结果 |
| --- | --- | --- |
| capabilities / categoryChainCapabilities | capabilities | HTTP 200、CommonRes SUCCESS，schemaVersion1、supported=true |
| create_draft / categoryChainCreateDraft | create | SUCCESS，draft revision `1`，operation CONFIRMED |
| update_draft / categoryChainUpdateDraft | update-draft | SUCCESS，revision `2`，CONFIRMED |
| get / categoryChainGet | get / card | 草稿与发布后读取 SUCCESS |
| publish / categoryChainPublish | publish | SUCCESS，revision `3`；真实 WS、Post、SQLite、Emit |
| get_mine / categoryChainGetMine | get / mine | SUCCESS；本人选择经历0→2→0 |
| set_participation / categoryChainSetParticipation | append | 两类参与与空集取消均 SUCCESS；revision `4` / `5` |
| entries / categoryChainEntries | get / entries | SUCCESS；两类各一项，取消后空页 |
| reconcile / categoryChainReconcile | reconcile | SUCCESS，确认同一参与 mutation、revision `4` |
| close / categoryChainClose | close | SUCCESS，revision `6` |
| retract / categoryChainRetract | retract | SUCCESS，revision `7`，最终 RETRACTED |

Native run：`category-native-1788838907898449000`，550 hops，11语义无缺项，9路径与3视图齐全，query mode数量均1、语法有效，body mode唯一，transportValid=true、overflow=false。

Go读回：一条 chain、一条稳定 Post、7个 mutation、2条已取消 entry、0条活跃 entry/participant、5个 channel_event、5/5 delivered outbox。revision `3..7` 的 event identity 与 WS ingress、EventSink、SQLite anchor逐项匹配；末态card与chain一致。使用原有五表 `chain/chain_entry/chain_participant/chain_mutation/chain_viewer_state` 的 CATEGORY模式，未新建旧七表方案。

## 环境及清理

真链运行已有 tagged Go API fixture，装配真实 Router/App/SqlStore/WS/dispatcher，端口由 httptest 临时分配；只连接专用 `127.0.0.1:55441/cses_im_test`。schema为本轮自有 `category_chain_http_1788838907907110000`，退出时清理并读回 remaining=0。测试7.73秒、exit0，Native --run仅执行一次。第一次沙箱执行在DB连接前被禁止，无schema与业务请求；获得联网执行审批后才开始本轮run。

实际 Native候选为四crate `0.1.9`，二进制：
`/var/folders/xp/tcg1zrbd3tz557b6qvw977sr0000gn/T/helix-post-chain-v3-qnrq328p/candidate/target/debug/category-native-smoke`

SHA256：`a0bdfe17e335e7efca3cc1420bb06128eabfd719a2bc16762b751357d444d4bd`。

客户端已有 `.cargo/category-chain-helix` 四包patch仍为0.1.9且v3已就绪，本轮未改runtime源码、版本、manifest或锁文件。正式Helix仍为0.1.1，不能将两个版本的源码/二进制证据混为一体。

Go文档当前提供的真实项目入口是 `imApiHost=http://127.0.0.1:18068`，其余host保持PRE；HTTP base自动追加/api/cses。本轮没有操作此持续服务，没有复制PRE身份或向真实群发送。18067是历史fixture入口。日常18066/55432未动；真实PRE账号需在测试库正常新建会话，权限依实际成员决定。

## 验证及边界

- 新客户端合同首轮红24/24，修复后分类26 + TEXT9 + renderer5，共40 PASS；覆盖缺能力/旧版本/远端false/业务失败、超时不重试、两处账号切换、renderer销毁、nested DTO/精确版本和五读六写注册。
- App TypeScript、包边界、两仓diff check通过；完整 `cargo check --offline --locked --manifest-path src-tauri/Cargo.toml -j2` 通过，实际解析本地0.1.9 runtime。
- Tauri新增两项Rust测试2/2 PASS：alias到实际加载模块、读族req_id及30秒超时；未执行其余614个测试。
- 包构建被既有 `@ccc/core-common/context` 的 rootDir/TS6059阻塞；扩展回归中旧 lifecycle mock 缺CoreEventListener、旧parity分母遗漏im_stage_generated_media；lint命中原有 `let promise` prefer-const。均未改这些无关失败，不报告全仓Green。
- GitNexus CLI已尝试，但私有npm域名不可达。按包内规则降级为完整引用扫描，未宣称图谱影响分析通过。涉及现有高风险Gate，未新增运行时/业务owner。

真实Tauri窗口、PRE账号UI/DOM、Web/FFI、Pulsar/跨Pod、故障注入恢复保持未验收。Native fixture PASS不等于18068部署验收。下一步在UI捕获账号/频道/幂等键并等待匹配operation终态，再跑真实账号创建、参与、取消与详情流程；不可把accepted作为关窗/成功条件。description当前为字符串，若UI要结构化说明字段，需先冻结独立跨层合同。

本轮未commit、push、publish、部署或操作日常DDL。原始私有日志留在/tmp，不交付内容。
