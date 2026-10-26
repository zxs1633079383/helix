# Native 分类参与空资料兼容修复

当前验收：**LIVE_NATIVE_SMOKE_PASS**，限定本地单 Native 客户端 + 独占 Go fixture。修复后 run `category-native-1788793161209345000` 已由 Go 独立 Quality accepted；`TestCategoryChainNativeSmoke` PASS（包 7.214s）。Native PASS、result/cleanup 均 null、550 hops、overflow=false、11 路由无缺失；revision/eventSeq 3–7 逐次 WS ingress、EventSink 与 SQLite 对应。Go 终态为 1 Post、7 mutation、5 事件/outbox 全部 delivered、2 CANCELLED tombstone、0 活跃、RETRACTED revision7，安全 Card/anchor 一致。Go 独立只读清理后验确认专用分类 schema 0、六核心表均 0 行；本任务未连接或查询 Go 数据库。

实跑 binary SHA256：`b651e69f204dd523accfce72a2c89255ab6894e945ade090047224a1e9f8f766`，与正式回迁阶段校验值一致；空 User/Profile fixture 未填充姓名。另有非分类后台 HTTP NOT_SUCCESS，Go Store 全 scope 覆盖率 95.511% 仍 partial，本 smoke 不改变其门禁状态。正式 0.1.1 窄回迁已有 25 项分类回归与 check 证据；真实运行证据属于该 0.1.9 候选 binary。

证据：[Native脱敏记录](/tmp/helix-category-0.1.9-candidate-kortuqqf/runs/category-native-1788793161209345000/evidence.json)、[Go报告](/Users/cses-11/Desktop/cses-im-server/docs/reports/0907-分类接龙Native跨仓Smoke.md)、[Helix机器汇总](category-native-profile-compat-readiness.json)。旧失败和修复红绿记录保留如下。


原始运行 `category-native-1788792407693178000` 在 `set-participation` / corr 79 获得 HTTP 200 + CommonRes SUCCESS，随后 operation RECONCILING，未进入分类 storage dispatch。仅必要读取本轮 SQLite 的列结构和卡片版本：本地消息仍为 revision 3；Go 脱敏证据显示远端 revision 4、2 个有效 entry、1 个参与人。未读取输出原始身份/内容，未操作 Go DB。

第一失配在 `crates/helix-im/src/category_chain/wire.rs:1460` 的 `validate_entry_preview`。Go `internal/api/category_chain_realchain_test.go:109` 的隔离 fixture 仅创建 channel/member，不创建 User；`internal/store/sqlstore/category_chain_store.go:215` 对 User 不存在保留空 Profile；`internal/domain/categorychain/aggregate.go:350` 原样生成 preview。`aggregate_test.go:355` 明确测试空 Profile 快照在编辑、取消、分页间保持不变。Helix 原先多加了 displayName 最少 1 字符的限制。

原始 HTTP body 和 operation.errorCode 没有留存，故未声称恢复了原始错误文本。依据同一 fixture 与 Go 生产逻辑重建完整、脱敏、双分类非空 preview 的公开 Command 回放，红测得到：

`parse error: CATEGORY_CHAIN_AUTHORITY: CategoryEntryPreview.displayName length out of bounds`

生产修复只有该分类校验函数的注释及 displayName 下界 1→0；保留必填、字符串类型、2000 字符上界，保持原始空值，不填充姓名。非空 userId/entryId/content、decimal revision、viewer/chain/channel/mutation、card/Post 一致性、稳定 anchor 和持久屏障均不改。分页和共享 WS 复用该校验，不在 driver 或客户端另补业务规则。

新增 3 个公开 seam 测试：完整同人双类响应经过 Persist/readback 后唯一 CONFIRMED 与 canonical SQLite row 等价、重复回灌无重复事件；空资料在 entries 与 WS 原样保留；missing/null/number/超长 displayName、空内容、错误 viewer、非 canonical revision、card/Post 不一致仍拒绝且不 dispatch storage。既有失败/取消/timeout/迟到测试一并回归。

runner 仅增加脱敏诊断：operationError 保存来源、有限白名单原因和有界错误摘要；未知错误不输出原文。HTTP previewProfileShapes 最多记录 4 类×5 条和分页首 5 条的资料字段类型、空值、最多 2001 字符长度，不记录姓名/身份/参与内容或 headers。

验证：红测 2 fail/1 pass；最终分类生命周期 **25/25**；runner 离线测试 **4/4**；Native runner build PASS；热路径审计 PASS。中间新增 WS 测试把 channel eventSeq=4 喂给 cursor=0 而被正确 gap guard 阻挡，已修正测试为 eventSeq=1（category revision 仍为4），生产顺序规则未动。所有红/中间/最终日志均保留。

新 binary：`/tmp/helix-category-0.1.9-candidate-kortuqqf/target/debug/category-native-smoke`

SHA256：`b651e69f204dd523accfce72a2c89255ab6894e945ade090047224a1e9f8f766`

精确源码补丁：`/tmp/helix-category-0.1.9-candidate-kortuqqf/profile-compat.patch`。除 wire.rs、category_chain_lifecycle.rs、harness evidence.rs 三文件外，预先记录的 candidate crates/harness 源码及 manifest 哈希保持不变（README 另补当前状态入口）；core/host/native、普通消息、观测生产代码均未改。诊断阶段正式生产源未动；随后仅将分类空姓名修复和3项回归回迁正式0.1.1，见 [回迁证据](category-profile-compat-0.1.1-evidence.json)。Cargo版本和锁文件始终未动。无提交或发布。

此前等待 Go idle 后新 fixture 的协调项已由主任务/Go owner完成；本任务仅核对现有脱敏文件并同步状态，没有再次执行测试、改变源码/候选binary或触发其它任务。

**仍未验证**：UI DOM、Web/FFI、跨Pod/Pulsar、故障注入恢复、日常部署服务及所有非分类后台路径。本轮通过不等于三端或全后端门禁通过。
