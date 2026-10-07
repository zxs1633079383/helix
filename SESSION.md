🎯 EXPECT 书签计数由 Helix 缓存并增量更新；连续读取只发一次 HTTP，重复响应不重复增量，写入期间旧计数不能覆盖新状态；不修改版本或发布。 | 赌注 中 | 可逆 是 | @2026-10-07
- 本任务交接：docs/交接/bookmark-count-20261007.md
✅ DONE 本仓计数缓存实现、64 项定向测试和 crate check 通过；真实 Go/Tauri 接入、发布未执行，详见交接。

🎯 EXPECT 按群聊空值红测完成三仓定向修复，保留明确清空；不提版或发布。@2026-10-07
- 交接：/System/Volumes/Data/workspace/angular/visual/cses-client/docs/交接/1006-群聊空值覆盖审查/handoff.md
✅ DONE 本地源码与定向验证完成：Helix 222 项（含 6 项真实 SQLite）通过；三仓证据、安装包接入与真实业务验收边界统一见交接。

🎯 EXPECT 本轮仅修改 Markdown；历史转发证据缺口单列不可复核，公共包示例与当前 manifest 对齐；修改文档路径/版本/索引检查及 diff check 通过，不宣称全库或运行态 Green。 | 赌注 中 | 可逆 是 | @2026-10-07
- 本任务交接：docs/交接/Markdown日检-20261007/handoff.md
✅ DONE 本轮独立文档对齐与聚焦检查通过；七项历史附件标为不可复核，旧 Strict Matrix checker 失败单列，不宣称全库或运行态通过。提交/推送结果见交接所指外部报告。

🎯 EXPECT 全局书签新增独立只读命令，限定认证用户和企业；复用读回关联与失败结算，不改现有群内查询、版本或发布。 | 赌注 中 | 可逆 是 | @2026-10-06
- 本任务交接：docs/交接/global-bookmarks-20261006.md
✅ DONE 本仓实现：75 项定向测试、五 crate check、core 测试、wasm build 通过；全量套件被历史绝对路径缺失阻塞，真实链未验证，未发布。
⚡ DRIFT-PRED stop 未取消普通读关联，生命周期红测暴露后复用既有 identity-change 取消逻辑修复；详情见交接。
🎯 EXPECT 取消最后一个回应的空数组会随 cursor 原子持久化；HTTP失败能按 req_id 回传；不新增命令、表或版本。 | 赌注 中 | 可逆 是 | @2026-10-06
- 本任务交接：docs/交接/reaction-feedback-20261006.md
🎯 EXPECT 表情历史复用 im_recent_history，发送/回应独立、最近使用优先、重启恢复；不新增表、不改版本、不发布。| 赌注 中 | 可逆 是 | @2026-10-06
- 本任务交接：docs/交接/emoji-history-20261006.md
✅ DONE 本仓实现：公开回放4项、真实SQLite2项、crate check、局部fmt/diff通过；全包历史路径缺口与桌面/发布状态分别记录于交接。
🎯 EXPECT 指令优化只改规则、Skill和文档；生产代码、100%覆盖率、生产/发布/版本/指标批准不变；文档差异与技能元数据检查通过。 | 赌注 中 | 可逆 是 | @2026-09-23
- 当前任务交接：/System/Volumes/Data/workspace/golang/cses-im-server/docs/交接/Token消耗与指令审阅/handoff.md
✅ DONE 指令优化：规则与Skill修改，9入口元数据检查及白名单diff检查通过；未改生产代码/版本/质量门禁，未push。详细证据见任务交接。

# 0921 关闭群输入状态恢复

🎯 EXPECT 关闭终态与频道可写状态同事务持久化，重启及迟到 increment 不恢复输入；既有 terminal marker 可恢复旧缓存；不修改真实数据库、版本或发布。| 赌注 中 | 可逆 是 | @2026-09-21
- 分支 reactor/MessageV3-helix；用户已授权 Helix/客户端跨仓修复。

✅ DONE 源码与本地回归：新增 is_closed 持久布尔投影，同 terminal cursor 原子提交；旧库按 terminal marker 恢复，频道快照输出 close，在线关闭在 PersistOk 后通知。
- Helix 库442、Host SQLite9、同步GS1–GS4共8通过；五crate check、core测试、wasm构建通过。日志 /tmp/0921-closed-{rust-lib,host,sync,cargo-check,core,wasm}.log。
- 客户端定向首批98与追加批60通过（含重复），lint/包边界通过；关闭状态贯穿分页/当前会话/输入，WDIO真实桌面回归资产已添加。
- 用户原数据库只读备份到内存验证恢复 is_closed=1/delete_at=0；未改原库、版本、依赖、Git refs或发布。
- 交付 PARTIAL：已安装客户端仍固定registry 0.1.31，候选尚未发布/接入；WDIO未在候选二进制执行，不能宣称真实UI修复已生效。未重跑全仓历史失败Jest，不扩大无关修复。

---

# 0921 话题引用导致消息窗口超时

🎯 EXPECT 修正 render_scope 对消息话题引用的递归误判；未知、同公司和外公司话题引用均不改变父消息可见性，外公司主会话/消息继续拒绝；公开 latest 命令经过 PersistOk 发出 ready；不改版本、协议或运行数据库。| 赌注 中 | 可逆 是 | @2026-09-21
- 分支 reactor/MessageV3-helix，基线 f38abb700e388d626c6ebcacfe3b97b67ad4616d；用户已允许跨仓修复。

✅ DONE Helix 源码修复与定向回归：topic 引用不参与父消息归属扫描，主会话/消息 channelId 仍校验。
- RED：新增公开 latest 生命周期测试原先无终态，1 failed；/tmp/0921-topic-scope-red.log。
- GREEN：库 442、G-10 5、G-15 2、render_scope 15，共 464 通过；/tmp/0921-topic-scope-regression.log。
- cargo check -p helix-im 通过（既有 9 条警告），git diff --check 通过；未改协议、schema、版本或 driver。
- 用户实际缓存样本保留原 topic 后离线回放发出 im:timeline:ready；证据位于客户端 artifacts/timeline-timeout-20260921/fixed-comparison.txt。HTTP 回包为离线构造，不是真实服务验证。
- 交付仍 PARTIAL：客户端固定 registry 0.1.30；未发布、push、更新依赖、重启或执行最终候选真实 WDIO。桌面回归须接入修复候选后验证原会话及重新进入，不能以旧二进制替代。
- 无新 Harness card；沿用现有权限隔离和持久屏障回归。

---

# 0921 发布 Helix 0.1.30

🎯 EXPECT 用户授权发布最新Helix并更新客户端版本；按四公共crate依赖顺序发布0.1.30，八包版本图完整，客户端使用四个registry精确依赖。无Git push、桌面安装包发布或应用重启。

✅ DONE 四包0.1.30上传crates.io并逐一解析成功；八包cargo check及逐包dry-run通过。源码6bb1dc33包含删除态过滤和原始扫描游标修复；复用相同源码442项库测试及7项SQLite回归。证据 /tmp/0921-helix-release-{plan,publish}.log。发布版本事务随本条提交并建立注释Tag helix-v-0.1.30；客户端接入进度见主交接。真实Native/UI及前阶段全量Jest失败边界保持。

---

# 0921 频道同步删除态过滤

🎯 EXPECT 只修改 helix-im 频道同步/读取投影及对应 Host SQLite 回归；删除/移除频道在成员校验前过滤，零版本旧成员不污染页；游标按原始扫描位置推进，整页删除后仍能取到后续活跃频道；不改 core/driver 协议、schema、Angular 校验和包版本。| 赌注 中 | 可逆 是 | @2026-09-21

- 基线 cad6755a8936ea9690854bdad46a9d288f5dc3f6，独立分支 codex/0921-channel-sync-deleted。
- 主交接：/System/Volumes/Data/workspace/angular/visual/cses-client/docs/交接/0921-频道同步拒绝日志/handoff.md。
- 授权修复、验证；发布、版本与客户端接入尚未授权。DNS/重启由用户处理。
- 入口 im_query_channel_sync_page，本地确定性：Command → channel Scan → member ScopedScan → im:read:result；HTTP/WS N/A。
- 成功终态：仅输出活跃频道，hasMore/cursor保留原始扫描续页信息；Scope/Persist/解析失败沿用错误终态，corr一次消费。


✅ DONE 本地源码修复：删除/移除频道在成员扫描与投影前排除，分页先按原始行裁页再过滤，普通列表/快照复用删除态闸门。新增公开命令→真实内存 SQLite→typed result 回归，前两页全删除仍读取有效尾页。
- RED：修复前 channel_sync 库测试 10 通过/2 失败，命中未过滤和提前结束；/tmp/0921-deleted-red.log。
- GREEN：cargo test -p helix-im --lib 442 通过；Host increment_unread_anchor 7 通过；cargo check -p helix-im 通过（既有 dead_code 警告）。证据 /tmp/0921-deleted-{green,host,check}.log。
- 客户端空页续拉/完整频道同步定向 39 通过，定向 lint 通过；/tmp/0921-deleted-{angular,lint}.log。
- 基于本任务5文件diff验收，git diff --check通过；无 schema/core/driver 生产协议变化，无新 Harness card，协议文档同步原始行游标语义。
- 运行边界：客户端仍锁 registry 0.1.29；没有发布、push、更新客户端依赖、重启或真实 Native/Go/UI 验收。本地独立提交依根 AGENTS 完成态授权执行。

---

# 两轮未读跳转修复

🎯 EXPECT
- run_id=0920-unread-jump-fix；操作=退出群、两轮收新消息、分别进入。
- channel_id=47tkjnag4pyeiy8n8n1zgw95dr（历史只读证据；测试使用隔离 fixture）。
- authority=Go viewer read boundary + visible posts；Helix 持久化及版本/Emit；Angular 捕获进入交互快照并定位。
- 首轮 seq32，次轮 seq41；已读到47时 count0/anchor空；禁止从 seq+1 推断。
- 保留 unReadPostId wire / unreadPostId projection / unread_post_id storage 既有命名映射；显式空清除。
- 迟到/重复不倒退；PersistErr 不假报终态；Angular 快速切群取消旧定位，进入后新消息不拖动视口。
- 预期文件：Go increment store/app与相关authority投影，Helix channel member projection/write/tests，client typed sync/entry lifecycle/tests/WDIO。范围以源码取证收敛。
- 验证：先 focused 红绿，再受影响测试/类型/边界；真实 Native/Go/DOM 分层记录，不以 mock 冒充。
- 三仓独立 worktree；不 push、不部署、不修改共享业务数据、不自动调整包版本。
- 主交接：/System/Volumes/Data/workspace/angular/visual/cses-client/docs/交接/两次未读跳转定位/handoff.md

## 当前结果
- 实现与相关回归完成；独立复核发现的负 count 与 Sync 身份缺口已关闭。
- 最终 six-crate check/WASM build 通过；Host/IM focused 33通过，IM lib437通过。
- IM最终全量1572过/4失败/11忽略；四失败均同SHA基线复现。此前Core/IM/Host/Web全量1876过/7失败/12忽略，七失败同源错误完全一致。
- 新两轮回归基线失败、修复树通过；具体证据在 docs/交接/两轮未读修复/，绑定 verified-source-fingerprint.json。
- 状态PARTIAL（完整门禁/真实业务验收未关闭），未提交/发布；真实Tauri与Go端到端未执行。

---

## 2026-09-20 成员通知统一摘要发布 0.1.27

✅ DONE 四公共crate 0.1.27由正式脚本发布crates.io，八包check、四包dry-run/上传与registry回读通过。合并后IM1567 PASS/4基线失败/11忽略；Core24 PASS/1忽略，WASM构建及零运行时依赖通过。Client合并后91项通过。证据 /tmp/cses-notice-sync-0920/release-*.log。此提交固化版本事务，随后创建注释Tag并按授权push；客户端Cargo接入后续提交，未打包部署或做真实业务验收。

🎯 EXPECT 按用户授权合并 Helix !6 与 Client !27 后发布四公共 crate 0.1.27；精确依赖版本、registry 回读、发布提交及注释 Tag 一致，Client 仅更新 Cargo 依赖与锁文件。复用已记录业务回归，本轮验证发布与接入；不做桌面打包或真实业务操作。 | 赌注 中 | 可逆 首次上传前是

## 2026-09-20 成员通知在线/离线摘要身份

- EXPECT：无姓名 join/leave 的实时消息、离线历史、频道末条持久化/读回输出相同 summaryParts，且不含姓名快照；仅修改 helix-im 投影，无 DB/driver 变更。
- 原因：simpleMessage 固化“群成员”，宿主目录更新后无法恢复身份。新增有序文本/用户引用计划，复用旧字符串摘要规则；宿主仅解析用户身份，不重算业务文案。
- 生产范围：message_summary、member_notice、event/post、query/render_ready/core；测试先红后绿，lib 427 项通过；完整 crate 1562 PASS/4 FAIL/11 ignored，4项均在原基线复现；最终8项摘要回归与 cargo check 通过。全量后仅补空白公司别名处理并定向重验，未重新全量运行。未发布/部署或真实 Native/WDIO 验收。
- 证据 /tmp/cses-notice-sync-0920；独立 codex/0920-notice-summary-identity 工作树，父级交接在 Client docs/交接/0920-群成员摘要/handoff.md。

## 2026-09-20 群成员摘要 producer 补漏

🎯 EXPECT 四个既有 Helix producer（message-item、dialog lastPost、sync lastPost、exact posts）均复用 `attach_parts`；NOTICE join 产出无姓名的 `summaryParts`，撤回不产出旧 parts，exact 重建不丢失；只改四个 producer 与对应单测/本记录。红绿定向、cargo check 通过；不改摘要核心算法/Go/schema/driver，不跑已知全量失败。| 赌注 中 | 可逆 是 | @2026-09-20

✅ DONE 四个 producer 均已接入 `attach_parts`；message-item 撤回跳过，sync lastPost 与 exact 重建保留 `teamId/revoke`，保证二次 `prepare_post` 不丢公司作用域或复活撤回 parts。原 8 项摘要 + 本轮 sync/exact roundtrip 定向通过，`cargo check -p helix-im` 与 `git diff --check` 通过。证据 `/tmp/cses-notice-sync-0920/rust-producer-{red,green,check}.log`、`rust-producer-roundtrip-{red,green,check}.log`。未跑全量；真实 Go/WS、Native/UI 与已知基线失败仍未验证。已提交本地 commit（SHA 以当前 HEAD 为准），不 push。

## 2026-09-20 Rust重新打包与客户端集成

✅ DONE 四公共crate 0.1.25已发布crates.io并可解析，八包check/逐包dry-run通过；源码09535194、版本368dae8f与注释Tag helix-v-0.1.25已push。客户端093f7770aa四包精确pin+registry lock、用户库迁移已push dev-message；yarn start完成Angular构建78.076秒、Rust编译2分22秒并Running，命令退出0。保留旧客户端实例，不把编译/启动尝试当作新运行实例或未读跳转验收；应用环境版本未另提，既有release.json改动未提交。无CDN/桌面安装包发布/业务验收。最终远端回读与证据见客户端 docs/交接/0919-increment-unread-jump/handoff.md。

🎯 EXPECT 四公共crate 0.1.24→0.1.25发布可解析，客户端精确依赖同步，yarn start真实编译通过后再push两仓当前分支及版本Tag。用户授权包含最新用户库/公司闸门与unread映射修复；保留无关交接/产物和client release.json已有kernel修改，桌面应用版本等待环境选择。复用同代码身份已验记录；历史4项独立失败明确保留，不冒充全仓/业务验收绿色。 | 赌注 中 | 可逆 发布前是

## 2026-09-20 increment 未读定位字段修复

🎯 EXPECT 仅修 program.rs 与 channel_update.rs 的 unreadPostId 输入映射，并新增 Host SQLite 回归；真实 Go unReadPostId 写入 channel/member，缺失保留、空串清除，channelSyncPage 返回同一 ID。使用已有工作树迁移实现，保留所有既有修改；不提版、发布或切换客户端依赖。run_id=unread-anchor-20260920；基线1ca4e8534c835f50e090de63711f533dd1c8ee4f。先红后绿的定向 Host 测试、相关 IM 测试与 diff 检查。 | 赌注 中 | 可逆 是

✅ DONE 源码及本地回归完成：修复前真实 Go fixture 落 channel 的 anchor 为空，修复后 SQLite 两表及 channelSyncPage 返回同一 ID；2 项 Host 回归（含7种更新场景）+453项 IM 测试通过，cargo check -p helix-im 与限定 diff 检查通过。仅改上述2处映射、新增1个测试文件；保留其他任务改动。验证基于共享迁移工作树，未独立拆分提交；无发布/push/客户端依赖更新。桌面仍锁 registry 0.1.24，真实 Tauri/WDIO/线上 SQLite 未验，不能视为运行端已生效。详见客户端 docs/交接/0919-increment-unread-jump/handoff.md。

## 2026-09-19 用户级本地库与公司渲染硬闸门

🎯 EXPECT 同一用户 B 公司数据允许补水及持久化，但当前 A 公司在线/离线渲染投影中 B 数据为零；increment timeStamp 来自当前用户当前公司消息最大 createAt；物理库按用户隔离且旧库可恢复迁移。聚焦回放、SQLite 与三端编译分层验证，不自动发送真实消息、发布或 push。基线 1ca4e8534c835f50e090de63711f533dd1c8ee4f，保留既有脏文件。 | 赌注 高 | 可逆 是

PARTIAL：源码硬闸门/自动补水/用户库迁移已落地；render13、内部补水11、真SQLite共库/离线/水位、三端check/wasm、客户端local patch迁移9+路径3+generation1通过。全IM1552通过/7失败，相关3项修后补测53通过、消息id负例再测11通过；余4项独立契约/产物失败未扩修。客户端仍锁0.1.24，编译互锁阻止新库配旧包，发布接入待授权；Go C019本轮全覆盖未证，真实链/UI未验。未提交/发布/push/部署，CSES保持修复中。详见 docs/交接/task-6aacb31382481a450ebfd3aa/handoff.md。

## 2026-09-15 发布 Helix 0.1.22

🎯 EXPECT 用户已授权升级发布和 push：四公共 crate 0.1.22 发布并可从 registry 解析，发布提交及注释 Tag 推送，客户端升级精确依赖。本轮复用已通过的定向测试、三端编译和 WebDriver 布局证据；真实消息联调未验证，全量测试既有 mv3_g05l 失败不扩大修复。 | 赌注 中 | 可逆 首次上传前是

## 2026-09-15 最近转发成功关联

本地实现已验：durable sent/received 补 requestId，WS 与 sync 发送对账共用确认工厂；发送22、G05 3、库428、core、三端check及wasm通过。G05旧摘要断言按当前生产字段补齐；canonical由真实producer生成。全量测试指定当前reference_root后停在未修改的mv3_g05l负向断言。用户随后授权 0.1.22 升级发布及 push；本批先提交内核改动，真实后端联调仍未验证。

🎯 EXPECT 只给 durable post 终态补可选 requestId：HTTP accepted、落库失败、非本端转发不输出关联成功；发送和 G05 回放通过。不改 Go wire、版本或发布；分支 reactor/MessageV3-helix，基线 be863e4e42cab004294897e50c93a0173dd81b23。 | 赌注 中 | 可逆 是

## 2026-09-15 连续事件持久化屏障

✅ DONE Channel以已提交cursor、单在途序号和有界BTreeMap区分接收连续性与落库；连续突发无补偿，缺号/存储失败仍恢复，缓冲交接先占位防重复写。428库测试+13gate+9read+5新增回归全通过；09:27:18 CST开始，Cargo编译18.40秒，各测试套件合计1.23秒。首轮新增测试曾因PortOutcome错误枚举编译失败，已纠正为Storage错误；缺陷红测复用前阶段相同基线回放。不改Go/driver/client协议，未发布crate或做真实WS/SQLite/UI验收。GitNexus无Helix索引，按源码调用点与精确diff审查。

🎯 EXPECT Channel聚合区分已收到和已提交：连续type6→type1突发在PersistOk前不触发补偿，逐条回执后cursor连续推进；真实缺号/落库失败保持恢复。范围gate/flow.rs、聚焦回放测试及本记录；不改Go/driver协议。基线766b1f2c，reactor/MessageV3-helix；保留既有脏文件。 | 赌注 中 | 可逆 是

## 2026-09-12 Markdown 日检

✅ DONE CLI Trace环境配置、同步屏障/日志批处理入口与历史部署边界已对齐；955份既有Markdown清单、1701相对链接目标、Harness18/18/18及6项聚焦校验通过。全库逐句语义PARTIAL；未跑Cargo、真实Go/UI或线上采集。发布结果见本轮自动化报告。

🎯 EXPECT 仅修改有当前源码证据的Markdown，链接、Harness索引与相关checker通过；不改代码/配置/生成物，不宣称运行态Green。 | 赌注 中 | 可逆 是 | @2026-09-12T08:03:00+08:00
分支 codex/helix-markdown-20260912-0802；upstream origin/reactor/MessageV3-helix；基线 b3473970b3d7793527c04f7d0e7ecd1dca8846b0；worktree保留。

## 2026-09-11 Grafana修复发布

✅ DONE 四个公共crate 0.1.19完成八包check、逐包dry-run、上传和registry解析，发布脚本RELEASE PASS。真实业务验收交用户；发布提交/Tag立即创建，源码推送及客户端升级继续。

🎯 EXPECT 用户已授权发布0.1.19、推送源码和Tag、升级客户端依赖与PRE版本；真实业务验收由用户负责。合并最新远端仅涉及8份文档，生产代码与427项IM库测试及八包check通过的5a6fecb0一致。

## 2026-09-10 Grafana全量面板修复

✅ DONE 恢复与出口：恢复屏障回归先红后绿，IM库420、相关集成28、host diagnostics11、客户端observability10通过。第三/第四真实只读UI各46页全落库；恢复耗时14.487s/8.336s，pending0/mask0，UI success count2000。第四轮诊断accepted/exported9380一致，日志与metrics丢弃0，WDIO1/1通过2m10.3s。

✅ DONE PRE类型兼容：Collector 0.152.0仅将6个旧Histogram重命名为legacy_histogram，新Gauge保留原名；原生validate、dry-run、RV patch与gen10 rollout通过。6个新Gauge与6个旧Histogram桶同时真实暴露，Prometheus完成时间metadata为Gauge且有1789036006.023样本。Trace环境维度前序配置已验。

✅ DONE 看板v15：保留原111面板并追加1个全窗计数；10表分页/固定高度，回到最近3小时总览。STATIC PASS（112面板/171查询），最终全171查询API成功，96数据面板有有限真实样本、1缺完整实时WS cohort、15结构面板。比例空值明确采集不完整/窗口未完成，不把缺证据显示100%。

🎯 EXPECT 整体未全部验收：面板7仍需真实序号WS收到→落库→30秒窗口关闭证据；候选客户端用了local Cargo patch，Helix未发布新crate，Go PRE候选未push/deploy。交接与四轮证据位于Go docs/交接/Grafana面板缺失分析。用户已有CSES进程与其他任务脏改动保留。

## 2026-09-11 Markdown 日检

✅ DONE 四公共包README及docs入口版本对齐0.1.18，删除IM本地副本时态，补UpsertSpec源码入口；954份既有Markdown清单、1692相对链接目标、Harness18/18/18、Spec047默认/self-test、历史Matrix与索引均通过。全库逐句语义PARTIAL；Cargo/真实链未跑，canonical仍NOT_GENERATED；发布结果见本轮自动化报告。

🎯 EXPECT 仅修改有当前源码证据的 Markdown；四公共包版本对齐0.1.18，链接、索引和聚焦checker通过；不改代码/配置/生成物、不宣称运行态Green。 | 赌注 中 | 可逆 是 | @2026-09-11T08:01:37+08:00
分支 codex/helix-markdown-20260911-0802；upstream origin/reactor/MessageV3-helix；基线 f4def0494a9934587abe6e4ca5b107c768e478ad；worktree保留。

## 2026-09-10 发布 Helix 0.1.17

✅ DONE 客户端9c817d7abe四包精确pin/registry锁解析均0.1.17，PRE cargo check通过（49.25s）；Helix发布提交17921c21及注释Tag helix-v-0.1.17已完成。未push、未发布桌面包、未重跑真实业务链。

✅ DONE 四公共crate 0.1.17经八包check、逐包dry-run/上传及registry解析通过；本提交记录完整版本事务，注释Tag helix-v-0.1.17随即创建；客户端升级继续执行。未push、未发布桌面安装包。

🎯 EXPECT 四公共包0.1.17可从crates.io解析，完整版本事务提交并建注释Tag，客户端四包精确pin和锁解析升级且PRE cargo check通过；不自动push或发布桌面包。 | 赌注 中 | 可逆 首次上传前是 | @2026-09-10

## 2026-09-10 合并冲突解决

✅ DONE 两处清理冲突同时保留双方意图；五 crate check、55项IM聚焦、core测试、WASM build、rustfmt及diff check通过。仅本地合并，不push；真实链未重跑。交接见 docs/交接/0910-合并冲突/handoff.md。

🎯 EXPECT 仅人工解决 trait_impl.rs 两处冲突，同时保留同步观测取消与分类请求取消；五 crate check 与分类/同步聚焦测试通过；保留未跟踪产物，不 push。 | 赌注 低 | 可逆 是 | @2026-09-10T11:28:00+08:00

## 2026-09-10 Markdown 日检

🎯 EXPECT 基于 0cbbb6b59098793d2ae54fd6851c7419d01599fd，仅修改有源码证据的 Markdown；公共包版本与 manifest 一致，全库本地链接与聚焦 checker 通过，不修改代码/配置/生成物，不宣称运行态 Green。 | 赌注 中 | 可逆 是 | @2026-09-10T08:03:00+08:00
分支 codex/helix-markdown-20260910-0802；upstream origin/reactor/MessageV3-helix；独立 worktree helix-markdown-20260910-0802，保留不删除。

✅ DONE 本轮文档修正：四公共包版本0.1.15、ADR018及分类合同入口；分类11命令路由/view及历史reconcile私有回执与源码对齐。950份Markdown清单、1689条相对链接无缺失、Harness18/18/18、Spec047默认/self-test、历史Matrix与索引、diff check通过；未跑Cargo/真实链，canonical仍NOT_GENERATED。全库逐句语义未重证。 | @2026-09-10T08:09:10+08:00

## 2026-09-10 清理本地 pre-push 并继续分类推送

用户明确要求关闭 pre-push。来源提交32eec39c已删除初始化脚本；本轮删除共享Git目录残留hook，清理README、操作手册、ADR及交接文档中的安装建议，保留人工检查与CI入口。分类生产及Native实现保持b8757f7c，既有114项领域测试和最终跨仓分类复跑通过；旧发送套件11通过/3失败/7忽略保留为未关闭问题，不宣称全仓绿色。推送结果以本轮远端SHA回读为准。

## 2026-09-10 Native 分类开发候选通过

已合入be7c6f68并在开发0.1.15实际Go候选上运行Native分类全场景，1/1 PASS（8场景，3会话cleanup成功）。证据 artifacts/messagev3-phase1-4-real-chain/hx-p1-category-native-33408-8/category.json；保留开发Phase池/raw pump及既有SQLite清理，不覆盖新版shared实现。

## 0909 分类开发合并候选

本地隔离候选合并开发 11cbbaf8 与分类 32eec39c。五 crate check、IM lib 414、分类 39、重连 6、stress 3、Host 分类 6/guard 3、FFI parity 4 通过。保留开发发送终态、成员投影与事件序列守卫，保留分类版本守卫和事件读回。H3、新增非法 revision 回归与 Native 分类真实链仍须继续合入验收；未推送。

✅ DONE 消息摘要统一归 Helix，补齐系统通知末条及有界嵌套 forwardDetail；客户端已接入本地 0.1.16 修复副本。摘要、发送、频道与嵌套定向回归及 Native 实测通过；按用户授权提交源码，不提版、不推送、不发布。超大嵌套字节限制与按需展开暂缓。 | @2026-09-10

✅ DONE 用户追加授权修复摘要断言：15c58e98仅修改Host测试预期，Host check、SQLite关闭重开2/2、IM摘要38/38通过；文档链接1663条与Spec/Matrix检查通过。恢复本轮分类提交与目标分支发布，push仍需现有完整gate通过。 | @2026-09-09T09:39:18+08:00

历史阻断记录（已由15c58e98修复该断言，完整发布门禁待复验）：8 份 Markdown 的证据修正保留于独立 worktree；链接/版本/Harness/Spec/Matrix/历史索引/dry-run 通过。完整 gate 第5阶段因既有 markdown_persist::rich_markdown_command_survives_sqlite_reopen 断言失败退出101（实际 [图片] @张三，期望 [媒体]@张三）；不修改Rust/测试，不commit/merge/push。 | @2026-09-09T08:08:48+08:00
⚡ DRIFT-PRED 文档聚焦验证通过，但pre-push按crate目录升级全量gate并命中既有摘要断言；发布条件未满足，保留本轮修改待代码owner处理。

🎯 EXPECT 本轮 Markdown 日检基于 6a76cc4a，仅修改证据明确的 Markdown 与 SESSION；Cargo 版本说明对齐 manifest，全库本地链接及相关 checker 通过，不改代码/配置/生成物，不将历史验收升级为当前 Green。 | 赌注 中 | 可逆 是 | @2026-09-09T08:04:00+08:00
分支：codex/helix-markdown-20260909-0802；upstream：origin/reactor/MessageV3-helix；独立 worktree：helix-markdown-20260909-0802；本轮保留不删除。
✅ DONE Helix 0.1.14 四个公开 crate 已通过发布脚本的八包 check、逐包 dry-run、上传和 crates.io 解析；合并详情修复及摘要测试校正已提交。完整门禁未通过：PRE G10e 失败后终止剩余扩展验收，不宣称全量通过；客户端依赖升级和 Git push 待完成。 | @2026-09-09T15:57:41.206066+08:00

🎯 EXPECT 发布 Helix 0.1.14：四个公开 crate 统一 patch 版本并可从 crates.io 解析；发布后立即提交版本事务、创建注释 Tag 并正常 push。保留现有 artifacts；客户端原地 rebase、依赖升级和推送独立核验，不把编译当作实际版本 UI 验收。 | @2026-09-09T15:42:56.704536+08:00

✅ DONE 合并转发媒体源码修复：实时/历史/ACL共用投影；lib407+G05回放3通过，客户端6项及静态检查通过，源码测试包WDIO打开原有3图记录成功（20.9秒）。全量artifact_schema因旧机器路径缺少合同文件5项失败；正式客户端仍registry0.1.13，未发布、未push。 | @2026-09-09T14:37:33.737324+08:00

🎯 EXPECT 合并转发媒体修复：实时 authority 与历史 render-ready 共用同一 forward detail；RICH/IMAGE/VIDEO/FILE/AUDIO/VOICE 保留 durable props 与媒体顺序，fileName 收敛为 name；100项边界与脏JSON保持失败终态。聚焦producer回归和客户端消费验证，不发布、不push，不改既有 artifacts。分支 reactor/MessageV3-helix，保留当前分支。 | @2026-09-09T14:22:48.948637+08:00

✅ DONE 按用户明确授权分类提交并准备合并推送：Go 613a744/33037c6，Helix 4d1f1449 与既有同步/指标批次，客户端 a876d6e485；保留原覆盖率及未部署边界。 | @2026-09-08T18:47:33.376189+08:00

✅ DONE 同步广播与背压行为实现：Go 发起连接范围与64投影HTTP分页；Helix重复hello去重、单页PersistOk放行、断线关联清理。真Go→Native→SQLite2436行→EventSink ready、39页零增量广播通过；IM1444/core23/五crate/WASM/WSrace通过。Go完成态提交仍受C019 app98.1%门禁限制；未push/deploy。 | @2026-09-08T18:41:34.252140+08:00

🎯 EXPECT 完成同步行为修复：同一物理连接重复 hello 不重置恢复、不重复触发 HTTP；增量仅定向发起连接；恢复生产有界且不阻塞共享 Hub，跨 Pod 路径保持身份过滤与顺序，队列容量保持 2048；通过慢消费者、重连与清理回归，不将未部署代码冒充 PRE 修复 | 赌注 中 | 可逆 是 | @2026-09-08T17:47:47.887495+08:00

✅ DONE WS观测独立批次Go c77d940；测试engine指标批次Helix 6caea561。PRE定向3/3、Collector/Prometheus样本闭合。五crate check、core测试、WASM build通过。未push/deploy。 | @2026-09-08T17:37:24.988847+08:00

🚧 PARTIAL 200活跃群配额按用户确认关闭释放，Go全Store测试通过但覆盖94.4%未达仓库100%；Helix G08a 7/7、Angular定向15/15，真实发布后quota拒绝宿主链未验证。三仓quota差异保留未提交，避免混入已验收观测批次。同步广播/重复hello/批次流控完成静态审查，行为修复未实施。 | @2026-09-08T17:37:24.988847+08:00

🎯 EXPECT 200活跃群配额：Go按创建者与公司在事务内拒绝第201群，关闭释放；Helix业务失败产生带requestId的事件且不Persist/created；Angular匹配请求复用轻提示不报错弹框；定向测试覆盖并发、重复及资源清理。同步恢复与测试指标独立批次，不把未部署代码当PRE已修复 | 赌注 中 | 可逆 是 | @2026-09-08T17:00:24.344316+08:00

✅ DONE Helix 0.1.13 四包 dry-run、上传和 registry 解析通过，八包 cargo check 通过，发布前 helix-im 1437 测试通过；客户端升级继续执行。

🎯 EXPECT 发布 Helix 0.1.13 并更新客户端 registry 精确依赖；发送终态独立投递定向回归及 PRE Native 两项通过，真实 WebView 本轮未复现，历史根因与存储失败恢复边界仍保留。详见 docs/0908-发送终态独立投递与验证.md。

✅ DONE 发布 Helix 0.1.12：四公共 crate dry-run、上传及 registry 解析通过；完整八包 check 通过，包含发送衔接修复 10fc4000。客户端精确依赖、monorepo 合并及 yarn start 验证正在交付；三仓统一 Tag 0908-1617-basic-message 待绑定最终提交。

✅ DONE 源码修复：查询代际与 WS 断线不再吞掉已落库发送衔接，身份切换仍撤销旧回包。18 项发送、1,434 项 IM 全量、3 项 release 承压及 PRE 真链 G01A/G02C 2/2 通过；未发布新 crate/客户端，未验收 Angular 感叹号及网络容量。详见 docs/0908-发送衔接竞态修复与验证.md。

🎯 EXPECT 修复附着时间线查询代际淘汰导致发送与 15 秒 timer 丢失：公开 ImModule::handle 红绿回归，覆盖交错并发、乱序、重复与超时；保持 P1 持久化门槛及旧窗口不回写，不把回放当容量或桌面验收 | @2026-09-08

✅ DONE 0.1.11 跨仓发布交付：四包 crates.io 发布成功，发布提交 5ba66ba9 与注释 Tag helix-v-0.1.11 已推送；客户端 884857a04a 精确依赖已推送 monorepo-message，PRE cargo check 与剪贴板 7/7 通过，忽略的本地锁文件四包来源为 crates.io。Go dev-paas2.5 已推送 55e05c5，PRE c391ddcf Ready/Available=1；Helix core/IM 1453 项、八包 check、Web/WASM 与 pre-push 检查通过。仅完成 crates 发布及客户端依赖升级，未发布桌面安装包，Angular 真页面终态仍待验；三仓既有非本任务配置/产物保留。

✅ DONE Helix 0.1.11 四个公共 crate 已经 canonical 发布脚本完成八包 check、逐包 dry-run/上传和 crates.io 解析；core/IM 154 组 1453 项测试通过。版本提交及 helix-v-0.1.11 Tag 绑定此批 manifest；客户端依赖更新与开发分支同步继续执行。

🎯 EXPECT 发布四个公共 crate 0.1.11，同步八个 workspace manifest 与 Cargo.lock；四包可从 crates.io 解析、发布提交与注释 Tag 同源；客户端精确 pin 升级且保留既有配置改动，三仓当前开发分支同步远端 | 赌注 中 | 可逆 首包上传前是 | @2026-09-08T14:29:10+08:00

✅ DONE 本轮 Go PRE/Helix Native 指标修复与证据 @2026-09-08 | Go CI35/CD16 已部署；Helix provider身份+Cumulative+selfcursor修复，19定向测试、最终Native在线/离线/OTLP故障发送双端SQLite与155双MCP对拍。现役Tauri crates发布及Angular终态仍PARTIAL；见 [验收说明](docs/Metrics/0908-PRE面板证据/验收说明.md)。


🎯 EXPECT 补全双端生产观测接线：持续有界采集、精确字段全量导出、消息身份与Trace关联、真实终态统计、默认客户端接线和看板；可证伪：全部看板业务查询有对应producer，身份筛选无全局误归属，聚焦失败/去重/重启测试通过；仅最后真实用户操作可保留待验 | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE 本轮独立业务诊断producer与Native采集验证：真实Go双身份收发seq5、离线重登seq6，两端SQLite与Loki对账通过；Promtail 6/6上线，看板v3与332查询通过。Angular真实页面、PRE新二进制与严格端到端率仍未验收，不将本批提交称为生产闭环。详见 docs/Metrics/业务埋点与真实链验收-2026-09-07.md。
⚡ DRIFT-PRED 本机diagnostic-local不在既有Metrics环境白名单，样本归unknown；新队列Gauge单位1出现ratio后缀，源代码修正为空单位 → 候选教训：新增Gauge必须核对Collector实际输出名与资源标签。
🎯 EXPECT 补齐Helix/Angular真实登录同步、WS接收与匹配持久终态、缺口区间和群水位诊断，接入有界Loki导出及OTel低基数自观测；可证伪：诊断不改变Effect/SQLite结果，身份与seq不进入时序索引，失败提交不报成功，聚焦失败路径/采集新样本/中文行为标题通过；真实宿主缺口必须保留，不push | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE 用户追加授权后已将独立看板发布到 PRE Grafana Helix 文件夹，UID helix-client-channel-v1、版本1；98个面板/分组、164条查询与变量回读一致，绑定现有Prometheus/loki-pre。未修改数据源或业务producer，诊断无数据与浏览器未验收边界保留，凭据不落盘、不push | @2026-09-07T15:52:07+08:00

🎯 EXPECT 沉淀客户端实测指标清单与用户/群聊诊断设计，生成独立 Grafana 看板；可证伪断言：① 71 个实测 Helix family 与快照逐项一致；② 用户/群聊筛选不把无身份标签的全局值当作个人数据；③ 登录、落库、Gap、seq 区分当前接入与拟新增；④ JSON/生成一致性/查询及 diff 检查通过；⑤ 不修改 Rust/Angular/Go 业务、运行配置，不部署、不 push，用户未跟踪文件保留 | 赌注 中 | 可逆 是 | @2026-09-07T15:17:43.008550+08:00

✅ DONE 本轮文档/看板artifact已完成：docs/Metrics沉淀71族实测快照、登录/用户/群聊/Gap/seq设计与验证记录；独立评审APPROVED_ARTIFACT_ONLY，独立UID看板含98个面板/分组及164条查询。静态生成一致性、42个文档链接、Node语法、diff检查与PRE顺序只读332/332查询通过；71族对应151条查询有数据，4个Span派生族因缺环境维度保留隔离为空。业务诊断producer/OTLP Logs pipeline/latest-state服务未实施，Grafana未发布、未做真实业务或UI验收，未改Rust/Angular/Go及用户既有未跟踪产物 | @2026-09-07T15:42:43.669094+08:00

⚠️ 发布BLOCKED：本轮G02C修复后PRE83/83，完整门禁Native113/113；旧Phase1 localhost:8066前置失败，改PRE诊断另有G01K/G02B两项独立失败。遵守无关失败停止扩张规则，诊断改动已撤销，未绕过hook；push失败、MR未创建。详见0907-PRE四项验收复核与修复报告。
✅ DONE G02C后续根因修复：HTTPauthority原先提前发sent，晚WS已被cursor去重；改为窄写集PersistOk后received。403个lib、6个G02C focused、满队列observer关闭回归和最终PRE83/83（53.195秒）通过。正常pre-push结果另记MR，不绕过hook、不自动合并。
✅ DONE PRE验收设施独立批次：统一有界观察预算，失败分类/关联/清理证据及pin原始WS断言；聚焦3/3、固定远端基线PRE82/83，G10E/G05F/G05G通过。G02C已落库sent却无匹配received，当时业务修复仍PARTIAL，未创建MR。
⚡ DRIFT-PRED 起始本地基线83/83未在远端基线上保持；G02C重复超时，提前独立订阅终态仍失败，撤销该假设改动。禁止把观察设施修复称为生产根因闭环。
🎯 EXPECT 修复PRE四Gate失败：定位重试/置顶seed终态和notify重开回读；保持现有业务断言，聚焦红绿与PRE --all验证；仅在独立worktree提交并提MR，不纳入主仓脏文件 | 赌注 中 | 可逆 是 | @2026-09-07T17:32:36.969089
分支：codex/0907-pre-four-gates；基线 a288f7ae；目标 reactor/MessageV3-helix；无自动删除

🎯 EXPECT 将用户明确授权的 PRE 测试账号池内置到 real-chain 配置；可证伪断言：① `--env pre` 在无凭据变量时自动注入 16 个测试账号且不打印密码，已有 JSON/File 仍可覆盖；② local/未来其他 profile 不误用 PRE 账号；③ CI 不再要求运维配置凭据，只要求 Node.js 20+ 与现有 Rust/Cargo；④ Node 凭据选择断言、83 Gate dry-run、GitLab CI lint 与 `git diff --check` 通过；⑤ 只提交脚本/CI/SESSION，不纳入当前 helix-im 用户改动与未跟踪产物 | 赌注 中 | 可逆 是 | @2026-09-04T10:01:00+0800

🎯 EXPECT 本轮只修改 CI 与诊断脚本；所有作业 timeout=6m，诊断前置逐项限时且失败退出，Cargo 超时后终止进程组；不修改 Rust 业务代码 | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE PRE real-chain 已内置 16 个用户明确授权的测试账号，无凭据变量时自动向 Native Gate 注入且不输出密码；显式 JSON/File 可覆盖，local profile 保持隔离。CI 已删除凭据变量前置，只保留现有 Rust/Cargo 与 Node.js 20+ 要求；账号池/覆盖/隔离断言、83 Gate dry-run、Node 语法、GitLab CI lint 和 `git diff --check` 通过。未运行远端 PRE 写入回归，当前 helix-im 并行改动及未跟踪产物均未纳入 | @2026-09-04T10:04:00+0800

🎯 EXPECT 将 PRE real-chain 凭据入口从强制 GitLab File 扩展为 Masked JSON 环境变量；可证伪断言：① `CSES_WDIO_CREDENTIALS_JSON` 优先且不写入仓库、日志或 artifact，File/local path 仅作兼容兜底；② 账号发现仍按稳定顺序只选出三个共享 `companyId` 的身份，不随机制造不可复现失败；③ CI 命令固定 `--env pre`，同一 PRE 门禁串行执行且 30 分钟有界；④ credential focused test、GitLab CI lint 与 `git diff --check` 通过；⑤ 不触碰既有 artifacts、tmp 与 Cargo 私仓 Prompt | 赌注 中 | 可逆 是 | @2026-09-04T09:40:23+0800

✅ DONE PRE real-chain 已支持 `CSES_WDIO_CREDENTIALS_JSON` Masked 内联账号池并保留 File/local 兜底；账号仍以稳定顺序寻找三个同租户身份。GitLab job 固定 `--env pre`、同资源组串行、30 分钟有界并校验 Node.js 20+；credential focused test 4/4、PRE profile 三端点断言、GitLab CI lint 与 `git diff --check` 全部通过。未运行远端 PRE Gate，未写入或输出账号明文，既有 artifacts/tmp/私仓 Prompt 保持未跟踪 | @2026-09-04T09:44:00+0800

🎯 EXPECT real-chain `--all` 将按 Phase 拆为 5 个 Cargo 批次；每个批次最多 5 个 run-owned 频道、按账号复用一条长连接并保活，Phase 末统一关闭；Gate 的 HTTP/WS/Projection/SQLite/EventSink/corr/cleanup 证据字段不削弱；Focused Rust/Node 静态校验通过，并记录一次 PRE 执行的 Cargo 进程、频道创建、WS 连接和总耗时对照 | 赌注 高 | 可逆 是 | @2026-09-03T18:35:38+0800

🎯 EXPECT 每日 Markdown 熵管理只修改有当前代码/manifest 证据的文档；可证伪断言：① 四个公开 crate README 与 docs 索引统一对齐 Cargo `0.1.4`；② Metrics 文档精确对齐 registry 的 182/129/40/11/2 分母与新增 6 个 MessageV3/Web Runtime 指标；③ docs/mmd 的 92 Gate 客户端合同入口从已落后的 sync-event-alignment worktree 切到当前 cses-client 仓根，92/92 contract 路径存在且 Phase 33/20/15/12/12；④ 不修改 Rust、JSON、TOML、脚本或生成产物；⑤ 本轮直接相关链接、索引、manifest 与 diff checker 全部通过，既有 checker Drift 单列不冒充本轮失败 | 赌注 中 | 可逆 是 | @2026-09-03T08:15:00+0800

✅ DONE 每日 Markdown 熵管理已对齐 0.1.4 版本、182/129/40/11/2 指标分母、92 Gate 当前 cses-client 真源、ROADMAP 与历史设计文档边界；90 个变更文件仅为 Markdown/Mermaid，本地链接 1589/1589、Gate 92/92、Mermaid 94/94、HX-C 18/18、Spec047 30/30 默认校验、strict-matrix 50/54/13 与 diff check 通过；既有 generated/agent/full-map checker Drift 已单列，未冒充本轮失败 | @2026-09-03T08:30:03+0800

⚡ DRIFT-PRED 原预期把新增量写成“6 个 MessageV3/Web Runtime”，源码 registry 实际为 7 个 MessageV3 Runtime 修订且 `WebEventBatchSize` 仍为 Planned → 候选教训：指标分类必须同时从 availability match arm 与真实 caller 计算，不能按名称分组估算 | @2026-09-03T08:30:03+0800

⚠️ PARTIAL 三仓远端合并后 Helix Phase1–5 最终回归 @2026-09-01 | 集成 HEAD `193f6e3e`，Go fresh Pod `41726d…` 与 PRE ES forward 上执行 `scripts/messagev3/phase1-5-real-chain.mjs --all`，summary `72 PASS/92`，5 NOT_VERIFIED、6 BLOCKED_PRECONDITION、4 NOT_IMPLEMENTED、5 NOT_APPLICABLE，runner exit 1；Cargo 94/0 通过；未 push/未打新 Tag | 详见 `docs/reports/0901-跨仓Phase1-5验收报告.md`

🎯 EXPECT 修复指标 Skill/Agent 的全局发现路径；可证伪断言：① Skill 同时存在于 `/Users/xpa-12/.agents/skills/otel-prometheus-metrics` 与 `/Users/xpa-12/.codex/skills/otel-prometheus-metrics`，含 `SKILL.md` 与 `agents/openai.yaml`；② Agent 以可解析 TOML 安装到 `/Users/xpa-12/.codex/agents/helix-otel-prometheus-metrics-agent.toml`；③ bundle installer、README、evaluator 同步且不触碰既有未跟踪 artifacts；④ 官方用户目录、全局 Agent 内容校验、Skill/bundle/diff 检查通过；⑤ 需新任务或重启 Codex 刷新 slash 索引，不把当前旧会话列表冒充已刷新 | 赌注 低 | 可逆 是 | @2026-08-31T14:02:00+0800

🎯 EXPECT 在 `.codex/autoresearch/helix` 沉淀一个带 review gate 的 OTel/Prometheus 指标 Agent 与渐进式披露 Skill；可证伪断言：① 新增专用 Agent、SKILL.md、只读 Prometheus 验证脚本和按需参考，且 Skill frontmatter/开放元数据可校验；② review 未输出 `REVIEW: APPROVED` 时不允许修改指标生产代码；③ 指标只经既有 bounded `AsyncMetricSink::try_record`，不在 core/业务主线程执行网络 I/O、不新增无界队列或高基数标签；④ 验证区分 focused、worker/export、Collector scrape、Prometheus metadata/query 和真实业务触发，HTTP 200/canary 不冒充采集成功；⑤ Router、manifest、README/引用入口同步，保留既有 artifacts 与用户未跟踪文件，`git diff --check` 和全部结构校验通过 | 赌注 中 | 可逆 是 | @2026-08-31T13:20:59+0800

🎯 EXPECT 收敛 Spec051 发布前基线并发布 Helix 0.1.2；可证伪断言：① 四个旧 `ImConfig` 测试构造显式补齐宿主平台且不改变 Effect/ACK 合同；② `cargo fmt --all -- --check` 与 `cargo test -p helix-im` 全绿；③ 发布脚本回归不再写死上一版本，`--plan`、8 包 check 与真实 publish 全部通过；④ 客户端切到精确 registry 0.1.2，移除持久 `HELIX_ROOT` 后 `yarn start` 不再临时 path 化；⑤ WDIO 只完成前置审计，不经用户确认启动真实 lane | 赌注 高 | 可逆 发布前是、首包上传后否 | @2026-08-29T

✅ DONE Helix `0.1.2` 版本事务已完成：发布脚本按 core→im→host→native 顺序执行 package dry-run、8 包 check、真实上传与 registry 轮询，`cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 四包均已可解析；8 个 workspace manifest 与 Cargo.lock 已同步到同批依赖图。未创建 tag、未 push，等待客户端精确 registry 依赖切换与 WDIO 前置审计收口 | @2026-08-29T

✅ DONE 客户端 worktree 已提交四个精确 registry `=0.1.2` 依赖，metadata 确认均来自 crates.io；`.zshrc` 唯一持久 `HELIX_ROOT` 已移除且新登录 shell 为 unset，常规启动包装器不再临时 path 化。ignored `release.local.json.imApiHost` 已指向当前 Go worktree 宿主 `http://127.0.0.1:18066` 并通过 Phase1/2/3 release-local 解析与 health 校验。WDIO 尚缺 node_modules、只读账号池、candidate、dist/browser、完整 Xcode 与 ONNX staged dylib，未获用户确认也未启动真实 lane | @2026-08-29T

✅ DONE 发布前测试债务已收敛：旧 `ImConfig` 构造补齐 `client_platform`，全量运行暴露的旧事件名、查询投影、权威 WS echo 与原子持久化断言已对齐当前合同；`cargo test -p helix-im -- --nocapture` 全绿（含 100k 顺序事件、5k WS gap flush、5k sync backfill），`cargo fmt --all -- --check` 通过。Native 本地 lib/integration/stress/sync targets 全绿；workspace 仅剩 `message_v3_domain` 73 个真实链 Gate 因默认 8066 与当前 Go worktree 宿主 18066 未对齐而阻断，保持外部前置缺口，不冒充整体 Green | @2026-08-29T

🎯 EXPECT 按已批准 ADR 统一 WS 与 HTTP sync 的 CanonicalEvent/reducer；可证伪断言：① WS 根 `seq` 只保留为 delivery metadata，任何业务 cursor 只接受持久 `channel_event.event_seq`；② 在线与 sync 对同一事件集经过同一 reducer 后，`channel_member` 的绝对 unread/projection revision、message 与 cursor 在实时、断线回补和重启后完全一致；③ 重复、乱序、缺口及 PersistErr 不会双算未读或提前推进 cursor/revision；④ 31 个注册 WS action 与 Go 生产 action 的映射/明确忽略合同均有回放断言；⑤ 只修改 helix-im 与严格必要的 host/schema/测试/文档，不改版本、不 push、不把 Mock 或单侧 Emit 冒充三端真实通过 | 赌注 高 | 可逆 是 | @2026-08-28T

🚧 IN-PROGRESS WS 根 `seq` 已与 `streamSeq` 分离；`channel_stream_event` 与 HTTP `streamEvents` 共用 `EventEnvelope`/storage 语义，redacted marker 只推进 cursor，full 正文缺失 fail-close；`memberProjection` 与 message/cursor 同一 PersistAtomic，`channel_member` 持久化 revision/effect/membership epoch；canonical 在线、buffer flush、sync 与 terminal prefix 已禁止本地 unread delta。合入 Helix 0.1.2 基线后，`cargo fmt --all -- --check`、`cargo test -p helix-im`、五 crate check、core tests 与 Web/WASM 构建通过；真实 Go+WS+SQLite+重启/乱序链尚未执行，任务不得标 DONE | @2026-09-01T
## 2026-09-10 Native 分类真实链通过

message_v3_domain::category_chain_native 使用隔离 Go HTTP/WS + 三个 Native engine + SQLite + EventSink，覆盖能力、创建(maxCategories=2)、发布、成员接收、参与/取消、历史receipt revision3与公共revision4、重复对账、真实SQLite故障无成功终态；三个会话关闭且SQLite删除后证据才标PASS。日志 /tmp/category-gfix-evidence/native-category-final3.log。此为源码候选，不代表Angular或远端部署。

## 2026-09-10 分类 revision 非法输入关闭

CATEGORY_CHAIN 的 live post、post_update、posts_update 与 sync 快照在 gate/cursor/storage 前统一校验 revision，拒绝缺失、null、数值及非 canonical 值；合法同 seq 可恢复，普通消息不受影响。公开 Tick/PortReply 回归 7 项通过，原实现 5 项红测；未作 Native/UI 验收。

🎯 EXPECT 为 CATEGORY 接龙加入可选 `maxCategories` 协议字段；可证伪断言：① 新草稿 `max_categories` 仅在存在时映射为 Go HTTP `maxCategories`，且只能是 `1..=categories.len()` 的 JSON 整数；② Card、CategoryChain 与 myParticipation 只接受该一个可选字段，旧 NULL 投影省略它，RETRACTED 卡不得携带它，其余未知字段仍 fail-closed；③ HTTP、WS、draft、mine、CanonicalPost/read-back 均复用同一个 Card 值，空 `selections` 和 `PARTICIPATION_LIMIT_EXCEEDED` 原样透传；④ capabilities 仅新增 `postChainCategoryMaxCategories=true`，wire v3/schema 1 不变；⑤ focused red→green、`cargo check -p helix-im`、`cargo test -p helix-im` 和 diff check 留证，真实 Go/WS/SQLite 链标 UNVERIFIED。run_id=helix-category-max-20260909; corr=req_id/category chain command; channel_id=1; expected_event=im:category_chain:{operation,projection}; expected_fields=maxCategories optional absolute projection | 赌注 高 | 可逆 是 | @2026-09-09T00:00:00+08:00

## 0. 本轮分类合并修复

✅ DONE H3 本地修复：历史 CONFIRMED receipt 独立持久化并读回后结束操作，较新公共 head/message 不倒退；正常 reconcile 继续更新公共投影。category_chain_lifecycle 40/40、helix-im check、两文件 rustfmt 和 diff check 通过。Native 真实链与开发分支合并待完成。

分支：codex/0909-category-merge-fix；基线origin/helix-message-test@32eec39c；目标reactor/MessageV3-helix。独立worktree，保留主工作区修改。

🎯 EXPECT 历史CONFIRMED receipt低于公共head仍结束操作且不回退持久head；新增driver-native/tests/messagev3分类回归并实际通过；不修改core/ABI/schema | 赌注 中 | 可逆 是 | @2026-09-09T18:23:06.546548+08:00

✅ DONE 分类接龙上限协议已完成：`max_categories` 缺省不进入 HTTP，显式整数仅允许 1..当前分类数并映射为 `maxCategories`；Card、Chain、myParticipation 只新增这个可选字段，历史 NULL 保持省略，RETRACTED Card 禁带字段，Card/Chain 与 Card/mine 不一致均 fail-closed。公开 Command→HTTP、HTTP 回包→Persist/read-back、WS 均由同一真实回放覆盖，超限业务码保留。index 前 wire 的公开红测以 `unknown or authority field 'max_categories'` 失败后恢复当前 hash；分类 target 39/39、`cargo check -p helix-im`、diff check 通过。全量 helix-im test 仍由既有 `stress_test.rs` 缺 `ImConfig.client_platform` 阻断；未运行真实 Go/WS/SQLite/Native，且未同步客户端 0.1.9 runtime。| @2026-09-09T00:00:00+08:00

🎯 EXPECT 分类set_participation/publish只新增可选user_snapshot→HTTP userSnapshot展示字段；Go保持身份权威；正式wire和公开seam测试先红后绿，再对客户端0.1.9候选仅同步这两个文件的本轮增量。TEXT五文件、runtime/facts、版本/锁/其他并发改动不变，不提交/重启；记录候选前后SHA与验证 | 赌注 低 | 可逆 是 | @2026-09-08

✅ DONE 分类参与/发布新增可选展示快照user_snapshot→userSnapshot，userName/deptName/orgName不改名、不替代Go身份；主仓RED3失败→GREEN35分类/371lib/三端check/wasm；候选仅wire与四项测试追加，临时副本33分类通过，实际候选SHA匹配，TEXT/runtime/facts/版本/锁未变，无提交或重启。报告 docs/reviews/2026-09-08-category-chain-user-snapshot.md；真实Go/UI由协同任务验证，本任务未运行。 | @2026-09-08
⚡ DRIFT-PRED 客户端impact补跑在候选同步后，CLI先遇registry DNS失败、改用既有runner仍因目标未索引返回UNKNOWN；未记作低风险，以公开入口和调用点补充核验，保留图覆盖与时序偏差。候选初测缺既有fixture，只复制原fixture到临时目录后33通过，未改制品。

🎯 EXPECT 修复分类审查五项：普通Post迟到不倒退、本人权限同参与版本可刷新、Web空Get与CAS冲突可恢复、fact读回不受旧路径截断；公开seam先红后绿，Host/Web存储及三端Rust检查通过；不修改Go/客户端/版本/锁文件，基线备份 /tmp/helix-category-review-baseline | 赌注 中 | 可逆 是 | @2026-09-08

✅ DONE 本轮分类优化的本地实现与聚焦验证完成：旧Post/编辑/批量编辑及缓冲释放保持revision10；本人同参与版本权限刷新；Web空Get/CAS恢复；fact先按版本过滤再limit；Pending枚举去magic phase。TEXT五个领域/命令/WS/提交文件逐字不变，九路由兼容与混合批次失败隔离通过。IM lib371、分类31、Host6+3+21/89、Web40、core23、三端check/wasm及Native release7项通过；报告 docs/reviews/2026-09-08-category-chain-optimization.md。未提交/push，未改Go/客户端/版本/锁/旧制品，真实Go/UI本轮未重跑。 | @2026-09-08
⚡ DRIFT-PRED 全量IM验证遇到未改动旧测试缺ImConfig.client_platform → 保持聚焦修复边界，单列历史阻塞，不把局部通过称为全仓绿色；旧/workspace/java日志路径不在当前可写环境，偏差记录保留于本报告。

分支：reactor/MessageV3-helix @caf20f3b；upstream origin/reactor/MessageV3-helix；保留分支。

🎯 EXPECT 本轮恢复当前 monorepo 分类命令桥接并在55441独立schema验证CATEGORY_POST_CHAIN_WIRE/v3五表真链；11语义/9路由/3视图有HTTP、WS、SQLite、Emit与清理证据，客户端UI既有文件保持逐字不变且分类按钮不启用；不提交发布，不操作日常18066/55432 | 赌注 中 | 可逆 是 | @2026-09-08

✅ DONE 当前monorepo客户端补齐11个分类Gate/Tauri入口、能力版本门禁、nested DTO与canonical事件透传、捕获账号代次保护；原23个dirty UI文件hash不变，按钮禁用。Native五表真链category-native-1788838907898449000 PASS，550 hops/9路径/3视图/11语义、Go outbox与revision3–7匹配、schema remaining=0；客户端40测试/TypeScript/boundary/Tauri check/2Rust测试通过。包构建与广回归有既有无关失败，UI/DOM未验收；报告docs/category-post-chain-v3-current-integration.md，无提交发布 | @2026-09-08

🎯 EXPECT 按CATEGORY_POST_CHAIN_WIRE/v3只增加分类query mode=CATEGORY与wire版本3及runner query/JSON重复mode取证；正式/0.1.9公开seam红绿、TEXT九动作不变；产出持久副本精确patch与新runner，旧v2制品/版本/锁文件不动、不live，等待Go五表 | 赌注 低 | 可逆 是 | @2026-09-08

✅ DONE（V3_BUILD_READY_WAITING_GO_FIVE_TABLES）分类query/body mode、版本3feature与严格query/重复body mode取证完成；正式29分类/25chain/check，0.1.9 29分类/7runner/final build和audit/diffcheck通过。增量patch已只读apply-check交主任务，新runner路径/hash见docs/category-post-chain-v3-readiness.json；TEXT与旧v2制品/证据未动，未live/DDL/部署，等待Go五表。

🎯 EXPECT 按CATEGORY_POST_CHAIN_WIRE/v2保留分类Native入口，仅wire复用post-chain+mode=CATEGORY及get view=card|mine|entries，新增postChainCategoryMode能力；公开seam红绿与TEXT九动作缺mode回归通过，产出client持久0.1.9源码精确patch，旧runtime/配置/版本/锁/普通TEXT源不动，无部署 | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE（V2_BUILD_READY_WAITING_GO_FIVE_TABLES）v2统一mode/view wire+feature已实现；正式29分类/25chain/check、独立0.1.9 29分类/6runner/build通过。精确持久runtime补丁已只读apply-check并交主任务，新runner路径/hash见docs/category-post-chain-v2-readiness.json；旧binary/run保留，无live/DDL/部署，等待Go原五表实现，旧七表PASS仅历史。

🎯 EXPECT 只核对修复后run脱敏证据并更新分类合同/报告状态为限定LIVE_NATIVE_SMOKE_PASS；保留首次失败历史与UI/Web/FFI/跨Pod/部署未验证边界，不改代码/版本/candidate binary，不运行测试或触发其它任务 | 赌注 低 | 可逆 是 | @2026-09-07

✅ DONE 已核对category-native-1788793161209345000：Native PASS/550 hops/11路由齐全，revision3–7 WS+EventSink对应；Go独立Quality accepted与terminal DB/outbox/清理后验由Go报告佐证。Helix合同与机器证据已同步限定LIVE_NATIVE_SMOKE_PASS，原失败历史保留；无代码/测试/版本/binary变更 | @2026-09-07

🎯 EXPECT 将已证实空displayName兼容窄补丁同步到正式0.1.1分类wire与公开seam测试；原函数逐字一致，先红后绿，25分类测试及helix-im check通过；除两处源/测试文件与文档外不改，版本/锁文件/普通消息/观测源和正在联调的0.1.9候选binary哈希保持不变，无提交发布 | 赌注 低 | 可逆 是 | @2026-09-07

✅ DONE（BACKPORTED_FOCUSED_PASS）仅将空displayName下界与3公开seam回归同步正式0.1.1分类功能；先红2失败/1通过，绿25/25、helix-im check及热路径审计通过。原0.1.1测试前缀（含im:post:received）逐字保留；其他源/manifest/lock及联调中0.1.9候选源码/binary SHA未变。窄patch与前后hash见docs/category-profile-compat-0.1.1-evidence.json；无提交发布或Go消息 | @2026-09-07

🎯 EXPECT 仅修复/tmp 0.1.9候选分类切片及smoke诊断；公开Command回放先复现set-participation真实成功响应进入RECONCILING，再通过严格canonical/持久屏障回归与runner编译；正式版本/锁文件、普通消息与观测生产源不变，不运行live或操作Go DB | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE（FIXED_FOCUSED_REAL_RERUN_PENDING）Go 缺用户资料时保留空 displayName；公开Command红测重现该字段导致RECONCILING，候选wire仅修正资料下界并保留全部canonical/持久屏障。25分类生命周期+4runner离线测试与build通过，新增有界脱敏错误/输入形状诊断，准确新binary SHA与补丁见docs/category-native-profile-compat-report.md。新binary未实跑，无Go DB/忙消息/正式版本锁变更；原始失败保留 | @2026-09-07

🎯 EXPECT 只在/tmp 0.1.9候选新增参数化Native smoke harness及本仓说明；复用生产engine/HTTP/WS/SQLite/EventSink，11路由可执行生命周期和脱敏证据，离线编译/配置测试通过；不运行live、不改四包生产源码、不操作Go库或客户端 | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE（BUILD_READY_NOT_RUN）候选新增category-native-smoke：生产Native engine/HTTP/WS/SQLite/EventSink参数化装配、11路由工作流、关联HTTP+WS+持久读回脱敏证据、失败/cleanup记录。离线build与3个测试通过，仅执行--help；四包源码/manifest SHA未变、无live run目录、未连接服务或操作Go库。真实端点/隔离身份/WS保活由主任务协调Go owner提供，准确输入与命令见本地合同/harness README；未改客户端/正式Cargo/版本，无提交发布 | @2026-09-07


✅ DONE Helix 0.1.1 已发布：`cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 四包均经 crates.io 实时查询确认可解析；8 个 workspace manifest 与 Cargo.lock 已同步到同批依赖图。版本事务等待本次发布提交承载，不自动 push，既有 artifacts 保持未跟踪 | @2026-08-27T

🎯 EXPECT 修复 crates.io 一键发布的 workspace 版本事务；可证伪断言：① 四个公共 crate 仍是唯一上传集合；② Web、FFI、host-cli 与 helix-perf 只同步内部精确依赖，自身 package.version 保持不变；③ `--plan` 能在上传前发现任何未登记的 workspace path 精确依赖；④ 首次上传前失败按字节恢复 8 个 manifest 与 Cargo.lock；⑤ 单测、候选版本图复现、`--plan` 和 `git diff --check` 通过，不执行真实 publish、不修改既有 artifacts | 赌注 高 | 可逆 是 | @2026-08-27T

✅ DONE crates.io 一键发布版本事务已覆盖 4 个公共上传包和 Web、FFI、host-cli、helix-perf 4 个非上传消费者；自动扫描根/workspace manifests，遗漏或版本不一致时 `--plan` fail-closed，首次上传前回滚覆盖 8 个 manifest 与 Cargo.lock。发布脚本 7/7 回归、旧 core 精确引用 0、联网 `--plan`、8 包 `cargo check` 与 `git diff --check` 通过；当前 0.1.0 四包可解析且 0.1.1 四包未占用，未执行真实 publish。完整 Gate 的 check/wasm/clippy 通过，随后被未改动的 `f1_gap1_reconnect_read_channel_test.rs` 缺 `ImConfig.client_platform` 挡住，未越界修复 | @2026-08-27T

🎯 EXPECT 沉淀并实现 MessageV3 十二项核心业务指标；可证伪断言：① 新增 Spec 050 冻结 GoServer、Helix host、客户端 ACK 与 Prometheus recording rule 的 Owner/分母/终态，GitLab issue 标记 ready-for-agent；② Go 只在真实事务、Outbox 与每接收人入队终态计数，Helix 只在真实 MessageV3 Emit/同步恢复终态计数，绝不把 Socket 写成功冒充客户端 ACK；③ Prometheus/Grafana 只从低基数原子指标计算比率与 P50/P95/P99，标签不含 user/channel/post/corr/seq；④ 先写公开 registry/dispatch seam 红测再实现，Go 与 Helix 聚焦测试、JSON/PromQL 静态检查和 git diff --check 通过；⑤ 保留 Helix artifacts 与 Go perf/tmp/`=` 等既有未跟踪文件，不修改业务协议或伪造缺失终态 | 赌注 高 | 可逆 是 | @2026-08-24T20:04:22+0800

✅ DONE Spec 050 与 GitLab #127 已冻结十二项指标口径；Go 已在事务 commit、Outbox 生命周期和既有 WS per-connection seam 暴露原子指标，Helix host 已在稳定 Emit 与本地 Command lineage 暴露投影/同步/恢复/正确性/Command 终态；PrometheusRule、recording rules 和 12 面板中文 Grafana JSON 已生成。Go `go test ./internal/app ./internal/ws` 通过，Helix 89 个 lib 单测与 5 个 business metrics 测试通过；全包仅既有 `media_pipeline_e2e` 等待真实 SQLite sent 超时，未扩张修复。客户端真实 ACK 与同 scope 未读权威对账保持 Planned/无样本，未伪造 0% 或 100%；未运行真 Go/Native/Collector/Grafana 链，状态为 `REAL_EVIDENCE_NOT_RUN` | @2026-08-24T20:27:04+0800

🎯 EXPECT 全局搜索新私聊在 Helix 持久成功后无感跳转；可证伪断言：① `im_create_channel` 只返回 accepted，Angular 不把它当频道；② `im:channel:created` 在 PersistOk 后带同一 `requestId`；③ 前端只用该权威投影的非空频道 ID 缓存并切换；④ 已有私聊直接切换不受影响；验证 `dialogSwitch.service.test.ts` 与 `message_v3_g08a` | 赌注 高 | 可逆 是 | @2026-08-24T

🎯 EXPECT 用 Rust 小白可复述的方式梳理 `crates/helix-driver-native/tests/messageV3-domain/Phase1/Post/G01a/main.rs` 发送链；可证伪断言：① 课程明确 Cargo target→G01a→shared helper→Tick/Effect→HTTP/WS/SQLite/EventSink→cleanup 的每一跳；② 每个断言都标明能证明与不能证明什么；③ 明确当前未启动 Go/WS/Native，结论保持 `UNVERIFIED`，不把 evidence 中静态 `status=SUCCESS` 写入冒充 HTTP 回包；④ 只新增 teach 教学资产与 SESSION 预期，不修改 Helix 业务代码或既有 artifacts | 赌注 中 | 可逆 是 | @2026-08-21T

🎯 EXPECT Helix 消费原生 `TEXT_CHAIN` 与同作者接龙 Upsert；可证伪断言：① HTTP/WS Post 与 postUpdate 不再依赖 ANNOUNCEMENT 语义；② 相同 entry_id 的后续权威投影覆盖 SQLite 现有行而非追加；③ PersistOk 后才 Emit，客户端只消费权威绝对态；④ driver-native 聚焦合同与真实 18066 链路对账通过 | 赌注 高 | 可逆 是 | @2026-08-20T17:00:00+08:00

🎯 EXPECT 在 `helix-driver-native` 新增 Phase5/Chain/Common 与 Phase5/Chain/Text 的 8 个文字接龙 Gate 入口并注册到 `message_v3_domain`；可证伪断言：① `CHAIN-COM-G01..G06` 与 `CHAIN-TXT-G01..G02` 每个都有独立 `main.rs`，不存在仅用静态注册冒充通过；② 真实测试从公开 Command 入口验证 HTTP/WS、Helix projection、SQLite durable read-back、Native EventSink 和清理；③ `helix-driver-host` 不承载文字接龙业务 reducer；④ 本地 SQLite 若缺 chain 表，只补最小 schema/migration；⑤ Go 通过 `make docker-up-clean` 运行在 18066，所有缺失真实前置明确写 `BLOCKED_PRECONDITION`/`NOT_VERIFIED`，不伪造 PASS | 赌注 高 | 可逆 是 | @2026-08-19T

## EXPECT（2026-07-24：G02/G04/G05/G06/G07/G08/G10/G13/G14/G15 FrameV2 全 GN 文档）

- 产出根级字段与验收总索引，以及 10 个 GN 各自的字段类图、主链路时序、Frame 接受值、字段级合同、三端验收合同与 handoff。
- 以当前 Helix 严格矩阵、源码聚焦测试、cses-client WDIO 现状和 Mobile QuickJS/GTest 真源为依据；明确 `PASS / PARTIAL / BLOCKED / CONTRACT_DRIFT`，不把 Mock 或 accepted 冒充真实链路。
- 仅沉淀文档合同与生成/校验能力，不实现各端业务代码；G01 仅作为参考且不纳入本轮新增范围。
- 验证：生成器语法与幂等、索引校验、Mermaid 可解析性、链接/路径、`git diff --check`。

## DONE（2026-07-24：G02/G04/G05/G06/G07/G08/G10/G13/G14/G15 FrameV2 全 GN 文档）

- 已新增根级字段与证据总索引，并为 10 个 GN 各生成 7 份深挖文档；每个 GN 连同原有 8 份基础页共 15 份。
- 已固化 FrameV2 envelope、SurfaceMutation、typed VM、canonical command、HTTP/WS、Helix producer、WDIO mock/real/dual、日志/断言/report、Mobile GN GTest 与 Venus 接码合同。
- 已显式标记 G05、G07、G08、G13、G14、G15 的 Matrix/source 或 Surface/producer 漂移，所有未重跑的三端真实链保持 `BLOCKED/unknown`。
- 生成器语法、Python 编译、19 UC/25 Gate 索引与本地链接、生成幂等、10 个 Mobile 现有 GTest 路径、70 份 selected-GN Mermaid 渲染和 `git diff --check` 通过；本轮是文档合同沉淀，未运行 Cargo、WDIO 或 Mobile runtime 测试。

## 0. 当前任务

✅ DONE G04a-e 本批回执修复：创建/取消以 pending request 匹配并单次消费；PersistAtomic 成败均释放关联，WS 重放恢复与迟到 HTTP 不重复结束请求；getSchedule 显式业务失败和坏 JSON 返回关联错误。聚焦 105/105、helix-im check（8.56s）、diff check 通过；新增 6 项回归先红后绿（5 项复现，HTTP 失败矩阵原已通过）。 | @2026-09-09T16:52:31+08:00

生命周期核对：创建/取消 HTTP 2xx + SUCCESS 只 admission；当前 viewer 的 WS authority 进入 PersistAtomic；失败只返回 SCHEDULE_PERSIST_FAILED，无 readback/领域成功；成功返回单次 read relay，随后 Get durable row 才发 schedule-created/canceled。G04c 仍由取消/创建两命令组成；G04e 仍由普通发送/取消组成；G04d 无 WS/Persist 阶段，HTTP 读回保留 req_id。上传失败、120 秒上传超时与 stop/迟到回包由既有聚焦测试覆盖。
本轮证据只到 sans-IO public seam；未重跑真实 Go/WS、Native SQLite、UI 或全量 Gate。热路径 schedule handler 脚本通过；通用 dispatch 的 1 项、dispatch_read 的 5 项 clone 告警在 HEAD 原文复测一致，本批未新增。原 artifacts/output 不纳入提交，不 push。
既有并发风险（不计本批已修）：schedule committed revision 直接 insert 及无版本 readback context 均早于 79aa3476（1124664e 等历史实现）；反序 PersistOk 可能回退内存水位，多个 Get 可能重复 Emit 同版本。独立静态审查发现，未运行破坏性回放或验证 driver FIFO，按本批范围保留后续核验。


🎯 EXPECT 收口 G04a-e：仅修定时请求回执生命周期；HTTP admission 不提前成功，PersistAtomic 失败清理关联，WS 重放与迟到 HTTP 不重复结束请求；G04a-e、G08/G09、read_relay 聚焦测试与 helix-im check、diff check 通过。基线 44914c4a，分支 reactor/MessageV3-helix，保留既有 artifacts/output，不 push。 | 赌注 中 | 可逆 是 | @2026-09-09T16:49:46+08:00

🎯 EXPECT 本轮 Markdown 日检仅对齐已有 CI/脚本事实；可证伪断言：只修改 README、scripts/README、Harness OPERATING/decisions 与 SESSION，诊断入口、6m/5m/180000ms 超时层级及 PRE 凭据覆盖顺序均能在当前配置找到；923 份 Markdown 的相对路径链接与聚焦 checker 通过，不修改代码/配置/生成物 | 赌注 低 | 可逆 是 | @2026-09-08T08:04:17+0800

本轮隔离：`codex/helix-markdown-20260908-0801`，upstream `origin/reactor/MessageV3-helix`，基线 `68dbf21c`；worktree `/System/Volumes/Data/workspace/angular/visual/helix-markdown-20260908-0801`，初始 clean，保留供追溯、不自动删除。源 checkout `3d421dbb` 的四组未跟踪资产不纳入。

清单/真源映射：923 份 tracked Markdown；根 README/AGENTS/GOAL/ROADMAP → Cargo workspace、Spec047 manifest 与当前 CI；crate 文档 → 各 crate manifest/src/tests；API/脚本说明 → contracts/schema 与 scripts；Harness → HX-C 文件/索引/checker；历史 Spec/Frame 报告仅核对定位与链接。上轮 `9bcba4dc` 后非 Markdown 变化只有 `.gitlab-ci.yml` 和两个 MessageV3 runner/诊断脚本，本轮语义核对重点为这些变化，未逐条重证全部既有 API/运行态声明。

✅ DONE 本轮 5 份 Markdown 已对齐 CI 诊断分支、6m/5m/180000ms 限时、实时日志与 PRE 凭据顺序；删除强制 File/WASM 前置的失效描述。923 份 Markdown 的 1599 条相对路径链接、HX-C 18/18/18、8 个 workspace member/四公共 README 0.1.9、83 Gate 只读测试 3/3、Spec047 默认 checker、Strict Matrix、InteractionFrame 历史索引、CI/凭据源码断言与 gate-fast 均退出 0。Spec047 当前产物仍为 NOT_GENERATED；未重跑既有生成物/跨仓 agent 检查，未执行 Cargo、PRE 登录或真实业务链。已具备本轮文档提交条件，push 结果由自动化报告记录 | @2026-09-08T08:05:24+0800


🎯 EXPECT 每日 Markdown 熵管理只修改能由当前 `1355772d` 代码、Cargo manifest、配置、测试或脚本证明漂移的 Markdown 与必要 SESSION 记录；可证伪断言：① 不修改 Rust、JSON、TOML、脚本、生成产物或共享 checkout 未跟踪资产；② README/AGENTS/GOAL/ROADMAP/Spec/contract/harness/链接与路径结论均可回溯到当前真源；③ 直接相关链接、索引、checker 与 `git diff --check` 通过；④ 既有生成 hash、Tag 或代码问题只报告不越界修复；⑤ 有真实 Drift 才创建结构化中文 commit 并普通推送，否则报告 `NO_CHANGES` | 赌注 中 | 可逆 是 | @2026-09-06T08:02:30+0800

✅ DONE 本轮在独立 worktree 仅更新 Markdown/Mermaid 与本条 SESSION：对齐 core 7 Tick/11 Effect/9 StorageOp/9 ports、host scoped storage 与 UploadFile/3 秒 drain、native host-pump 路由、host-cli 8066 YAML 配置；修正 MessageV3 `Effect::Emit { event: DomainEventBytes }`（bytes `{event,data}`）及 PersistOk 后 sending 语义；为旧 InteractionFrame/Frame projection、UC automation、Phase1-4/Phase5 交接补 `RETIRED_HISTORICAL` 边界；补 HX-C016 YAML 元数据。验证：`git diff --check` PASS；相对 Markdown 链接 1596 条、broken=0（另有 10 条绝对路径引用）；cutover checker default/self-test PASS；strict-matrix default PASS；18/18 harness metadata PASS；Cargo metadata 四包均 0.1.9；未修改 Rust/JSON/TOML/脚本/生成物或共享 checkout 资产 | @2026-09-06T08:50:51+0800

⚡ DRIFT-PRED 当前 checkout 的 `specs/047-im-event-gate-inventory/.v1/gates` 未生成，`--require-generated` 仍失败；`scripts/strict-matrix/check.mjs --check-generated` 仍报告既有生成矩阵漂移；跨仓 docs/mmd 与旧 InteractionFrame 目录仍保留未逐文件清理。它们均属本轮明确不生成/不扩张的边界，后续需绑定当前 revision 的独立收敛任务 | @2026-09-06T08:50:51+0800

🎯 EXPECT 每日 Markdown 熵管理只修改有当前代码、manifest、配置、测试或脚本证据的现行文档；可证伪断言：① 仅更新 core/host/native 当前设计图与说明及必要 SESSION，不修改 Rust、JSON、TOML、脚本、生成产物或共享 checkout 未跟踪资产；② Tick、Storage、engine route、worker 并发、FFI query 与 reconnect 文档与 `b8868f4f` 基线源码一致；③ 现行 Markdown 相对链接、Gate/Spec/harness 索引和 `git diff --check` 通过；④ `.v1`、generated hash、历史 allowlist 等非 Markdown owner Drift 只报告；⑤ 有真实 Drift 才创建结构化中文 commit 并普通非强制推送，否则报告 `NO_CHANGES` | 赌注 中 | 可逆 是 | @2026-09-07T08:02:26+0800

✅ DONE 本轮在独立 worktree 仅更新 17 份 Markdown/Mermaid/SESSION（90 行新增、62 行删除）：对齐 7 Tick、10 Storage 方法、PortProgress/one-shot timer、PersistAtomic/UploadFile correlation、EngineDeps/transport registration/fairness、HostStorage 单 writer + 可选 ReadPool、`max_http_inflight` worker、`helix_query` 元信息边界与 ReconnectPolicy。验证：10/10 Mermaid PASS；Markdown 本地链接 1596 条、broken=0；InteractionFrame index PASS；Spec047 30 Gate default/self-test PASS；Strict Matrix 50/54/epoch13 PASS；harness 索引 18/18；Cargo metadata 四包 0.1.9；source assertions PASS；`git diff --check` PASS。未修改 Rust、JSON、TOML、脚本、生成产物或共享 checkout 未跟踪资产 | @2026-09-07T08:02:26+0800

⚡ DRIFT-PRED 初始审计范围为当前 design/ 文档，复核后发现同一运行时事实也落在 native AGENTS 与 host 并发规范，故扩展到两个现行规则/规范 Markdown；候选教训：文档真源审计必须按同一运行时事实的所有现行入口收敛，而不能只按 design/ 目录筛选 | @2026-09-07T08:02:26+0800

🎯 EXPECT 每日 Markdown 熵管理只修改有当前代码、manifest、配置、测试或脚本证据的文档；可证伪断言：① 只在本轮独立 worktree 产生 Markdown/必要 SESSION 改动，不触碰 Rust、JSON、TOML、脚本、生成产物或共享 checkout 未跟踪资产；② 远端 `origin/reactor/MessageV3-helix` 基线固定为 `0ef1692890670e3e3e8f14f455c771d377cbef3a`，所有版本、路径、Gate/Spec/索引结论均可回溯到该 tip 或当前真源；③ 直接相关链接、路径、索引、checker 与 `git diff --check` 通过；④ 若发现既有 checker/Tag/生成物 Drift，仅报告不越界修复；⑤ 有真实 Drift 才提交并普通非强制推送，否则报告 `NO_CHANGES` | 赌注 中 | 可逆 是 | @2026-09-05T08:01:00+0800

✅ DONE 本轮仅更新 Markdown：四个公共 crate README、docs/README 与 ROADMAP 对齐 `0.1.9` 和 `0ef16928`；根索引、Spec 047、历史 Strict Matrix/Frame/full-map/fixture/curl 文档已明确 `RETIRED_HISTORICAL`、30 Gate 当前 manifest、epoch 13 历史分母和真实 producer 路径。验证：Markdown 链接 1605/1605、cutover checker 30 Gate PASS、manifest test path 30/30、Strict Matrix retired epoch 13 为 50（47 active/3 excluded）、Cargo metadata 四包均 0.1.9、`git diff --check` PASS；未改 Rust/JSON/TOML/脚本/生成物或共享 checkout | @2026-09-05T08:55:14+0800

⚡ DRIFT-PRED 严格 Matrix `--check-generated` 仍因既有 generated hash 漂移失败，`helix-v-0.1.9` 本地与远端均缺失；两项均按预期保留为发布/生成流程债务，未用文档改动掩盖 | @2026-09-05T08:55:14+0800

🎯 EXPECT 每日 Markdown 熵管理只修改有当前代码、Cargo manifest、脚本或测试证据的文档；可证伪断言：① 四个公开 crate README、docs 索引与 ROADMAP 都精确对齐 `0.1.7`；② `docs/README.md` 与 `docs/ROADMAP.md` 的代码快照更新至本轮独立 worktree 基线 `088184df`；③ 不修改 Rust、JSON、TOML、脚本或生成产物；④ 直接相关链接、版本、索引与 `git diff --check` 均通过；⑤ 缺失 `helix-v-0.1.7` 注释 Tag 只报告为发布流程 Drift，不以文档掩盖 | 赌注 低 | 可逆 是 | @2026-09-04T08:00:55+0800

✅ DONE 每日 Markdown 熵管理已将四个公共 crate README、docs 索引与 ROADMAP 对齐当前 `0.1.7` manifest/registry 事实，并更新代码快照至 `088184df`；本轮 7 个文件只含 Markdown，链接 1599/1599、Cargo metadata、四个 `cargo search` 版本、旧 `0.1.4` 文本扫描与 `git diff --check` 通过；`helix-v-0.1.7` 仍缺失，作为发布流程 Drift 单列，不以文档掩盖 | @2026-09-04T08:17:07+0800

🎯 EXPECT 收敛已发布的 Helix `0.1.5` 版本事务并沉淀发布后提交/Tag 铁律；可证伪断言：① 四个公开 crate 与四个内部消费者 manifest、`Cargo.lock` 全部保持同一 `0.1.5` 依赖图；② 四个 crates.io 包均能被 `cargo search` 解析为 `0.1.5`；③ 根 `AGENTS.md` 明确发布后必须提交版本号并创建注释 Tag；④ 只提交本次发布版本改动、规则和必要 SESSION 记录，保留既有未跟踪 `artifacts/` 与 Cargo 私仓 Prompt；⑤ `git diff --check`、发布脚本回归和 metadata 解析通过，创建 `helix-v-0.1.5` 且不覆盖已有 `helix-v0.1.5` | 赌注 中 | 可逆 Tag 前是、Tag 后否 | @2026-09-03T

✅ DONE Helix `0.1.5` crates.io 版本事务已完成核验：四个公开包均由 `cargo search` 解析为 `0.1.5`，8 个 workspace manifest 与 `Cargo.lock` 保持同批依赖图；发布脚本回归 7/7、metadata、三端 Rust check、core 16/16、Web WASM build 与 HX-C001 零运行时检查通过。既有未跟踪 artifacts/私仓 Prompt 未纳入；本次发布提交与 `helix-v-0.1.5` 注释 Tag 随任务落地 | @2026-09-03T

🎯 EXPECT 在独立 worktree 对齐当前仓库 Markdown 与 `512a0aeb` 代码事实；可证伪断言：① 只修改能由当前目录、Cargo/API/config/source、测试或脚本证明漂移的 Markdown 和必要 SESSION 记录，不改 Rust、JSON、TOML、脚本或生成产物；② 全库现行 Markdown 建立清单与代码真源映射，历史文档只核对定位、链接和退出现行入口边界；③ 保留共享 checkout 的未跟踪 artifacts/Prompt，不复用或清理其他 worktree；④ 聚焦 checker、相对链接/路径/索引、`git diff --check` 全部通过后才提交并普通推送目标分支 | 赌注 中 | 可逆 是 | @2026-08-29T08:05:00+0800

✅ DONE 已在独立 worktree 对齐 14 份现行/历史 Markdown：harness 母本三处旧机器路径改为当前真实 sibling 文件；FFI 局部入口移除两份已删除阶段文档并把 query 契约路由到现行 AGENTS/source；六份 EventBus 方案与两份 V2/跨仓快照明确标为 `RETIRED_HISTORICAL`，撤销已删除过会稿、Mobile 文件和临时 worktree 的失效链接。全库 913 份 Markdown、1573 个本地链接 broken=0，HX-C 18/18/18，Spec 047 checker PASS、30/30 test path 与 production owner 存在，四个公共 crate README 版本与 0.1.1 manifest 一致，纯 Markdown diff 与 `git diff --check` PASS；未运行 Cargo。Spec manifest 仍有 10 Gate 的空 `verification` 命令/路径，属于本轮禁止修改的 JSON/checker Drift，已单列未伪造 Green | @2026-08-29T08:25:18+0800

⚡ DRIFT-PRED 现有 Spec 047 checker 在 10 个 Gate 的 `verification` 含空 `--test` 与 `.v1/gates//generated` 时仍返回 PASS，且审计脚本先后暴露 zsh 只读变量与 Git 中文路径转义误判 → 候选教训：文档日检需独立校验 manifest 语义非空，并固定 `core.quotePath=false` 与跨 shell 安全变量名 | @2026-08-29T08:25:18+0800

🎯 EXPECT 对齐当前仓库 Markdown 与 `caf20f3b` 代码事实；可证伪断言：① 只修改已存在且能由当前目录、Cargo/API/config/source 证明漂移的 Markdown，不改生产代码、不新造功能；② 删除或更正已不存在的模块、接口、配置与流程，并补齐已实现但入口文档缺失的重要事实；③ 保留既有文档结构和表达方式，已有未跟踪 artifacts/Prompt 不纳入；④ 全库 Markdown 相对链接、路径/符号抽查、`git diff --check` 通过，最终逐文档列出修改与主要 Drift | 赌注 中 | 可逆 是 | @2026-08-28T

✅ DONE 已按 `caf20f3b` 当前 workspace、Cargo manifest、源码目录、Spec 047 manifest/verification 和现存脚本对齐 13 份活跃 Markdown：删除失效的架构索引/过会方案/night-loop 路由，更正 MessageV3 P0-P7/R4 完成态、TGCR/30 Gate 当前真源、Web A5 与产品 renderer/上传边界、crate 0.1.1 版本、IM/driver/脚本目录及 HX-C008 命令。活跃入口 51 文件相对链接 broken=0，scripts README 22 个入口 missing=0，HX-C 18/18/18 对账，Spec 047 checker PASS，Web adapter/recorder 10/10，四个公共 crate README 与 manifest 一致，`git diff --check` PASS。额外运行的发布脚本单测因仍硬编码 0.1.0→0.1.1 而 5/7 通过、2 个 error，属于既有测试代码漂移，本次文档任务未越界修改；用户未跟踪 artifacts/Prompt 保持不变 | @2026-08-28T

🎯 EXPECT 修复推送门同根因的 ImConfig 测试配置遗漏：只在 FFI egress、Host media、IM reconnect 三个测试补齐 Native 平台配置与邻近职责注释，不改生产逻辑/断言；三个聚焦 target 编译通过，运行结果如实记录，任何其他根因失败不扩修；随后继续正常 hook 推送。 | 赌注 低 | 可逆 是 | @2026-09-09

✅ DONE 原 stress_test 配置修复已在 `59ca6ab0` 提交，check及3/3通过；剩余三个同根因配置已补齐，聚焦target cargo check通过（25.88s），IM reconnect 6/6通过并独立提交。FFI parity 2/4（事件/生命周期断言失败）、Host media 0/1（SQLite sent等待10秒超时）属于不同于缺字段编译错误的后续阻塞；两文件保留工作区、不提交、不扩修生产逻辑。推送仍未成功。日志 `/tmp/helix-platform-fixtures-{check,test}.log`。 | @2026-09-09
⚡ DRIFT-PRED 全部目标通过编译后，FFI事件断言与Host媒体终态仍失败 → 候选教训：配置编译修复只能证明target恢复可执行，不能替代测试行为验收；按publish-only边界停止扩张。

🎯 EXPECT 按用户授权提交工作区与暂存区的分类接龙改动；仅补齐当前必填 maxCategories 的库/Host 测试输入，聚焦 IM 分类与 SQLite 守卫测试及五 crate check 通过；生产业务语义、历史 patch 字节和 tmp 备份不变，按功能与证据分类提交，不 push，不扩修客户端或无关历史失败。分支 helix-message-test，开局 HEAD caf20f3b，无 upstream，保留分支。 | 赌注 中 | 可逆 是 | @2026-09-09

✅ DONE 代码批次 `1ed15f08` 已提交：IM lib 371/371、分类39/39、Host分类SQLite6/6、存储一致性21/21、守卫3/3及五crate check通过。只补当前合同的旧测试输入并刷新canonical来源哈希；生产语义不变。合同/审查资料单独归档，明确实际客户端准入BLOCKED、真实恢复未验；历史patch原字节和tmp备份保留，补丁数据的空白上下文由docs/.gitattributes精确豁免行尾空格，diff check与JSON/patch解析通过。未push。 | @2026-09-09

🎯 EXPECT 分类接龙独立 review 只追加 SESSION 与 docs/reviews/2026-09-09-category-chain-review.md 及测试证据；核对正式 0.1.1 与客户端实际 0.1.9 的 11 命令、持久屏障和恢复路径，执行现有专项与邻接测试；不改业务源码、不提交、不运行真实账号链，断线漏事件恢复未实测则不得报 PASS。run_id=category-review-20260909 | 赌注 中 | 可逆 是 | @2026-09-09

✅ DONE 独立 review 完成，客户端准入 BLOCKED：实际 0.1.9 复现旧 Post 覆盖更高 revision、同参与版本返回旧权限、历史 CONFIRMED reconcile 降为 RECONCILING、旧 fact 路径截断；测试基线缺 required maxCategories/fixture。正式专项39/39、恢复与若干邻接通过，但库/Host旧样例及普通echo有失败；实际专项31/36（5缺fixture）。完整 SESSION_REPORT/v1 见 docs/reviews/2026-09-09-category-chain-review.md，原始测试和只读依赖probe在/tmp/category-review-20260909。701份生产Rust源SHA保持一致，无业务/仓内测试改动，无提交发布、真实账号链和漏事件恢复验收。 | @2026-09-09


🎯 EXPECT 在/tmp从checksum匹配的0.1.9四个发布归档构建不可发布候选，迁入分类接龙delta并保留四个观测API；分类专项与Native兼容检查通过，正式源码/版本/锁文件不改；Web/FFI若缺归档则精确标明，不伪造同版来源 | 赌注 中 | 可逆 是 | @2026-09-07

✅ DONE（Native候选可验证；Web基线缺失）/tmp/helix-category-0.1.9-candidate-kortuqqf由四个checksum匹配归档组装，保留0.1.9观测API与Post/WS新基线，只迁分类delta；workspace check、22分类+4SQLite+2回滚+1观测API测试通过，依赖图统一0.1.9、5份private fixture逐字一致。Web/FFI未上传源码不能由四包恢复，保留UNAPPLIED Web guard补丁并显式NOT_RUN。客户端patch配置/准确命令/来源哈希已交本地合同，正式生产代码/Cargo/版本未改，无提交发布 | @2026-09-07
⚡ DRIFT-PRED 0.1.9本人Post终态为im:post:sent，旧分类publish测试的received预期在候选上失败 → 保留发布生产行为，仅按真实新基线修订候选测试；发布归档不包含Web/FFI源，不能把四包Native验收误报为三端完整验收。证据见候选category-precompat.log与assembly.json | @2026-09-07


🎯 EXPECT 分类接龙私有operation补齐请求捕获的channelId/viewerId；只改runtime、现有生命周期测试、生成fixture及本地交接文档；create无chainId/publish/reconcile/fail实际Emit覆盖，WS和read CommonRes字段不变，聚焦测试与cargo check通过，不改客户端/版本/提交 | 赌注 低 | 可逆 是 | @2026-09-07

✅ DONE 分类接龙私有operation仅在durable complete阶段增加Request.channel/viewer；fail私有错误也带viewerId，read CommonRes/WS字段不变。首次create红测因channelId为Null失败；22/22聚焦测试和cargo check -p helix-im通过，5个实际Emit JSON与SHA256 provenance已写docs/fixtures/category-chain-operation。完整Go receipt扁平保真、reconcile查询原形与WS原形均断言通过；临时0.1.1 path接入=0.1.9客户端的方法写入本地合同，未执行客户端构建或修改正式Cargo文件；未提交/推送/提版 | @2026-09-07


🎯 EXPECT 分类接龙独立接入 helix-im Command→HTTP/WS→PersistAtomic/read-back→EventSink；版本十进制字符串不降精度，私有本人状态不进入共享卡片，TEXT_CHAIN/普通发送聚焦回归保持通过；只改本仓、不提交/推送/改版本；focused/SQLite/真实Go/UI证据分层记录 | 赌注 高 | 可逆 是 | @2026-09-07

✅ DONE（Helix本地切片；整体PARTIAL）分类接龙11命令及HTTP/WS→PersistAtomic/read-back→EventSink已完成；focused 414 passed、SQLite相关43 passed（最终分类链+guard 6项复验通过）、core 23 passed/1 ignored、Web37 passed，五crate check/wasm build/git diff --check通过。Host全量受既有media_pipeline_e2e缺client_platform与上传测试sandbox bind限制；真Go/Native运行态/双用户/UI均NOT_RUN，服务端仍未部署/runtimeSupported=false。保持reactor/MessageV3-helix@caf20f3b，不提交/推送/改版本。合同与SESSION_REPORT/v1见docs/category-chain-client-contract.md | @2026-09-07
⚡ DRIFT-PRED 普通CanonicalPost流与分类head可不同步，红测复现旧HTTP覆盖新message及旧Card-only WS倒退投影 → 新增通用UpsertSpec.update_guard并由Host/Web兑现原子回滚，同时在写前和最终读回核对message版本；外部字面量需补None。候选教训：独立投影的head不能替代共享消息事实的版本证据。约定的外部/workspace/java/logs路径在本机不存在，本轮证据保留于本仓交接文档与/tmp日志 | @2026-09-07


🎯 EXPECT 将 hello 后 `channels/load/increment` 的 timestamp 改为 tenant-local `message.update_at` 最大值；可证伪断言：① hello 先产生 `Scan(message, limit=1, ORDER BY update_at DESC)`，scan 回包后 HTTP body 的 `timestamp` 等于首行 `update_at`，空/失败回包为 0；② 现有 `cursors[{channelId,fromSeq}]` 内容与顺序保持不变；③ 启动投影未完成时的既有 bootstrap retry 仍只在 `finish_startup_scan` 触发，且也重新读取 message watermark；④ 只改 Helix increment/相关 correlation 与 focused tests，不改 Go、客户端或既有 `artifacts/`；⑤ 聚焦 Rust 测试与 `git diff --check` 通过 | 赌注 高 | 可逆 是 | @2026-08-20T

🎯 EXPECT 新增仓内 `$messagev3-cross-layer-coding` Skill，专门把 MessageV3 跨层需求从契约冻结推进到 TDD、逐层实现、真实链和分层交付；可证伪断言：① 新目录含合法 `SKILL.md` 与 `agents/openai.yaml`，并在 `.agents/README.md` 注册；② 明确 Angular 只提交 intent/消费绝对 projection，Helix 拥有业务事实与 PersistOk 屏障，driver 只做 I/O/批量交付，Go wire/远端持久化责任可追溯；③ 固化红→绿、字段 ledger、失败矩阵、跨仓文件租约、REAL_NATIVE_CHAIN 与 run-owned cleanup，不允许 fallback/HTTP 200/单点 EventSink 冒充完成；④ `quick_validate.py`、YAML、路径、无尾随空白和 `git diff --check` 通过；⑤ 不修改现有排查 Skill、业务代码或 `artifacts/` | 赌注 中 | 可逆 是 | @2026-08-20T

✅ DONE 通用 `$messagev3-cross-layer-coding` 已落地并提交 `9e6bc60e`：覆盖任意 Gate 的 Projection/Emit 编码、TDD、owner 边界、真实链八步、失败分类和清理；不绑定 `post_read`，只保留 read/receipt 作为示例。Skill 校验、YAML、引用路径、无尾随空白和 staged diff check 通过；未运行业务 Cargo/Go/Angular 测试，当前 G06b 源码、其余 SESSION 记录和 artifacts 继续保留为其他任务 dirty。

🎯 EXPECT 将 G06b `post_read` 从作者侧四字段 delta 收敛为 Go 权威、Helix 完整处理、Angular 仅消费绝对 receipt projection；可证伪断言：① Go 作者定向 WS 帧包含 `snapshotId/authorUserId/readerUserId/memberIds/readMap` 且不伪造绝对 projection；② Helix 只在 matching PersistOk 后 Emit `im:post:readers`，并输出 `readUserIds/unreadUserIds/readCount/unreadCount/allRead`；③ Angular 不再根据 `readerUserId` 做差集、刷新或缺基线兜底；④ Go/Helix/Angular focused seam 均有失败→通过证据；⑤ 不触碰既有 `artifacts/`、客户端脏改和无关分支；赌注 高 | 可逆 是 | @2026-08-20T

🎯 EXPECT 新增仓内 `$helix-command-driver-host-test` Skill，让其他 Command 可复用 G01a 的 driver-host/Native 测试装配；可证伪断言：① Skill frontmatter、`agents/openai.yaml`、引用矩阵和 `.agents/README.md` 入口齐全；② 明确 TDD 红测、共享 adapter、真实链八步、FOCUSED/LOCAL_INTEGRATION/REAL_NATIVE_CHAIN/UI_DOM 分层、失败分类和 run-owned cleanup；③ 不复制业务实现、HTTP/WS/SQLite harness，不把 HTTP 200/health/EventSink 单点冒充 real-chain PASS；④ `quick_validate.py` 与 `git diff --check` 通过，既有 `artifacts/` 脏目录保持不变 | 赌注 中 | 可逆 是 | @2026-08-19T

✅ DONE `$helix-command-driver-host-test` 已落地到 `.agents/skills/`，包含中文工作流、`agents/openai.yaml`、Command→driver-host adapter/证据矩阵，并在 `.agents/README.md` 注册；覆盖 TDD 红测、G01a 八步、真实 HTTP/WS/SQLite/EventSink 边界、失败分类、幂等和 cleanup。`quick_validate.py` 通过临时 `uv --with pyyaml` 环境验证，Ruby YAML、无尾随空白和 `git diff --check` 通过；未修改业务代码或既有 `artifacts/`。系统 Python 缺少 PyYAML，已保留该环境限制 | @2026-08-19T

🎯 EXPECT 将新增 Command 的 TDD 前置与完整生命周期固化到根 `AGENTS.md` 和 `crates/helix-driver-native/tests/AGENTS.md`；可证伪断言：① 每个新 Command 明确入口、分类、成功/失败终态、corr/req_id/pending/timer 清理和证据等级；② Native 测试规则固定 G01a 八步并要求 focused red→green、真实 HTTP/WS（适用时）、本地持久化/read-back、terminal EventSink 与 cleanup；③ 不修改业务实现、不把 HTTP 200/health/mock/replay 冒充 PASS；④ 两份 AGENTS 链接、章节、`git diff --check` 通过 | 赌注 中 | 可逆 是 | @2026-08-19T

✅ DONE 根 `AGENTS.md` 已增加每个 Command 的 TDD 前置、成功/失败/幂等/清理最低集合；新增 Native tests `AGENTS.md` 固化 G01a 八步、命令分类、真实边界、证据字段和反模式；`git diff --check`、章节/路径检查通过，未修改业务代码、未运行 Cargo，既有 `update_channel.rs` 与 `artifacts/` 脏改动保持原样 | @2026-08-19T

🎯 EXPECT 完成 `IMAGE → RICH` 新写硬切并接通 PC/Mobile：可证伪断言：① Helix/Go 接受旧入站 `IMAGE` 时只规范化为 `RICH`，任何新 DB/WS/发送事实不再产出 `IMAGE`；② PC 图片与图片+视频选择合并为单条 `RICH props.files`，渲染端按文件 MIME 在同一网格显示图片或视频，同时保留历史 `IMAGE` 只读渲染；③ Mobile FFI 的 rich 回执只接受正式 `RICH`；④ 不改 TEMPLATE/IMAGE 子类型、不做数据库 DDL/历史行迁移、不改 Cloud-eye 通用历史索引语义；⑤ 聚焦测试、构建、diff check 与 Helix 热路径审计通过 | 赌注 高 | 可逆 是 | @2026-08-14T12:00:00+08:00

✅ DONE `IMAGE → RICH` 新写硬切已完成：Helix 在发送构造和上传规划入口把旧 IMAGE 归一为 RICH，并让同一 `props.files` 按顺序承载图片/视频；Go 在 producer 与 consumer 两个入口归一积压 IMAGE，正式 Ent 枚举只保留 RICH；PC 选择层、上传、投影、渲染、摘要与 E2E G01c/G03c 合同统一 RICH，历史 IMAGE 仅保留读取渲染；Mobile QuickJS/C++ 改用 `RICH props.files` 并按正式 RICH 回执结算。Helix 聚焦用例与 check、Go 相关包测试/build/vet、PC 聚焦 Jest/66 条 WDIO 合同/build、Mobile 3 条聚焦 GTest、Cloud-eye PostEntryTest 和五仓 diff check 通过；未执行数据库 DDL、历史行迁移、真实设备或真实 HTTP/WS/DB 全链 | @2026-08-14T18:45:00+0800
⚡ DRIFT-PRED PC 全量参考样本生成器与已收敛样本存在无关漂移，已只保留 G01c/G03c 的 RICH 字段变化；Helix 热路径脚本在 `topic_activity.rs` 报出的两处多 clone 与 HEAD 基线逐字一致，本次新增分支未引入分配 → 候选教训：生成器全量重写和全文件静态审计都要先与当前基线对照，再决定任务边界 | @2026-08-14T18:45:00+0800

🎯 EXPECT 将正式 `RICH` wire 类型接入 Helix/Go/Cloud-eye，且暂不修改 PC/Mobile；可证伪断言：① Helix 不再把 `rich` 规范化为 `IMAGE`，同一 `props.files` 内 image/video 分别使用 `picture/attachment` bucket，全部完成前零 `posts/create`；② 只扩展 helix-im 媒体规划/重试/合同与测试，不改 helix-core/driver；③ Go `post/post_approval/post_schedule` 三套 PostType 接受 `RICH` 并保持 props 原样透传；④ Cloud-eye 明确将 `RICH` 投影为 `rich + image + video` contentTypes；⑤ 聚焦验证和热路径审计通过，PC/Mobile 保持未改与未验证 | 赌注 高 | 可逆 是 | @2026-08-14T11:17:59+08:00

✅ DONE `RICH` 已在 Helix 正式规范化并支持同一 `props.files` 的 image/video 混合上传：图片使用 `picture`、视频使用 `attachment`，旧 `IMAGE` 仍为图片专用；durable journal、重试目标和完成态 bucket 校验同步支持视频。Go 已新增 `PostTypeRich`，再生 Post/PostApproval/PostSchedule 的 Ent validator/schema，并以 API 测试证明 `RICH` 混合 props 到 SenderTopic 原序原字段透传；Cloud-eye 既有通用投影由新增回归测试证明输出 `rich,image,video`。Helix 聚焦测试、Go 聚焦测试/build/vet、Cloud-eye PostEntryTest 与三仓 diff check 通过；PC/Mobile、真实数据库约束、真实上传/WS/搜索运行态均未执行 | @2026-08-14T11:34:15+0800
⚡ DRIFT-PRED Cloud-eye 生产代码原本已通过未知类型降级和 `props.files` MIME 扫描支持 RICH，因此最小正确改动只有回归测试；Helix 审计脚本同时报告 `retry_send.rs`、`durable.rs`、`request.rs` 的既有多 clone 债务，本次新增规划/类型代码无新 clone，未越界重构旧热路径 → 候选教训：全文件热路径审计应区分本次 diff 与既有基线，否则兼容枚举扩展会被无关债务误判阻断 | @2026-08-14T11:34:15+0800

🎯 EXPECT 客户端 Helix native composition root 统一接入可选 OTLP Trace 与 Metrics；缺少环境变量时保持 Noop 且 `yarn start` 不因 Collector 不可达失败；聚焦 `cses-client/src-tauri/src/features/helix_im/{mod,runtime,observability}.rs` 与 `.env.example`，Helix core 不新增运行时依赖 | 赌注 高 | 可逆 是 | @2026-08-12T

✅ DONE 客户端已在 `helix_im::runtime::spawn` 复用 Helix host 的 `HostOtelRuntime`、`TraceHooks`、`BoundedMetricRuntime` 和共享有界 `AsyncMetricSink`，覆盖 HTTP/WS/Storage/Event/Engine；debug `yarn start` 无变量时使用 Host 默认 Collector，release 无变量或显式关闭时 Noop。`cargo check --manifest-path src-tauri/Cargo.toml --lib`、可选布尔值聚焦单测、`tauri-dev-with-portable-helix.mjs --check`、`git diff --check` 通过；新 observability 文件热路径审计通过，桌面真实启动与 Collector 收包未在本轮执行，仍需真实运行态确认。

🎯 EXPECT 在 `feat/MessageV3-helix-trace` 完成 Helix Trace + Metrics 逐跳归因体系；可证伪断言：① `helix-core` 零 OTel/metrics/runtime 依赖，所有热路径只通过 driver-host 有界 `try_record` O(1) 上报；② 补齐 Tick admission/queue、Core/Effect、Persist/HTTP/Upload pool 的 enqueue-block/queue-residency/execution、PortReply、WS、Storage、Timer、Event consumer、FFI/Web、生命周期与 Trace exporter 自观测，并保持高基数 correlation 只进入 Trace；③ 用 RecordingMetricSink/RecordingTraceHooks 的确定性剧本证明“Helix 背压、I/O 慢、PortReply 积压、客户端消费慢、Telemetry 自身故障”可区分；④ 先发布 ready-for-agent Spec，再经用户确认 tracer-bullet Ticket 后实施，每个独立 Ticket 聚焦验证并提交、不自动 push；⑤ Rust Cargo 只在完整模块和最终验收运行，最终执行 code review、三端 Rust 验收与 core 零运行时检查 | 赌注 高 | 可逆 是 | @2026-08-12T15:07:46+0800

✅ DONE Helix Trace + Metrics 第一轮生产接线完成：driver-host 已在 command admission、Tick/Core/Effect、persist/http/upload pool、PortReply、WS、Storage、Timer、Event、FFI 与 Trace exporter 边界发布 O(1) 低基数指标；固定 correlation 槽跨 Tick 闭合 command→HTTP/Persist reply→projection emit，250ms 默认阈值为慢 Span 添加 `helix.slow=true`。OTLP Trace/Metrics endpoint 已统一为 monitoring Collector，Grafana 合同扩为 96 个面板指标并通过生成/合同校验。`cargo check` 四 crate PASS，host 69/69、native 29/29、新增 FFI event 2/2 PASS；release Engine 10k 约 29k–30k roundtrip/s、混合约 23.6k ops/s，Storage 5000 行约 5.98ms。全 FFI lib 仍有 1 个未改动的空库 dialog query 异步事件断言失败，完整 package 仍被 `PostFields.present_fields` 私有化的既有集成测试挡住；Web bridge 与业务语义型 optimistic/authoritative/WS echo 指标明确标为 Planned，未伪造曲线 | @2026-08-12T18:20:00+0800
⚡ DRIFT-PRED generic host 无法诚实判断 optimistic/authoritative 或 WS echo 业务终态，因此实现从“所有名称直接产数”收敛为“96 个可闭合运行指标 + 业务 correlation 指标 Planned”；同时全量测试暴露两处与本 diff 无关的既有 blocker，最终证据按聚焦单测、四 crate check 与 release stress 分层报告 | @2026-08-12T18:20:00+0800

🎯 EXPECT 完成 MessageV3 第一批 9 个运行态 Gate 与第二批 Pin/Search/Todo 9 个代码 Gate 的真实闭环；可证伪断言：① 第一批只在新的 run-owned 活跃频道上重取 Go `posts/create`/producer/WS、Helix projection、SQLite read-back 和 emit 证据，G03a、G11a/b/c/d/e/g/h、G07d 未取得逐跳终态前不得改成 PASS；② Pin G05f/g 必须证明 Go pin HTTP→WS `post_pin`→Helix `im:post:updated`→SQLite props 读回，Search G14a-c 必须证明 Angular/Helix/Go command/body/`im:read:result` 合同，Todo G15a-d 必须证明真实 mutation→Go WS→Helix 自动 `im:todo:updated` 投影；③ 不改动现有 Angular `src-tauri/Cargo.toml` 与 `visual/`、Helix artifacts，不泄露 DSN/凭据；④ 每个新增/修改函数有紧邻注释，Rust cargo 只在完整模块和最终验收运行，最终按批次提交但不 push | 赌注 高 | 可逆 是 | @2026-08-11T16:05:00+0800

✅ DONE 第一批 9 个与第二批 9 个已完成真实复验：G03a、G11a/b/c/d/e/g/h、G07d、G05f/g、G14a/b/c、G15a/b/c/d 的最新 `evidence.json` 全部为 PASS。84 Gate 最新汇总为 Phase1 `22/5/5/1`、Phase2 `16/0/0/4`、Phase3 `15/0/0/0`、Phase4 `16/0/0/0`，总计 `69 PASS / 5 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 5 NOT_APPLICABLE`。验证包含 focused Rust real-chain、Go pin/store/app tests、`go build ./...`、`go vet ./...`、`git diff --check`；缺口清单与 handoff 已同步到 `crates/helix-driver-native/tests/messageV3-domain/`，未启动 WDIO、未执行真实数据库 migration、未 push | @2026-08-11T17:10:00+0800
⚡ DRIFT-PRED 运行态复现 Pin 的 PostgreSQL `42P10` 后，修复范围从预期的 Helix 测试扩展为 Go Store 兼容写法与手工 migration 文档；这是基于精确 add-pin 请求和 live schema 缺联合唯一约束的证据决定的，真实数据库 DDL 仍留给审批后的 migration 流程 | @2026-08-11T17:10:00+0800

🎯 EXPECT 修正为 Rust→真实 Go 8066 的 Phase1 Domain/Gate 集成测试；可证伪断言：① 仅在 `codex/messagev3-phase1-real-chain` worktree 新增 `helix-driver-native` Rust real-go test target 与必要 Cargo 注册，不改 Go/Angular 生产代码；② 测试默认直连 `http://127.0.0.1:8066/api/cses` 与 `ws://127.0.0.1:8066/api/v4/websocket`，自动发现现有凭据，禁止要求用户配置环境变量；③ 至少一条真实发送链同时证明 HTTP admission、WS `post`、Helix `sending→received/failed`、本地 SQLite row 与 Native EventSink emit，缺前置时明确 `BLOCKED/NOT_VERIFIED`；④ Phase1 目标按 Domain→Gate 命名，未能真实执行的 Gate 显式 `#[ignore]`/gap，不把 Node runner、health 或 fake server 计作 PASS；⑤ 聚焦 cargo target、`git diff --check` 通过后按独立批次提交，不自动 push | 赌注 高 | 可逆 是 | @2026-08-11T10:00:00+0800

✅ DONE Rust→Go 8066 真实 target 已落地：`cargo test -p helix-driver-native --test message_v3_phase1_real_go -- --nocapture --test-threads=1` 在当前 live Go server 上 `14 passed; 0 failed; 7 ignored`。真实发送路径取得 `sending` optimistic SQLite 行、Go `/posts/create` admission、WS `post`、Helix `im:post:received`、`send_status=sent` 行级读回和 `im:__bus__` envelope；失败路径取得 `im:post:send-failed` 与 `unsend/failed` 行级读回。按 registry 映射为 21/33 Gate real PASS，媒体 6 Gate 被 Go `media.type NOT NULL` 缺口阻断，其余显式 gap/非 Go 合同保持 `NOT_VERIFIED`/`NOT_APPLICABLE`；未改 Go/Angular 生产代码，未要求环境变量 | @2026-08-11T10:32:00+0800
⚡ DRIFT-PRED 先前的 Node runner 不是 Rust→Go 真实测试；本批次补入 `helix-driver-native` native engine/HTTP/WS/SQLite/EventSink 纵向集成 target，并把 Node runner 降为可选编排入口。当前仍不是 33/33 Green，媒体 schema 漂移需 Go owner 修复后再取消 ignore | @2026-08-11T10:32:00+0800

🎯 EXPECT Phase1 三仓真实链路测试按 `Phase1 → Domain → Gate` 分包落地；可证伪断言：① 只在独立 worktree `codex/messagev3-phase1-real-chain` 修改 Helix 测试/runner/证据文档，不改当前主工作树、Go/Angular 生产代码；② 默认无需用户配置环境变量，runner 自动指向 `127.0.0.1:8066` 与现有 Angular/Go 运行态，缺依赖时 fail-closed；③ 33 个 Gate 与 Helix registry、Angular manifest、真实 WDIO 类逐项对账，三个 Gap 保持 fail-closed；④ 任何业务 `PASS` 都必须有同轮 HTTP、WS、Native projection、DOM 和 storage 证据，canonical/行级 readback 缺失只标 `PARTIAL`；⑤ Node contract、dry-run、`git diff --check` 通过后按 Phase1→Domain 批次提交，不自动 push | 赌注 高 | 可逆 是 | @2026-08-11T09:00:00+0800

✅ DONE Phase1 三仓矩阵、33 Gate/3 Gap 对账、真实 WDIO runner 与 fail-closed summary 已落地；contract 4/4、Node syntax、diff check 通过，当前分支已拆为两批提交。真实业务 run 尚未启动：Go 8066 health=200，但 4544 已被现有 cses-client 占用，runner 按合同返回 `BLOCKED_PRECONDITION`，未把 health 或静态类当作业务 PASS | @2026-08-11T09:35:00+0800
⚡ DRIFT-PRED 实跑预期受现有 4544 监听进程阻断，未取得本轮 HTTP→WS→Native projection→DOM/storage 证据；释放 4544-4546 后须重跑至少 `send-core`，再决定 Phase1 全量是否可晋级 | @2026-08-11T09:35:00+0800
🎯 EXPECT 按 Spec 047 manifest 将 P1-P4 的 22 个 Helix Gate 拆为 `docs/mmd/Phase -> Domain -> Gate` 目录并补齐细粒度时序图；可证伪断言：① 新增 22 个 Gate Mermaid 文件和一个索引，目录/文件名与 manifest 的 phase/domain/gate_id 一一对应；② 每张图都包含 cses-client Tauri/Angular 入站、helix-im Tick/Effect/PersistOk、Go HTTP/WS、Native/FFI/Web raw event 交付与当前证据边界；③ 不把 HTTP 2xx、accepted、Rust replay 或静态注册写成真实三端 Green；④ 只改 docs/mmd 与本条 SESSION，不改业务代码、Spec manifest 或现有发送消息参考图；⑤ Mermaid 文本结构、manifest 对账、路径检查与 `git diff --check` 通过 | 赌注 中 | 可逆 是 | @2026-08-11T10:00:00+0800

✅ DONE 已新增 `docs/mmd/README.md` 与 22 张 Phase1-P4 Gate 时序图：P1=2、P2=9、P3=2、P4=9，manifest 对账 22/22；每图覆盖 cses-client/FFI/Web ingress、Tick/Effect/PortReply、SQLite/Go HTTP-WS、PersistOk/readback、Effect::Emit 与 raw bytes 交付，并保留 NOT_SYNCED_BY_SCOPE/NOT_VERIFIED 边界。Mermaid block、相对链接、非空文件与 `git diff --check` 通过；文档已提交 `da99c2a1`，未运行 Cargo/Go/WDIO/真实三端链路。

🎯 EXPECT 修正指定 10 张 Gate Mermaid 的解析语法且保持流程语义不变；可证伪断言：① 只替换 Mermaid 消息文本中的非法 ASCII `->`/`;` 等语法字符，不改参与者、步骤、事件名或分支含义；② 指定 10/10 文件可由 Mermaid CLI 解析；③ `git diff --check` 通过，不修改其他 Gate 或业务代码 | 赌注 低 | 可逆 是 | @2026-08-11T10:30:00+0800

✅ DONE 指定 10 张图已将消息文本中的 ASCII `->`/`;` 等 Mermaid 保留符号替换为等义 Unicode 文本，未改变流程步骤；Mermaid CLI 10/10 PASS，`git diff --check` PASS。未修改其他 Gate、业务代码或现有发送消息参考图；修正尚未单独提交。

🎯 EXPECT 按 cses-client 的 MessageV3 84 Gate 机读分母重建 `docs/mmd/Phase1-4/Domain/Gate` 流程图；可证伪断言：① Phase1/2/3/4 分别生成 33/20/15/16 张、总计 84 张 Gate Mermaid；② 目录领域严格对应 manifest（P2=read/timeline/sync，P3=channel/channel_member，P4=pin/bookmark/search/todo/announcement）；③ 每张图包含客户端入口、Helix command 或 inbound 边界、Tick/Effect/PortReply、SQLite/Go HTTP/WS、PersistOk/Result/Emit 与当前 partial/none 证据边界；④ 删除旧的 22 Gate 错相目录文件但保留 `发送消息.mmd` 参考图和现有用户脏改；⑤ manifest 对账、Mermaid 解析和 `git diff --check` 通过 | 赌注 高 | 可逆 是 | @2026-08-11T11:00:00+0800

✅ DONE 已按 cses-client manifest 重建 84 张 Gate 图：Phase1=33、Phase2=20（read/timeline/sync）、Phase3=15（channel/channel_member）、Phase4=16（pin/bookmark/search/todo/announcement）；旧 22 张错相目录图已移除，`发送消息.mmd` 保留为参考。manifest 目录对账 84/84，Mermaid CLI 11.16.0 解析 84/84 PASS；只读查询路径已收敛为 Command Result，当前 mock/helix partial、none、待定和断供仍按合同标注，未宣称真实三端 Green。

🎯 EXPECT 为 `docs/mmd/` 建立渐进式披露规则并由项目根 `AGENTS.md` 路由；可证伪断言：① 新增 `docs/mmd/AGENTS.md`，明确 L0-L4 阅读顺序、84 Gate 真源、目录合同、边界和验证；② 根 `AGENTS.md` 与 `docs/mmd/README.md` 均直接指向局部规则；③ 不修改 84 张 Gate 图、生产代码或用户已有 staged 参考图；④ 链接、规则章节和 `git diff --check` 通过 | 赌注 低 | 可逆 是 | @2026-08-11T15:11:53+0800

✅ DONE 已新增 `docs/mmd/AGENTS.md` 并在根 `AGENTS.md`、`docs/mmd/README.md` 建立入口；L0-L4 渐进式披露、84 Gate 分母、Phase/Domain、真源、负向事实和 Mermaid/manifest 验证合同均已固化。6 个局部链接、7 个必需规则章节和 `git diff --check` 通过；未修改 Gate 图、生产代码或根目录 `发送消息.mmd`。

🎯 EXPECT Phase1 三仓真实链路测试按 `Phase1 → Domain → Gate` 分包落地；可证伪断言：① 只在独立 worktree `codex/messagev3-phase1-real-chain` 修改 Helix 测试/runner/证据文档，不改当前主工作树、Go/Angular 生产代码；② 默认无需用户配置环境变量，runner 自动发现 `localhost:8066` 与现有 Angular/Go 运行态，缺依赖时 fail-closed；③ 每个 Phase1 Domain/Gate 至少有独立测试类或明确 `GAP`，覆盖 HTTP ack、Storage persist/readback、WS callback、Helix `Effect::Emit` 与 Angular 接收读回；④ 任何 `PASS` 都必须有真实服务/DB/WS/UI 证据，Rust synthetic、Go httptest、Angular mock 只能标 `CONTRACT`/`FOCUSED`；⑤ 通过 `git diff --check`、聚焦 Rust/Go/Angular 测试后按 Phase1→Domain 批次提交，不自动 push | 赌注 高 | 可逆 是 | @2026-08-11T09:00:00+0800

🎯 EXPECT 以独立 Search Gate 合同接通现有四个搜索命令；可证伪断言：① 仅修改 `helix-im` 现有 search outbound、三份聚焦测试、独立 Search 合同与本活账，不改冻结 Spec047、core 或任一 driver；② `im_search_post/user/channel/do` 均要求非空 `req_id`，拒绝 `user_id/team_id/company_id/sort`，并把 public snake_case 精确转换为 Go camelCase；③ post 支持有界筛选与 cursor，channel scope 缺 channel_id、无 keyword 且无筛选、非法时间或 page_size 均 fail-closed，另外三命令 keyword 必填；④ `im_search_do` 改为 read 并继续只通过既有 `im:read:result` 回灌，不新增事件、ABI、reducer 或缓存；⑤ focused test、`cargo check -p helix-im` 与 `git diff --check` 通过后独立提交且不 push | 赌注 高 | 可逆 是 | @2026-08-06T19:11:07+0800

✅ DONE 独立 Search Gate 最小实现已完成：四个既有 `/search/{post,user,channel,do}` 命令统一要求本地 `req_id`，拒绝客户端身份/租户/排序字段；post 支持 channel/global scope、发送人、内容类型、时间与有界 opaque cursor/page，目录与聚合搜索使用必填 keyword 和同一分页边界。`im_search_do` 已切为读族，Go 三段权威 JSON 继续经既有 `im:read:result` 原样回灌，未新增事件、ABI、driver、reducer、缓存或 SQLite。聚焦单元 15/15、P5a 18/18、read-relay 14/14 与 `cargo check -p helix-im` PASS；真实 Go/ES、PC/Flutter/Web runtime 与性能证据仍为 NOT_RUN。额外全量 lib 探测为 285/286，唯一失败是未改动的 `channel_sync_page_projects_capabilities_with_tenant_scope`，不冒充全量 Green且未越界修复 | @2026-08-06T19:20:40+0800
⚡ DRIFT-PRED 首次聚合测试先暴露 P5a 未导入公开 `is_read`，修正后全量 lib 又命中与 Search 无关的 channel-sync 既有失败；因此收口验证改为精确 Search 单元和两份集成 target，并保留全量失败原文 → 候选教训：小域提交应先跑聚焦 target，全量探测只能作为额外基线证据，不能因无关失败扩大文件租约 | @2026-08-06T19:20:40+0800

🎯 EXPECT 修复 Phase 1 显式失败重试在 Go 幂等 admission 后缺少第二次 WS echo 的真实链；可证伪断言：① 初次 posts/create HTTP 200 仍保持 Sending 并可在 15 秒后失败；② 只有 im_retry_send 的 HTTP 200 强制触发 getLatestPost 权威回读；③ 回读命中同 temporaryId 后以服务端 id 落定 sent、撤销 timer 且清理 pending；④ 只修改 helix-im send/query/correlation 与 focused F1 测试，不修改 Go 幂等合同 | 赌注 高 | 可逆 是 | @2026-08-01T04:30:00-0700

🎯 EXPECT 完成 MessageV3 Phase 1 发送域 33 Gate 的 producer-first 迁移；可证伪断言：① 首批只新增 `message-v3-phase1-send.registry.json`、`tests/message_v3/{support,contracts,post,upload,schedule,draft,announcement,channel}` 与 Phase 1 所需现役模块，不触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除；② 33 个 target 可发现且三个 Gap 保持 fail-closed；③ Core 不新增 Tauri/tokio/sqlx/reqwest 依赖，Rust Cargo 调用总计控制在模块完成与 Phase 验收两处；④ 未取得真实 HTTP/WS/Storage、dual-user 和 Native 证据的 Gate 不标 PASS | 赌注 高 | 可逆 是 | @2026-07-30T00:00:00-0700

✅ DONE MessageV3 Phase 1 发送域已完成 30 个真实接入 Gate + 3 个 Gap 的确定性生产链与独立 target：registry 提取的 33/33 target 全部通过，`helix-im` + `helix-driver-host` check 通过，Native/FFI/Web 三端 Rust 构建证据已沉淀。三个 Gap 继续 fail-closed，helix-core 未新增平台运行时依赖。预期偏离：旧 `UC04-16-EXECUTION-INDEX.md` 与当前 MessageV3 Gate Event 真源冲突，本轮一并删除；真实 PC/Mobile 消费、dual-user、Native 与 canonical 仍保持 UNVERIFIED，不借 deterministic/contract Green 晋级 | @2026-07-31T21:04:00-0700

🎯 EXPECT 完成 Spec 047 P2/G-06 快捷回复硬切；可证伪断言：① `im_quick_reply` 只发送 `{postId,emoji}`，HTTP ack 不发布 reaction 终态；② Go `post_update.quickReply` 原样写入 `message.quick_reply` 而不再寄生 `props.quickReply`，并与 cursor 原子提交；③ PersistOk 后必须从 durable message row 读回，sender/receiver 只收到同一绝对态 `im:post:updated {id,channelId,eventSeq,updateAt,quickReply}`；④同 seq 重放无重复事件、冷启动 durable row 可重建同一 payload，删除 G06 Frame/capability 专属测试，不修改 Go/PC/Mobile | 赌注 高 | 可逆 是 | @2026-07-29T06:34:00-0700

✅ DONE Spec 047 P2/G-06 完成：`im_quick_reply` 只发送 `{postId,emoji}`，HTTP SUCCESS 仅结束 admission、不伪造终态；可信 Go `post_update.quickReply` 不再寄生 `props`，而是经 O(1) 单列 patch 与 channel cursor 原子提交，PersistOk 后按 message id 读回 durable row，再向 sender/receiver 发布同一最小 `im:post:updated {id,channelId,eventSeq,updateAt,quickReply}` 绝对态。同 eventSeq 重放无 Effect，冷启动 durable row 可重建逐字相同 payload；乱序 reaction 先进入 MessageV3 连续性缓冲，前置 post 提交后继续走 G-06 原子链，不回落旧投影。旧 G06 Frame/Surface 回放测试、support scenario 与共享 reaction helper 已物理删除；G-06 4/4、G01/G05 4/4、server contract 12/12、sync mutation 12/12、projection schema 47/47、parser unit 5/5、八 Gate partial artifact schema、byte-exact 与新增热路径审计 PASS。四类 `.v1` JSON 由真实 Command/HTTP ack/Inbound/PersistAtomic/PersistOk/readback 回放生成；E3 真实 SQLite driver、E4 runtime、E5 dual-user 与 PC/Mobile 同步保持 NOT_RUN/NOT_SYNCED_BY_SCOPE | @2026-07-29T06:58:30-0700

🎯 EXPECT 完成 Spec 047 P2/G-05 转发与合并转发硬切；可证伪断言：① `im_relay_messages` 只读取 source durable rows，绝不改写 source identity；② HTTP 只发布每 target admission 的 `im:posts:batch-result`，可信 WS post 经 target message/channel/member/cursor 原子提交与读回后才发布 `im:post:received`、`im:channel:update`；③ sender 未读为 0，receiver 每个 target 独立 +1，合并转发 raw props 在事件边界收敛为 bounded `forwardDetail` 并同步进入 lastPost；④ batch-result 工厂迁出 render_ready，MessageV3 readback 后清理 forward target ledger，不修改 Go/PC/Mobile | 赌注 高 | 可逆 是 | @2026-07-29T06:15:00-0700

✅ DONE Spec 047 P2/G-05 完成：public `im_relay_messages` 只以 `StorageOp::Get` 读取 source durable row，不改写源 identity；individual/merged 两个 target 各自取得独立 HTTP admission 与 `im:posts:batch-result`，可信 WS authority 经 target message/channel/member/cursor 原子提交和复合键读回后才发布 `im:post:received`、`im:channel:update`，并在读回完成后消费 target ledger。sender 两个 target 未读均为 0，receiver 绝对态依次为 1/2；merged raw props 在事件边界有界归一为 `forwardDetail`，同时进入 post 与 lastPost。旧 render-ready batch-result 工厂及 Frame 专属 G05 测试已物理删除。G-05 2/2、server contract 12/12、projection schema 47/47、forward unit 3/3、七 Gate partial schema 与 diff check PASS；四类 `.v1` JSON 由真实 Command/Storage/HTTP/Inbound/PersistOk 回放生成。热路径新增 event/forward owner 无 clone；共享 `dispatch.rs` 的既有多 clone 债务未扩大。部分失败、E3 真实驱动、E4 runtime、E5 dual-user 与 PC/Mobile 同步保持 MISSING/NOT_RUN/NOT_SYNCED_BY_SCOPE | @2026-07-29T06:24:11-0700
⚡ DRIFT-PRED 删除共享旧 `batch_result.rs` 后，cutover checker 暴露尚未迁移的 G-12 manifest 仍把该文件登记为 owner；将其更新为当前实际承接 batch authority 的 `event/post.rs` 后复验，不恢复旧文件 → 候选教训：按 Gate 删除共享旧 producer 时必须同步修正所有后续 Gate 的预编码 owner 清单，路径存在性闸门应保留为物理下线后的即时对账 | @2026-07-29T06:26:00-0700

🎯 EXPECT 完成 Spec 047 P2/G-04a 离线撤回重放硬切；可证伪断言：① `im_sync_channels` 只使用 Helix cursor，type3 authority 与 cursor 原子提交前零 Emit；②提交后离线 receiver 收到最小 `im:post:revoke` 与既有 sync state，sender 不重复收到在线已处理事实；③同 seq replay 与恢复 cursor=1 的冷启动 no_change 均不重复发布；④删除 sync revoke 的 `im:post:deleted` 展示型投影，不修改 Go/PC/Mobile | 赌注 高 | 可逆 是 | @2026-07-29T06:55:00-0700

✅ DONE Spec 047 P2/G-04a 完成：explicit sync 的 type3 authority 将 `message.revoke=1` 与 channel cursor 原子提交，PersistOk 前零 Emit；提交后离线 receiver 只收到同 identity 的最小 `im:post:revoke` 与 `im:sync:state`，在线已处理的 sender 不重复接收。相同 eventSeq replay 被 cursor 丢弃，新 Module 恢复 cursor=1 后的 no_change 也不重复发布。sync 撤回旧 `im:post:deleted` 已从现役分支物理下线；G-04a 2/2、C3 mutation 12/12、sync effects 6/6、六 Gate partial schema 与 diff check PASS，四类 `.v1` JSON 由完整首次 sync、duplicate replay、cold-start recovery Tick 生成。热路径新增 revoke factory 无 clone；`sync_effects` 通用 batch helper 的既有 clone 债务未扩大。E3 真实 SQLite driver 读回、真实断网/runtime、dual-user 与 PC/Mobile 同步保持 NOT_RUN/NOT_SYNCED_BY_SCOPE | @2026-07-29T06:05:59-0700

🎯 EXPECT 完成 Spec 047 P2/G-04 在线撤回硬切；可证伪断言：① posts_update.revoke 在 message/cursor 原子提交前零 Emit，提交后双视角只发同 identity 的 im:post:revoke；② update_channel.lastPost JSON 解析、去传输元字段后原子写 channel/channel_member，复合键读回才发 im:channel:update；③ sender/receiver 未读绝对值分别保持 0/1，撤回不递增；④ 删除本 Gate 的 im:post:deleted、batch-updated、thin channel update 与 Frame 刷新，不修改 Go/PC/Mobile | 赌注 高 | 可逆 是 | @2026-07-29T05:30:00-0700

✅ DONE Spec 047 P2/G-04 完成：在线 `posts_update.revoke` 将 message 可变业务列、revoke 与 cursor 原子提交，PersistOk 后双方只收到同 identity 的 `im:post:revoke`；`update_channel.lastPost` 的 JSON 字符串去除 props/system_event/viewers 等传输与展示字段后原子更新 channel/current member，并经复合键读回发布 `im:channel:update`。sender/receiver 未读绝对值保持 0/1，撤回不递增；旧在线 deleted、纯撤回 batch、thin channel update 与 Frame 刷新均不再可达，混合 edit 的 batch 暂由未迁移 edit Gate 保留，离线撤回留给 G-04a。G-04 2/2、旧 revoke regression 2/2、五 Gate partial schema 与 diff check PASS；四类 `.v1` JSON 由真实回放生成。热路径新增 owner 除必要一次 JSON viewers 编码外无重复 clone；`update_channel` 未迁移通用分支的既有双 clone 仍记为后续 Gate 债务。当前 generated 为 5/30，E3 runtime、真实 dual-user 与 PC/Mobile 同步保持 NOT_RUN/NOT_SYNCED_BY_SCOPE | @2026-07-29T06:20:00-0700

🎯 EXPECT 完成 Spec 047 P2/G-03a 失败重发硬切；可证伪断言：① 初次发送的 optimistic sending 只作为失败前置，Gate sender 样例顺序精确为 send-failed→sending→received→channel:update，failed/retry-sending 复用同一 temporaryId 且没有 capability/snake 别名；② receiver 在成功 authority 前无事件，之后只收到 received 与自身 unread=1 的 channel:update；③ retry HTTP 复用原 temporaryId 且 PersistOk 前不发起；④ 四类 JSON 由真实命令、PortReply 与 Inbound 测试生成，不修改 PC/Mobile/Go | 赌注 高 | 可逆 是 | @2026-07-29T05:20:00-0700

✅ DONE Spec 047 P2/G-03a 完成：初次 optimistic sending 仅作为失败前置，Gate sender canonical 精确为 send-failed→同 temporaryId sending→received→channel:update，receiver 仅在成功 authority 后收到 received 与 unread=1 的 channel:update；retry PersistOk 前无 HTTP，成功/失败 posts/create 终态仍保留 window 与 causation。旧 sending/send-failed projection factories 已物理删除，所有发送、重试、超时及上传失败出口统一经 `event::post` 输出最小 MessageV3 data，无 capability 与 snake/camel 重复别名。G-03a 2/2、projection schema 47/47、posts_create unit 2/2、四 Gate partial schema 与 cutover self-test PASS；四类 `.v1` JSON 已由完整 Command/PortReply/Inbound 回放生成。热路径新 event helper 审计为 clean；其余 owner 命中的 clone 为既有债务且本任务未新增。当前 generated 为 4/30，E3 runtime、真实 dual-user 与 PC/Mobile 同步保持 NOT_RUN/NOT_SYNCED_BY_SCOPE | @2026-07-29T05:24:35-0700

🎯 EXPECT 完成 Spec 047 P2 的 G-02 文档消息硬切；可证伪断言：① sender/receiver 的 `im:post:received` 均保留 authority `props.document.url/title/placeholder`；② 两视角 `im:channel:update.lastPost` 指向同一 DOCUMENT post 并保留文档预览字段，sender unread=0、receiver 仅 +1；③ PersistOk 前不 Emit，四类 JSON 由真实测试生成；④ 不改变 G-01 TEXT 的最小 payload，不修改 PC/Mobile/Go | 赌注 中 | 可逆 是 | @2026-07-29T09:35:00-0700

✅ DONE Spec 047 P2/G-02 完成：DOCUMENT authority 的业务 `props.document` 只解析一次并同时进入 `im:post:received` 与持久化 `lastPost`，技术字段 `channel_event_seq` 不再重复泄漏到业务 props；sender/receiver 仍分别读回 unread=0 与 N+1。G-01/G-02 聚焦测试 4/4、三 Gate 部分 generated schema、cutover self-test 与 diff check 均 PASS；G-02 四类 `.v1` JSON 由 Rust producer 生成，manifest 标为 MIGRATED。全量 generated 仍为 3/30，E3 runtime、真实 dual-user 与 PC/Mobile 同步保持 NOT_RUN/NOT_SYNCED_BY_SCOPE；中途发现部分 schema 模式仍遍历 30 个空目录，已改为只校验存在 generated 的非空子集，默认全量模式继续要求精确 30 | @2026-07-29T05:03:00-0700

🎯 EXPECT 完成 Spec 047 P1 的 G-15a/G-01 基线硬切；可证伪断言：① G-15a 只在 channel/member 原子提交后发 `im:channel:created` 与 `im:channel:members`；② G-01 以复合键作用域存储原语在 O(1) SQL 内更新当前 viewer 的 `channel_member`，sender 未读绝对值归零、receiver 在前进守卫下仅加一；③ message、channel 公共 lastPost、viewer member 绝对态和 cursor 同一原子提交，提交后复合键读回再发 `im:post:received` 与 `im:channel:update`；④ 两 Gate 产物均由真实 `ImModule::handle` 测试生成四份 JSON，不含 Frame/Surface/capability/重复别名；⑤ 不修改 PC/Mobile/Go，不触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除 | 赌注 高 | 可逆 是 | @2026-07-29T08:30:00-0700

✅ DONE Spec 047 P1 基线完成：G-15a 的 channel/member 原子提交成功后只发 `im:channel:created` 与 `im:channel:members`，owner/invitee 的 `viewerRole` 分别为 OWNER/MEMBER；G-01 用 `ScopedGuardedBump` 与 `ScopedGet` 在复合主键上兑现 receiver 未读 +1、sender 未读归零，并在 message/channel/current-viewer-member/cursor 原子提交及读回后发 `im:post:received` 与 `im:channel:update`。两 Gate 聚焦测试 4/4、HostStorage 真 SQLite 1/1、core 21 项、三端 Rust check、Web wasm build、core 零运行时依赖与 cutover self-test 均 PASS；四类 `.v1` JSON 已由 Rust producer 生成，manifest 标为 MIGRATED。当前 generated 分母仍为 2/30，PC/Mobile/Go 未修改，runtime、真实 dual-user 与消费者同步保持 NOT_RUN/NOT_SYNCED_BY_SCOPE，既有 `UC04-16-EXECUTION-INDEX.md` 删除未触碰 | @2026-07-29T09:25:00-0700

🎯 EXPECT 按 Spec 047 的 90 项任务实施 MessageV3 Gate Event 硬切；可证伪断言：① 严格从 T001 开始按 P0→P7 串行，任何非并行任务失败立即停止；② 先完成治理、30 Gate 预编码删除清单与公共红测，再接生产代码；③ 每 Gate 必须真实 `ImModule::handle` 红测→PersistOk 后 `{event,data}`→生成四份 JSON→物理删除专属 V2/V1→聚焦 Green，任务证据存在后才标 `[X]`；④ 不修改 PC/Mobile/Go，不触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除；⑤ Rust 编码遵守 Cargo 全程 2-3 次纪律，最终 R4 只接受真实验证证据 | 赌注 高 | 可逆 是 | @2026-07-29T03:21:00-0700

🎯 EXPECT 重新执行 Spec 047 的 Plan/Tasks 并使 R2/R3 对齐最新规格；可证伪断言：① plan、research、data-model、schema、quickstart 与 spec 的 Helix-only MessageV3 范围一致，冻结 sample 不被 generated 覆盖；② 每 Gate 删除清单在任何生产编码前已经完整登记，PersistOk 公共实现前有专门红测试；③ 七个业务域与物理测试包映射无歧义，G-13/G-14 属 read、G-16b 属 channel-member；④ 90 个任务与 90 个 Loop Task Contract 一一对应且 R2/R3 PASS；⑤ 仅修改 Spec 047 规划资产、SESSION 和 loop ledger，不触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除，不运行 Cargo | 赌注 高 | 可逆 是 | @2026-07-29T03:10:00-0700

✅ DONE 已重新执行 Spec 047 的 Plan/Tasks：先修正 SC-009 与 FR-044 的范围冲突，把零引用限定为 Helix，客户端保持 `NOT_SYNCED_BY_SCOPE`；冻结 sample 与 `generated/*` 分层不再互相覆盖。P0 现在要求生产编码前完成 30 Gate 删除清单，T009 在公共 PersistOk 编排前先做“提交前不 Emit、匹配后恰好一次”的红测。七个实现域已与七个测试包对齐，`channel-member -> channel_member`，G-13/G-14 属 read、G-16b 独立归 channel_member。90/90 checkbox 与 Loop Task Contract、30/30 Gate 路径与 target、16/16 checklist、219 份 JSON 解析、schema 结构/负例逻辑与 `git diff --check` 均通过；Spec Kit R1 event 32、R2 event 33、R3 event 34 PASS，event 35 已给出 `IMPLEMENTATION_ADMITTED`。未运行 Cargo，未触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除 | @2026-07-29T03:12:02-0700

🎯 EXPECT 修正 Spec 047 的 MessageV3 测试组织；可证伪断言：① 30 个 Gate 不再平铺在 `tests/` 根，每个 Gate 都有 `tests/message_v3/<domain>/<gate>/main.rs` 独立目录；② 先按 post/schedule/timeline/read/channel/sync 六个领域分包，再由 `Cargo.toml` 为每个 Gate 显式注册唯一 `message_v3_<gate>` integration target；③ plan、research、data-model、schema、quickstart、tasks 的路径与命令一致；④ 30 个目录、30 个 target、90 个任务与 90 个 Loop Task Contract 精确对账；⑤ 只修正规划资产，不创建测试或修改生产代码，不触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除 | 赌注 中 | 可逆 是 | @2026-07-29T01:56:42-0700

✅ DONE Spec 047 已把测试布局修正为 `tests/message_v3/<domain>/<gate>/main.rs`：post 9、schedule 2、timeline 8、read 2、channel 5、sync 4，共 30 个独立 Gate 目录；每个红测试任务同时要求在 `crates/helix-im/Cargo.toml` 注册唯一 `message_v3_<gate>` 显式 target。公共 artifact/scenario 工具独立放在 `tests/message_v3/support/`，三项基础合同各自使用 `tests/message_v3/contracts/<contract>/main.rs`。迁移 schema 新增 `test_package/test_path`，quickstart 改用 `--test message_v3_g01`。机器对账确认 30/30 路径、30/30 target、90/90 task/contract，无旧平铺路径；Spec Kit R2 event 26、R3 event 27 PASS，JSON 与 `git diff --check` 通过。未创建测试类、未修改 Cargo/生产代码、未运行 Cargo | @2026-07-29T01:56:42-0700

🎯 EXPECT 在原 `specs/047-im-event-gate-inventory` 内完成 MessageV3 重构澄清、设计和任务编排；可证伪断言：① Constitution 2.0.0 与现有 Spec 同步改为 `Effect::Emit {event,data}` 唯一真源，不再要求 Frame/Surface；② plan/tasks 精确覆盖用户确认的 30 Gate，并按 P0→P7、G-15a→G-01→Post→Schedule→Timeline/Read→Channel→Sync→全局清除严格串行；③ 每个 Gate 都有真实 `ImModule::handle` 红测试和生产接码/生成 JSON/删除旧路径任务，生成器不再手拼 payload；④ 本版本只生成 Helix `.v1`，不修改或同步 PC/Mobile；⑤ Spec Kit R2/R3、90 个 checkbox 与 Loop Task Contract 一一对账、JSON/Node/diff check 通过，不运行 Cargo 或冒充 coding 已完成 | 赌注 高 | 可逆 是 | @2026-07-29T01:43:53-0700

✅ DONE 已在原 Spec 047 完成 Clarify、Plan、Tasks：Constitution 升级到 2.0.0，规格新增 FR-037 至 FR-048 和 SC-009 至 SC-015；plan 固定 MessageV3 `{event,data}`、Gate 纵向硬切、Helix-only `.v1` 生成及 P0→P7 顺序；research/data-model/quickstart 与四份 JSON schema 已落地；tasks 共 90 项，其中 30 Gate 各自严格拆为真实生产回放红测试和实现/生成/删除旧路径两步，90/90 Loop Task Contract 无缺失、重复或额外。Spec Kit R2 event 20 PASS、R3 event 23 PASS，Node 语法、全部 contract JSON、任务序列/表映射与 `git diff --check` 通过。当前仅完成可执行规格、重构方案与任务，不含生产 coding、Cargo、WDIO、Mobile GTest、runtime、dual-user 或 canonical Green；既有 `UC04-16-EXECUTION-INDEX.md` 删除未触碰 | @2026-07-29T01:43:53-0700

🎯 EXPECT 将 Spec 047 的 30 Gate 样例改为远端 PostgreSQL / 本地 Helix SQLite 结构驱动；可证伪断言：① 记录预发 PostgreSQL `10.200.9.168:5432/mattermost-imv2/public` 与本机 tenant 444/999 `tenant.db` 的只读 schema 真源；② 每个 Gate 的 inbound 明确 remote table/column → Go WS wire，outbound 明确 PersistOk 后 local table/column → `{event,data}`，不再把目标 UI 文案冒充持久字段；③ 30 Gate 均有 remote/local persistence basis，使用的 table/column 全部存在于锁定 schema；④ G04 撤回以 remote `post.revoke` / local `message.revoke` 和 viewer `channel_member.last_post/unread_count` 为依据，actorName/拼装系统文案不再伪装成表字段；⑤ 仅修改 Spec 047 生成器、规格、发现/验证/清单、schema lock 与 SESSION，不改生产代码，生成器、30 Gate schema 对账、JSON、幂等、Spec Kit R1 和 diff check 通过 | 赌注 高 | 可逆 是 | @2026-07-28T22:05:00-0700

✅ DONE Spec 047 已锁定远端 PostgreSQL `10.200.9.168/32:5432/mattermost-imv2/public` 与本地 Helix 444/999 tenant SQLite 的实时只读表、列、JSON/JSON-text 键形和观察计数；30/30 inbound 都带 remote persistence basis，30/30 outbound 都分别带 sender/receiver PersistOk 后 local basis 与逐 event field lineage。PostgreSQL row、Go WS wire、SQLite row、raw `{event,data}` 四层已分栏，未直落本地列的查询/分页/correlation/角色字段明确列入 non-storage/derived。G04 raw outbound 现只含 `message` 与 `channel_member.last_post/unread_count` 可读回字段，移除 actorName/systemEvent/viewer 文案；Go `system_event` 明确标为提交后 WS 派生。schema/table/column/lineage 对账 30/30、G04 禁用字段、60 份样例 JSON、两次生成幂等 hash `7daf8f6d...`、`.v1` 忽略规则/UI 术语、`git diff --check` 均 PASS；Spec Kit R1 event 17 PASS。未运行 Cargo/WDIO/真实业务链，capture 仍为 0/90 INCOMPLETE，未修改生产代码，未触碰既有 `UC04-16-EXECUTION-INDEX.md` 删除 | @2026-07-28T22:10:00-0700

🎯 EXPECT 纠正 Spec 047 G04 撤回双视角合同；可证伪断言：① inbound 展示 Go 实际 `posts_update` 撤回主广播及仅撤回“最后一条”时出现的 `update_channel` 副广播；② outbound 目标只保留 `im:post:revoke` 与 `im:channel:update`，消息原 identity 置 `revoke=true`，不把撤回建模为删除；③ sender 文案为“您撤回了一条消息”，receiver 为“张三撤回了一条消息”，双方 channel lastPost 为同一 viewer-local 系统消息且撤回不新增未读；④ 规格明确当前 Go 的全表最新判断、lastPost JSON 字符串、Helix `im:post:deleted`/“某人”/thin channel update 和 WDIO dialog N/A 缺口；⑤ 仅改 Spec 047 生成器/规格/验证/清单与 SESSION，生成器、G04 结构断言、Spec Kit R1 和 diff check 通过 | 赌注 高 | 可逆 是 | @2026-07-28T21:25:00-0700

✅ DONE G04 已按撤回语义重写：inbound 展示 `im_revoke`、Go 必达 `posts_update(revoke=true + system_event)` 与仅 lastPost 分支的 `update_channel(lastPost JSON string)`；outbound 目标收敛为双方各自的 `im:post:revoke` 和 `im:channel:update`，同一 post identity 不删除，sender 文案“您撤回了一条消息”、receiver 文案“张三撤回了一条消息”，channel lastPost 同步 viewer-local 系统消息，撤回不新增未读。源码核验确认当前 Go lastPost 判断错误地使用全表 `GetPostLaster()`，Helix 仍 emit `im:post:deleted + im:post:batch-updated`、receiver 只能“某人…”且 channel update 仅含 id，WDIO G04 明确 dialog mutation N/A，故保持 TARGET_REQUIRED/PARTIAL。Node 语法、G04 inbound/outbound 结构断言、全部 JSON、生成幂等 hash `7090f130...`、`.v1` UI 术语零命中、`git diff --check` 与 Spec Kit R1 event 15 PASS；未运行 Cargo/WDIO/真实链，capture 仍为 0/90 INCOMPLETE，未修改生产代码 | @2026-07-28T21:29:17-0700

🎯 EXPECT 纠正 Spec 047 G03a 的失败重发样例；可证伪断言：① inbound 明确为 `im_retry_send{message_id=failed temporaryId}`；② sender 顺序为 failed→retry 后同 temporaryId sending→received→channel update；③ receiver 只见成功 received/channel update；④ 不虚构当前生产不存在的 `im:post:retrying`，而在样例 warning 中说明 retry 复用 `im:post:sending`；⑤ 仅改 Spec 047 生成器/规格/验证与 SESSION，生成器、G03a 结构断言、Spec Kit R1 和 diff check 通过 | 赌注 中 | 可逆 是 | @2026-07-28T21:14:37-0700

✅ DONE G03a 已按失败消息重发链纠正：inbound 保留 `im_retry_send{message_id=temporaryId}`；sender 样例改为 `im:post:send-failed` → retry 后同 temporaryId 的 `im:post:sending` → `im:post:received` → `im:channel:update`，receiver 仍只见成功 received/channel update。当前生产 `continue_retry` 在 sending 持久屏障后复用 `emit_post_sending_with_reply`，因此没有另一个 `im:post:retrying` 事件，样例 warning 已明确该事实。Node 语法、G03a command/顺序/identity/视角断言、`git diff --check` 与 Spec Kit R1 event 11 PASS；未运行 Cargo/WDIO/真实链，未修改生产代码 | @2026-07-28T21:15:39-0700

⚡ DRIFT-PRED mandatory after-specify hook 按规格指纹变化自动更新 review-ledger，超出原预期文件集，但属于 Spec Kit 机器合同且 R1 已通过 | @2026-07-28T21:15:39-0700

🎯 EXPECT 将 Spec 047 全部 30 个 Gate 的 outbound 样例收敛为 sender/receiver 双视角；可证伪断言：① 每份 outbound 都只有 `sender_samples` 与 `receiver_samples` 两组 action event，每项精确 `{event,data}`；② persona-local Gate 的 receiver 为空，G14 read/readers 与 G16b roster/left 按实际观察者拆分；③ G01/G02/G03a/G05/G10c/G10d 补写后 channel 目标绝对态，G01 post 与 lastPost 的 createAt 一致；④ 所有 target/inferred Gate 明确标 TARGET_SAMPLE，不冒充当前 producer；⑤ 只修改 Spec 047 生成器/规格/验证和本条 SESSION，不改生产代码，30 Gate JSON、幂等、Spec Kit R1 与 `git diff --check` 通过 | 赌注 高 | 可逆 是 | @2026-07-28T21:02:18-0700

✅ DONE Spec 047 的 30 个 Gate outbound 已统一为 sender/receiver 双视角，每个 action sample 精确 `{event,data}`，移除旧 samples/failure 分组、adapter 包装、能力字段、重复别名与 delta。persona-local 9 Gate 的 receiver 明确为空；G14 拆为 reader `im:post:read` / author `im:post:readers`；G16b 拆为 remaining roster / leaver 目标 `im:channel:left`，且退出成员已从 remaining roster 移除。G01/G02/G03a/G05/G10c/G10d 补写后 channel 绝对态，G10d 分 latest sender 与 away-bottom receiver；12 Gate 因 target/inferred 保持 TARGET_SAMPLE。30/30 event union、全部 JSON、禁用字段、G01 identity/createAt/unread、G14/G16b 分流、两次生成 hash `15951de4...`、`.v1` UI 术语零命中、`git diff --check` 与 Spec Kit R1 event 9 PASS；未运行 Cargo/WDIO/真实链，capture 仍为 0/90 INCOMPLETE | @2026-07-28T21:08:42-0700

⚡ DRIFT-PRED 原预期只改生成器/规格/验证/SESSION，但 Spec Kit 质量合同要求同步 requirements checklist，mandatory after-specify hook 也更新 review-ledger；两者均为本规格机器合同，不是生产代码扩张 | @2026-07-28T21:08:42-0700

🎯 EXPECT 只补齐 Spec 047 的 G01 双视角频道终态；可证伪断言：① G01 outbound contract 新增 `im:channel:update`，并如实标为当前 raw producer 缺失的 TARGET_REQUIRED；② sender/receiver 样例中的每个信封仍只有 `{event,data}`；③ `data.lastPost` 只含消息 id、sender、正文、createAt，sender `unreadCount=0`、receiver 基线 `unreadCount=1`，不暴露 delta 给壳计算；④ 规格明确当前 Rust 已有 own +0/other +1 与写后读回，但 own +0 不等于清零；⑤ 不修改其他 Gate 或生产代码，生成器、JSON、聚焦结构断言和 `git diff --check` 通过 | 赌注 中 | 可逆 是 | @2026-07-28T20:40:36-0700

✅ DONE G01 outbound contract 已补 `im:channel:update` 并标记 `TARGET_REQUIRED/PARTIAL`：sender 序列为 sending→received→channel update(unreadCount=0)，receiver 为 received→channel update(unreadCount=1)，失败仍仅 sender 可见。channel data 只保留 channelId、lastPost{id,userId,message,createAt} 与写后绝对 unreadCount，每个样例信封仅 `{event,data}`。当前源码核验确认 own echo 用 GuardedBump delta=0、他人可见消息 delta=1、last_post/last_post_at 同批落库再读回；但现有 raw update 仅 `{channel_id}`，且 delta=0 不会清除历史未读，均已列为实现缺口。Node 语法、211 份 JSON、G01 字段断言、本地 v1 禁用 UI 层术语、`git diff --check` 与 Spec Kit R1 PASS；未运行 Cargo/WDIO/真实链，未修改生产代码或其他 Gate，capture 仍为 0/90 INCOMPLETE | @2026-07-28T20:45:00-0700

🎯 EXPECT 为 Spec 047 的 30 个 Gate 各生成一份可审阅的 inbound/outbound 结构样例；可证伪断言：① 每个 Gate 新增 `inbound-sample.json` 与 `outbound-sample.json`；② inbound 只使用当前 `Tick` 变体、Matrix command 字段及生产测试中的 WS wire 字段；③ outbound 每条均为真实 `Effect::Emit` `{event,data}` 信封并附工厂源码，Native `channel=event` 只作为 adapter view；④ 样例明确标为 SOURCE_DERIVED_SAMPLE、不是运行 capture；⑤ 60 份样例 JSON 可解析、每个 outbound event 能在引用源码中找到、生成幂等且 capture 仍为 0/90 INCOMPLETE | 赌注 高 | 可逆 是 | @2026-07-28T20:20:00-0700

✅ DONE 已在忽略目录 `.v1/gates/<Gate>/` 为 30 Gate 各生成 `inbound-sample.json` 与 `outbound-sample.json`：共 35 个 command、30 个 WS authority 和 66 个 Effect::Emit 样例；每份均附 Tick/emit 工厂源码、Native channel view 与 FFI batch view，并固定 SOURCE_DERIVED_SAMPLE/runtime_capture=false。30/30 Gate、60/60 样例文件、Matrix 必填 command 字段、66 个 emit 源文字面、adapter event 对账、JSON、忽略规则和 `git diff --check` 通过；连续两次生成全树 hash `36843760...`。未运行 Cargo/WDIO/真实链，capture 仍为 0/90、manifest INCOMPLETE | @2026-07-28T20:40:00-0700

🎯 EXPECT 纠正 Spec 047 的 Gate 证据边界；可证伪断言：① `.v1` 与本规格不再把 Surface/InteractionFrame/DOM 当业务出站；② 入站统一按 `Tick::{Command,Inbound,PortReply,Timer}`，内部副作用按 `Effect`，对端业务出站只认 `Effect::Emit` 的 `{event,data}`；③ Native 的 `channel=event` 与 FFI batch 仅作为适配器承载，不冒充第二套领域合同；④ 30 Gate 均列出当前源码已存在的 emit channel，缺失的 timeline/定位语义明确标 `INFERRED_REQUIRED`；⑤ 生成器幂等、`.v1` 零 Frame/Surface、JSON 与 `git diff --check` 通过 | 赌注 高 | 可逆 是 | @2026-07-28T19:40:00-0700

✅ DONE 已把 Spec 047 与忽略目录 `.v1` 改为 `Tick -> Effect -> Effect::Emit` 三段证据合同：30 Gate 各有 aggregate/inbound/effects/outbound 四份合同，outbound 只含当前源码可定位的 `{event,data}` channel；Native `channel=event` 与 FFI batch 明确降为适配器承载。G-10 至 G-11a 的窗口、anchor、定位与分页字段保持 PARTIAL/INFERRED_REQUIRED。30 Gate、120 合同、30 状态 JSON、全部 emit 源文字面核验、90 个空 capture 槽位、两次生成全树 hash `6cac41f7...`、忽略规则、旧 UI 术语零匹配和 `git diff --check` 均 PASS；未运行 Cargo/WDIO/真实链，manifest 仍为 INCOMPLETE | @2026-07-28T20:05:00-0700

⚡ DRIFT-PRED 首版把完整 Effect 流和旧 UI 投影错误合并成 outbound/response，甚至把 adapter 后的可见状态当 Helix 业务出站；当前源码证明模块输出边界是 `EffectSink`，跨端业务通知是 `Effect::Emit(DomainEventBytes)`，Native channel/FFI batch 只是交付机制 → 候选教训：Gate 证据先固定 module boundary，再分别登记 Tick、全 Effect 与 Emit，禁止从消费端 UI 合同反推 core 出站 | @2026-07-28T20:05:00-0700

🎯 EXPECT 为 Spec 047 建立被 Git 忽略的本地 `.v1` Gate 证据目录；可证伪断言：① 截图 30 Gate 各有独立目录与 `contract.json`、`capture-status.json`；② 每个 Gate 明确 Helix 视角的 ingress command/WS/HTTP response 与 egress HTTP/Persist/Frame，不把方向混写；③ 当前 Strict Matrix、Helix tests、WDIO mock/single/dual 与历史 evidence 仅作为 source index，未找到同一 run 的原始入站/出站/响应一律标 `NOT_CAPTURED`；④ `.v1` 被仓库 `.gitignore` 命中且不会进入 commit；⑤ 生成器重复执行结果稳定，30/30 目录、JSON 解析、状态汇总与 `git diff --check` 通过 | 赌注 中 | 可逆 是 | @2026-07-28T19:05:00-0700

✅ DONE 已生成 `specs/047-im-event-gate-inventory/.v1/` 本地证据索引：30/30 Gate 各有 aggregate、inbound、outbound、response 四份声明合同、capture status 与独立 captures 槽位目录；Helix producer/fixture 与 WDIO 三 lane 源码索引均为 30/30，历史 evidence 为 26/30，G-10b/G-10c/G-10d/G-11a 无独立历史 evidence。当前同一 run 原始 capture 为 0/90，manifest 如实保持 INCOMPLETE；生成器连续两次全树 SHA-256 相同，120 个合同 JSON、30 个状态 JSON、Git ignore、Node 语法和 `git diff --check` 均 PASS | @2026-07-28T19:20:00-0700

🎯 EXPECT 以当前 Helix producer 测试与 cses-client WDIO 场景为证据，梳理全量 Gate 的事件域、指令族与 sender/receiver 双视角合同；可证伪断言：① 覆盖截图中的 30 个 Gate，并按 im:post、im:channel、im:channel-member、sync/recovery 分类；② 每个 Gate 明确 actor、observer、触发、权威终态、Surface/DOM、当前 Helix/WDIO 覆盖与缺口；③ 源码或场景未覆盖的 receiver 视角显式标为 MISSING，并给出推导应有合同而不冒充现状；④ 只新增一个 Spec Kit feature、质量清单、loop discovery 与本条 SESSION，不修改生产代码或测试；⑤ requirements checklist、mandatory after_specify hook 与 git diff check 通过 | 赌注 高 | 可逆 是 | @2026-07-28T16:30:00-0700

✅ DONE 已按截图 30 Gate 完成 IM 指令域与 Actor/Observer 双视角规格：分为 sync/recovery、post、schedule、timeline/read、channel/channel-member 五组，逐 Gate 固定触发指令、角色终态、权威完成边界、Surface、Helix/WDIO 源码覆盖和缺口；当前 Matrix epoch 12 的 50/47/3 分母、截图遗漏 G-03 及 G-17 至 G-27 范围差异已单列。30 个主表行、90 个 WDIO Gate-lane 源入口、requirements checklist、JSON、模板占位与 `git diff --check` 静态校验通过；mandatory helix-loop R1 PASS 并进入 planning，但本轮按用户要求停在人工评审，未运行 Cargo、WDIO、Native、Go、WS 或双账号真实链 | @2026-07-28T18:53:00-0700

🎯 EXPECT 将 Mobile G-03 文件消息与图文混合消息对齐 Helix `main@94671a0` 并完成真实链验收；可证伪断言：① Mobile 原子 repin 的 bundle/header/library/Strict Matrix/Command API 同源于正式 commit blob，不读取 Helix 脏工作树；② Java base 显式配置为 `http://cses-server-paas25-test.cses.svc.cluster.local:3399`，Go/Java/WS 三个 origin 不互相推导；③ 删除 UC03 历史 waiver，FILE 与 rich 分别覆盖 C++ strict decode、QuickJS typed send、FFI batch、Surface Frame、Venus 绑定及真实 Prepare→OSS PUT→本地 Complete→posts/create→WS/readback；④ 不恢复旧 `/media/get` 或 Java complete，不把 HTTP 200、AcceptedCommand、Mock 或单条 FILE 冒充双链 Green；⑤ 聚焦 Helix 25 测、Mobile contract/local/Venus/real-chain、source parity 与 diff check 全部 required fail=0/skip=0 后才提交，Cargo 总调用控制在 2-3 次 | 赌注 高 | 可逆 是 | @2026-07-28T19:20:00+0800

🎯 EXPECT 收口已拍板的 G03 `Prepare → OSS PUT → 本地 Complete → posts/create` 链路并准备提交推送；可证伪断言：① 不恢复或调用 Java `/oss/completeMediaUpload`，ADR、Strict Matrix 与 G03 文档统一改为 PUT 2xx 后本地原子完成，最终 props 只保留 `bucket + uri`；② 不修改 canonical-id 对账链；③ 本次新增媒体完成/投影代码通过增量热路径审计且所有新增/改名函数具备紧邻注释；④ 聚焦测试、helix-im check/test、三端 Rust 验收、Web wasm build、core 零运行时依赖与 diff check 全部通过后才整理提交并 push | 赌注 中 | 可逆 是 | @2026-07-28T14:00:00+0800

✅ DONE G03 已收口为 `Prepare → OSS PUT 2xx → 本地 PersistAtomic Complete → posts/create`，不再调用 Java `/oss/completeMediaUpload`；内存只保留完成 file id，typed media item 仅投影字段完整的稳定引用，Strict Matrix 升至 epoch 12 并重生成命令/文档/四份 Frame fixture。G03 聚焦合同 34 passed，`cargo test -p helix-core -p helix-im` 全量通过，五 crate check、Web wasm build、core 零运行时依赖、格式、生成物与 diff check 通过；已从待推送历史移除 canonical-id 并行修复提交，`send/reconcile.rs` 与对应测试相对 origin/main 无 diff | @2026-07-28T18:10:00+0800
⚡ DRIFT-PRED 中途全量测试曾因待推送历史夹带已排除的 canonical-id 提交而出现 1 个失败；移除该提交后全量恢复绿色，本次新增的 4 条媒体 typed-item 回归也全部修复并复验通过 → 候选教训：发布前除看工作树 diff，还必须逐笔核对 origin/main..HEAD 是否仍含已移交他人的提交

🎯 EXPECT 将 G02-G16c 剩余 22 个业务 Gate 按 G01 V1 稳定目录和文档布局同步到 Apifox 项目 8449651；可证伪断言：① 创建或更新恰好 22 个 Gate 文件夹且每个包含总览、Command、PC、Mobile、字段枚举、Frame 状态流、逐 Frame、UI、覆盖、HTTP/WS、Mock、版本 Diff；② 只嵌入 producer 导出的 canonical fixture，缺失 fixture 明确标记 not-proven；③ 所有 payload 先通过 Apifox CLI schema 校验，回读目录归属和数量一致；④ 不修改 Helix/PC/Mobile/Go 业务代码，不把 AcceptedCommand、Mock 或 HTTP 200 冒充真实渲染完成 | 赌注 高 | 可逆 是 | @2026-07-26T20:00:00-0700

🎯 EXPECT 新增消息审批立项前置说明；可证伪断言：① 以阿里 P6 口径明确本期不承诺、现版组织能力不支持、未来规划未确认和核心 Owner 仅组织/消息两方；② 列清需业务产品、三端、测试、安全及运维角色配合，但不把审批 Owner 虚构给第三方；③ 仅在需求、组织能力、责任和排期四项 Gate 通过或管理层明确接受风险后，才进入现有方案决策稿与四份设计文件 | 赌注 低 | 可逆 是 | @2026-07-26T14:35:00-0700

✅ DONE 已新增 106 行立项前置说明，结论先行标明“本期不承诺、方案已预研、依赖和责任未就绪”，拆出需求、组织能力、长期 Owner、可执行排期四个前置问题及对应 Gate；核心 Owner 仍限定组织/消息双方，业务产品、三端、测试、安全和运维仅作为配合角色。文档提供管理层强推时的风险确认项、四份方案跳转及可直接使用的会议开场话术；README 已增加前置 Gate 链接，文件无行尾空白 | @2026-07-26T14:35:00-0700

🎯 EXPECT 将两套消息审批方案改写为阿里 P6 风格评审稿；可证伪断言：① 四个指定文件保持方案边界和事实不变，表达改为结论先行、业务目标、职责、链路、风险与落地动作；② 删除对抗性和空泛架构话术，方案一仍为推荐、方案二仍为需复审的兜底；③ 两张图使用简短泳道和评审术语，Mermaid CLI 解析与目视检查通过 | 赌注 低 | 可逆 是 | @2026-07-26T14:15:00-0700

✅ DONE 四个指定文件已改写为结论先行的 P6 评审口径：两份 Markdown 分别收敛为 114/120 行，统一使用背景目标、核心判断、职责、业务链路、一期范围、合同/模块边界、风险控制和结论；去除对抗性归责，方案一保持推荐并给出并行交付，方案二保持需单独评审的兜底并量化收益与架构成本。两张图改为“组织域/消息域”短泳道和简短动作，Mermaid CLI 解析及 PNG 目视检查通过，四文件无行尾空白 | @2026-07-26T14:15:00-0700

🎯 EXPECT 创建并注册 Gate Contract Specification Agent；可证伪断言：① 新 Agent 只负责自然语言到 source-validated Gate YAML、Strict Matrix requirements 与 Spec 输入，不吞并实施总控、Fixture producer 或终态 reviewer；② Agent metadata、15 个必需章节、两份 schema、模板/示例、manifest/reference/Router/README/diagram 全部机器可解析且示例通过 schema；③ 至少三条合同定义语句命中新 Agent，实施、WDIO、GTest 和最终 Review 仍命中既有 Agent；④ 不修改任何业务生产代码，不运行 Cargo，验证通过后只提交本任务文件和本条 SESSION | 赌注 中 | 可逆 是 | @2026-07-26T20:36:20-07:00

✅ DONE 已创建 Gate Contract Specification Agent，并完成合同/Strict Matrix 两份 Draft 2020-12 schema、可校验模板、明确标注 EXAMPLE 的样例、manifest/reference/Router/README/局部 AGENTS/总图注册；Router 新增完整短语匹配但排除短英文子串，三条自然语义均返回新 Agent，Gate 实施/WDIO/GTest/最终 Review 仍返回既有总控、Mobile domain 与独立 reviewer。Agent metadata、15 个章节、注册路径、required references、JSON/YAML、schema 正反例、模板/示例、Python 语法、Mermaid CLI 和 diff check 全部通过；未修改业务生产代码，未运行 Cargo | @2026-07-26T20:51:14-07:00

🎯 EXPECT 按 Spec 045 串行完成 G16c 群属性修改三仓纵向闭环；可证伪断言：① G16b 保持 `CLOSED`，锁定三仓 HEAD 与 Strict Matrix hash 后再实现；② Helix 独占 displayName/notice/info 的权限、终态、持久化与 `message.channel-summary` Frame V2，PC/Mobile 不新增业务 reducer；③ canonical Fixture 仅由 production `ImModule::handle -> Effect::Emit` 生成且三仓 byte-exact；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码边界、Frame V2、production-boundary 与 R4 全 Green且 SpecKit `CLOSED` 前不进入 GS1；⑥ Cargo 仅在完整 Helix 模块与 Gate 验收集中运行 2-3 次 | 赌注 高 | 可逆 是 | @2026-07-26T18:20:00-0700

🎯 EXPECT 将消息审批材料补成可直接拍板的决策包；可证伪断言：① 新增单页 README 作为唯一评审入口，明确推荐结论、决策请求、方案比较、双方交付、并行阶段和兜底切换闸门；② 推荐方案补齐最小组件合同与并行工作流，不要求组织部门理解消息内部实现；③ 兜底方案保持显式复审、架构债务和后续抽离条件，不得自动生效 | 赌注 低 | 可逆 是 | @2026-07-26T13:50:00-0700

✅ DONE 已新增 124 行单页决策入口，集中给出推荐结论、四项待拍板事项、方案对比、双方交付物、最小组件合同、四阶段并行建设与兜底复审闸门；方案一同步补齐双方独立交付、最小联调合同及“不得自动转消息侧”的停止条件，两份方案均回链决策稿。文档无行尾空白，既有两张架构图仍保持本轮已验证的 Owner 与风险表达；未修改生产代码 | @2026-07-26T13:50:00-0700

🎯 EXPECT 调整两套消息审批评审稿的真实组织边界；可证伪断言：① 方案一明确由组织架构部门建设和维护独立通用审批组件，并标为推荐方案；② 方案二明确为消息系统内建的风险兜底，不再与方案一写成等价优选；③ 四个文件均同步团队 Owner、选择结论和架构债务，Mermaid CLI 解析通过 | 赌注 低 | 可逆 是 | @2026-07-26T13:35:00-0700

✅ DONE 四个评审文件已同步真实组织边界：方案一明确“组织架构部门负责独立通用审批组件、消息部门负责内容与投递”并标为推荐；方案二改为消息系统内建的风险兜底，列出公共能力绑定 IM、重复建设与后续迁移等架构债务及严格启用条件。两张图已通过 Mermaid CLI 解析并完成 PNG 目视检查，文档保持 200 行以内且无行尾空白 | @2026-07-26T13:35:00-0700

🎯 EXPECT 将两套消息审批方案从详细设计收敛为评审稿；可证伪断言：① 两份 Markdown 均压缩为 200 行以内，只保留结论、边界、主流程、关键规则、风险与选择条件；② 两份 Mermaid 均压缩为单屏主链，节点不超过 20 个且可由 Mermaid CLI 解析；③ 不修改生产代码或扩展既有 Gate 状态 | 赌注 低 | 可逆 是 | @2026-07-26T13:05:00-0700

✅ DONE 两份方案已从 686/613 行压缩为 113/129 行，只保留评审所需结论、责任边界、主流程、一期能力、风险和选择条件；两份 Mermaid 从 88/81 行压缩为 46/44 行，改为自上而下的分区主链并减少交叉关系。两图均通过 Mermaid CLI 解析并完成 PNG 目视检查，4 个文件无行尾空白；未修改生产代码或 Gate 状态 | @2026-07-26T13:05:00-0700

🎯 EXPECT 并行产出公司大群消息可见性审批的两套架构方案及各自 Mermaid 真源；可证伪断言：① 仅新增 `docs/specs/2026-07-26-message-visibility-approval/` 下 4 个文档文件并在 SESSION 记录对账，不修改 Rust/Go/客户端生产代码；② 组件版与自研版共用已确认的组织、多任职、多上级、逐节点动态解析、复合 viewers、审批动作、组织变更、摘要所有权与幂等投递不变量；③ 两版明确区分组织架构提供能力与消息系统责任，不再跑偏为 UI 设计器或单纯流程引擎选型；④ `.mmd` 均可由 Mermaid CLI 解析，Markdown 链接和 `git diff --check` 通过 | 赌注 中 | 可逆 是 | @2026-07-26T12:30:00-0700

✅ DONE 已并行产出“组织审批组件接入”与“消息系统自研审批”两套 Markdown 方案及各自 Mermaid 架构真源；两版均区分 CURRENT/TARGET、组织与消息系统边界，并覆盖多任职/多上级、逐节点动态解析、逐级 N、会签/或签、拒绝/退回/撤回/转交、组织变更回查、复合目标统一投递、摘要所有权、权威状态回查与幂等投递。两份 `.mmd` 已通过 Mermaid CLI 实际解析，4 个新增文件均通过 whitespace check；仅修改本任务文档与 SESSION 对账，未运行 Cargo/Go/三端测试，未改生产代码 | @2026-07-26T12:30:00-0700

🎯 EXPECT 按 Spec 042 串行完成 G15 创建话题三仓纵向闭环；可证伪断言：① 前序 G15a 保持 `CLOSED`，先锁定三仓 HEAD、PC/Mobile Helix pin 与 Strict Matrix hash；② Helix 独占 `im_make_topic -> HTTP/WS -> topic channel/root relation persist -> channel-summary/thread-detail Frame V2` 的权限、终态、竞态与恢复语义，PC/Mobile 不新增话题 reducer；③ canonical Fixture 只由 production `ImModule::handle -> Effect::Emit` 生成，PC/Mobile byte-exact mirror 且不补字段；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码边界、Frame V2、production-boundary 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入 G16；⑥ Mobile shared real 的旧 raw CreateGroupPrerequisite 缺口归属后续 GS Gate，不得用来掩盖或替代 G15 required contract | 赌注 高 | 可逆 是 | @2026-07-26T11:38:45-0700

✅ DONE G15a 已完成 Helix、PC、Mobile 三仓纵向闭环并由 SpecKit event 32 `CLOSED`：Helix full-flow 6/6、Fixture 1/1；PC Mock 2/2、report 1/1、single-real 1/1、dual-real 最终 1/1；Mobile production-linked Frame/QuickJS 15/15、首轮 UC15 real 10/10、source pin/hard-cut/no-js-facade/共享 Venus 全绿；三仓三份 Fixture 与 provenance byte-exact。PC 首次 dual-real UCenter 目录超时与 Mobile shared full regression 的旧 raw create 前置/远端超时原文均保留为非 required 证据，不覆盖失败、不移动业务 reducer | @2026-07-26T11:38:45-0700

🎯 EXPECT 按 Spec 041 串行完成 G15a 创建群聊三仓纵向闭环；可证伪断言：① 前序 G14a 保持 `CLOSED`，先锁定三仓 HEAD、PC/Mobile Helix pin 与 Strict Matrix hash；② Helix 独占 `im_create_channel -> HTTP/WS -> channel/channel_member persist -> channel-summary/member-roster Frame V2` 的权限、终态与恢复语义，PC/Mobile 不新增群聊 reducer；③ canonical Fixture 只由 production `ImModule::handle -> Effect::Emit` 生成，PC/Mobile byte-exact mirror 且不补字段；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码边界、Frame V2、production-boundary 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入 G15；⑥ Helix Cargo 仅在完整模块与 Gate 验收集中运行 2-3 次，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-26T08:05:00-0700

🎯 EXPECT 按 Spec 038 串行完成 G13 会话已读三仓纵向闭环；可证伪断言：① G12 保持 `CLOSED`，三仓 HEAD、PC/Mobile Helix pin 与 Strict Matrix hash 先锁定；② Helix 独占 read authority、Persist barrier、participant receipts/channel summary 与完整 Frame V2，PC/Mobile 不新增 unread/receipt reducer；③ canonical Fixture 只由生产 `ImModule::handle -> Effect::Emit` 生成，PC/Mobile byte-exact 镜像且不补业务字段；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码边界、Frame V2、production-boundary 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入 G14；⑥ Helix Cargo 仅在完整模块与 Gate 验收集中运行 2-3 次，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-26T09:26:05Z

🎯 EXPECT 按 Spec 037 串行完成 G12 回复列表/分支三仓纵向闭环；可证伪断言：① 前序 G11 保持 `CLOSED`，三仓 HEAD、PC/Mobile Helix pin 与 Strict Matrix hash 先锁定；② Helix 生产 command/query/WS/persist 链独占 reply thread 的根消息、回复顺序、统计与完整 Frame V2，PC/Mobile 不新增 thread reducer；③ canonical Fixture 只由生产 `ImModule::handle -> Effect::Emit` 生成，PC/Mobile byte-exact 镜像且不补业务字段；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0，并收敛已暴露的 UC12 RealChain `im:messages:query_result` 超时；⑤ 行为、代码边界、Frame V2、production-boundary 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入 G13；⑥ Helix Cargo 仅在完整模块与 Gate 验收集中运行 2-3 次，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-26T07:30:00-0700

🎯 EXPECT 按 Spec 036 串行完成 G11 按 postId 定位三仓纵向闭环；可证伪断言：① 前序 G10a 保持 `CLOSED`，三仓 HEAD、PC/Mobile Helix pin 与 Strict Matrix hash 先锁定；② Helix 删除 `im:post:located`/`im:read:result` raw 成功旁路，以 HTTP 响应中的同 channel 目标为 authority，并在同一 `message.timeline` Frame 原子提交 `targetMessageId`、locate anchor 与 payload-free `revealMessage` directive；③ canonical Fixture 只由生产 `ImModule::handle -> Effect::Emit` 生成，PC/Mobile byte-exact 镜像且不补 reducer；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码边界、Frame V2、production-boundary 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入下一 Gate；⑥ Helix Cargo 仅在完整模块与 Gate 验收集中运行 2-3 次，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-25T23:45:00-0700

🎯 EXPECT 按 Spec 035 串行完成 G10a 上拉更早历史三仓纵向闭环；可证伪断言：① 前序 G10 保持 `CLOSED`，三仓 source pin 与 Strict Matrix hash 锁定后再实现；② Helix 将现有 raw `im:messages:older_loaded` 改为 production `HTTP rounds -> message Persist barrier -> message.timeline Frame V2 patch`，PC 删除 raw-event 二次 query/store，Mobile 只机械 apply typed Frame；③ canonical Fixture 仅由 `ImModule::handle -> Effect::Emit` 生成并在三仓 byte-exact/semantic 一致；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码、Frame V2、production-boundary 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入 G11；⑥ Helix Cargo 仅在完整模块与 Gate 验收集中运行 2-3 次，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-25T22:10:00-0700

🎯 EXPECT 按 Spec 034 完成 G10 切群首屏三仓纵向闭环；可证伪断言：① 锁定 G09 `CLOSED`、三仓 HEAD、PC/Mobile Helix pin 与 Strict Matrix hash 后再实现；② Helix 以生产 `im_query_messages_by_channel -> local scan/conditional HTTP/cache persist -> Effect::Emit(message.timeline)` 为唯一排序、覆盖度、失败和 Frame 真源，PC/Mobile 不补 anchor、page 或 reducer；③ canonical Fixture 只由生产 `ImModule::handle` 回放生成并在三仓 byte-exact/semantic 一致；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ 行为、代码边界、Frame V2 与 R4 全 Green 且 SpecKit `CLOSED` 前不进入 G10a；⑥ Helix Cargo 在完整模块和 Gate 验收集中运行 2-3 次，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-25T21:15:00-0700

🎯 EXPECT 串行完成 G-09 取消定时三仓纵向闭环；可证伪断言：① 先锁定 G08 `CLOSED` 与三仓当前 HEAD，再复用 fresh-group 和 G08 schedule authority，不复制第二套建群或定时 reducer；② Helix 从生产 `ImModule::handle -> Effect::Emit` 生成 G09 canonical Fixture，PC/Mobile 只机械同步和 apply；③ 覆盖取消成功、权限/基础设施失败、重复与迟到事实、恢复，所有 durable 终态都受 persist barrier 约束；④ PC Mock/Real/report 与 Mobile GTest/SDK/Bridge/SurfaceRuntime/QuickJS typed view required lane 全绿并完成三项 Review 后，SpecKit 才允许 `CLOSED`；⑤ 每个独立阶段验证后分类提交，不触碰 Mobile 既有未跟踪 bundle 与 `.DS_Store` | 赌注 高 | 可逆 是 | @2026-07-25T19:35:00-0700

🎯 EXPECT 修复 G09 真实链暴露的 channel-summary ActionBinding 注册缺口；可证伪断言：① G08 scheduled Frame 发出的 enabled `cancelSchedule` 在同一生产 projection 边界登记 actor/surface/scope/epoch/revision lease；② renderer 只回送 opaque actionRef 即可复用现有 `im_cancel_schedule -> HTTP -> WS -> PersistAtomic -> readback -> canceled Frame`，不新增第二套 cancel reducer；③ focused 回归必须从真实 Frame 提取 binding 并走 `__helix_interaction_action_invoke`，断言 accepted receipt、HTTP effect 与最终同 causation Frame；④ direct command 继续兼容，terminal Frame 后 action replay ledger 完成；⑤ 修改限于 helix-im action/surface 编排、G09 Scenario 与证据账本，cargo check/test 在完整修复后集中执行 | 赌注 高 | 可逆 是 | @2026-07-25T20:22:00-0700

🎯 EXPECT 按 Spec 032 完成 G08 创建定时消息三仓纵向闭环；可证伪断言：① Helix `im_create_schedule` 只在 `post_schedule_created` 权威事实的本地持久化读回后发布 `message.channel-summary` Frame，HTTP 受理和 raw projection 均不能建立终态；② `ChannelSummaryVm` 由 Helix 完整表达 owner-only schedule 字段和 `cancelSchedule` 能力，PC/Mobile 不新增 schedule reducer；③ canonical Fixture 只由生产 `ImModule::handle -> Effect::Emit` 生成并在三仓 raw/semantic 一致；④ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；⑤ G08 行为、代码、Frame V2 与 production-boundary Review 通过且 R4 CLOSED 前不进入 G09；⑥ Helix Cargo 仅在完整模块与 Gate 验收各运行一次 | 赌注 高 | 可逆 是 | @2026-07-25T17:30:00-0700

🎯 EXPECT 按 Spec 031 完成 G07 加急与确认三仓纵向闭环；可证伪断言：① Helix `im_urgent_post`/`im_urgent_confirm` 独占权限、HTTP/WS authority、持久终态、receipt 聚合与完整 Frame V2 字段，PC/Mobile 不新增 urgent reducer；② canonical Fixture 只由生产 `ImModule::handle -> Effect::Emit` 生成并在三仓 byte-exact/semantic 一致；③ 每次真实验收创建 fresh group 与 source post，PC Mock/single-real/dual-real/report 和 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；④ G07 行为、代码、Frame V2 与 production-boundary Review 通过且 R4 CLOSED 前不进入 G08；⑤ Cargo 仅在 Helix 模块完成和 Gate 验收运行 2-3 次 | 赌注 高 | 可逆 是 | @2026-07-25T22:31:00-0700

🎯 EXPECT 按 Spec 030 完成 G06 快捷回复 emoji 三仓纵向闭环；可证伪断言：① Helix `im_quick_reply` 独占权限、HTTP/WS authority、持久终态和完整 Frame V2 字段，PC/Mobile 不新增 reaction reducer；② canonical Fixture 只由生产 `ImModule::handle -> Effect::Emit` 生成并在三仓 byte-exact/semantic 一致；③ 每次真实验收创建 fresh group 与 source post，PC Mock/single-real/dual-real/report 和 Mobile production-linked GTest/QuickJS typed view required fail=0/skip=0；④ G06 行为、代码、Frame V2 与 production-boundary Review 通过且 R4 CLOSED 前不进入 G07；⑤ Cargo 仅在 Helix 模块完成和 Gate 验收运行 2-3 次 | 赌注 高 | 可逆 是 | @2026-07-25T14:24:00-0700

🎯 EXPECT 按 Spec 028 完成 G04a 离线撤回重放三仓纵向闭环；可证伪断言：① `im_sync_channels` 只接受 channel intent，cursor 与 sync/notify authority 由 Helix `sync::explicit`/`sync::flow` 决定，不在 PC/Mobile 恢复 raw query reducer；② Helix ScenarioRunner 从生产 PortReply 的 viewer-filtered sync batch 经过 PersistAtomic/PersistOk 生成 recalled Frame，并覆盖 failure/duplicate/late/recovery；③ canonical Fixture 在三仓 byte-exact/semantic 一致，PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view 全绿；④ G04a 通过行为、代码、Frame V2、production-boundary 与 R4 CLOSED 前不进入 G05；⑤ 每次验收创建 fresh group，不复用 G04 channel/post/revision | 赌注 高 | 可逆 是 | @2026-07-25T10:57:24-0700

🎯 EXPECT 续跑 Spec 027 完成 G04 在线撤回三仓纵向闭环；可证伪断言：① Mobile source pin 固定到包含 G04 canonical Fixture 的 Helix 正式提交并以 bundle/hash 复现，不读取 dirty worktree；② Mobile mirror 与 Helix raw/semantic/provenance 全量一致，GTest 链接生产 SDK/Bridge/SurfaceRuntime/QuickJS binding 并只通过 `im.surface.view` 验证 typed view；③ PC 已有 Mock/single-real/dual-real/report 证据保持有效，三仓不新增第二套业务 reducer；④ G04 必须通过行为、代码、Frame V2、production-boundary 与 R4 Review 并 CLOSED 后才进入 G04a；⑤ Mobile repin、Fixture、合同实现分别验证并分类提交，不纳入既有无关 bundle/.DS_Store | 赌注 高 | 可逆 是 | @2026-07-25T10:23:58-0700
✅ DONE G04 已完成 T001-T038 三仓纵向闭环并由 helix-loop 最终 CLOSED：Helix 6/6 full-flow、4/4 authority/persist、1/1 canonical Fixture；四份 Frame 与 provenance 在 Helix/PC/Mobile byte-exact/semantic 100% 一致；PC Mock 2/2、single-real 1/1、dual-real 1/1、report/boundary PASS；Mobile production-linked G04 5/5、UC05 non-real 7/7、source-pin/ABI/V1 hard-cut/共享 Venus PASS。行为、代码边界、Frame V2 与 evidence Review 均 Green，required fail/skip=0。Mobile real harness 额外观察在既有 GoLand 8066 进程上于 revoke 前 remote readback 超时，已作为非 required blocker 单列，未恢复 V1/raw reducer。总控 T005 完成，仅解锁 G04a | @2026-07-25T10:52:55-0700

🎯 EXPECT 修复 G03a Mock fresh-group 后缺失 timeline attachment 的真实合同；可证伪断言：① Helix `ImModule::handle` 额外导出建群后的空 timeline snapshot canonical Fixture，不由 PC 手写初始 revision；② G03a provenance/mirror inventory 从 7 增至 8 且 raw/semantic hash 全量一致；③ PC Mock 在每例 fresh group 上先 replay 该 snapshot，再执行 failed→retry sending→sent/failed；④ Helix focused fixture test、PC parity/typecheck/Mock lane 全绿 | 赌注 中 | 可逆 是 | @2026-07-25T06:30:00-0700

🎯 EXPECT 按 Spec 026 完成 G03a 重发失败三仓纵向闭环；可证伪断言：① retry 必须复用原 temporaryId/原消息体，只在 Helix 处理 durable lookup、terminal authority、竞态和 recovery，PC/Mobile 不新增 retry reducer；② canonical Fixture 仅由 `ImModule::handle -> Effect::Emit` 生成并在三仓 byte-exact/semantic parity；③ PC Mock/single-real/dual-real/report 与 Mobile production-linked GTest/QuickJS typed view 分层全绿；④ 每次真实验收先创建新群，不复用 G02 遗留数据；⑤ G03a 仅在行为、代码边界、Frame V2 与 R4 全 Green 后解锁 G04，Cargo 本 Phase 控制在最终 check/test 2-3 次 | 赌注 高 | 可逆 是 | @2026-07-25T05:42:00-0700

🎯 EXPECT 使用 Spec Kit 为 020 总控与 25 个 Active 子 Feature 生成可执行 Plan/Task；可证伪断言：① 每个 Feature 具备 plan、research、data-model、contracts、quickstart、tasks，并通过 R2/R3 mandatory loop；② 子 Task 明确覆盖 Helix 完整场景/Fixture/Golden、cses-client WDIO Mock/Real/断言/报告、Mobile mirror/GTest/生产 SDK-Bridge-SurfaceRuntime/QuickJS typed view 与跨仓 Review，且路径精确；③ 总控只编排 G01→G02 试点→G03a→G04-G16c→GS1-GS4，不复制各 Gate 业务实现；④ 新增机器和人读 Plan/Task 索引，支持按 Gate 进入而不全量加载；⑤ 26/26 R2、26/26 R3、任务格式/合同表/链接/JSON/脚本语法与 `git diff --check` 通过，不改三仓生产代码、不把计划冒充测试 Green | 赌注 高 | 可逆 是 | @2026-07-25T00:33:06-0700
✅ DONE 已为 020 总控与 25 个子 Feature 生成完整 plan/research/data-model/contracts/quickstart/tasks，并通过 Spec Kit setup-plan/setup-tasks 与 mandatory before/after hooks：26/26 R2 PASS、26/26 R3 PASS，全部停在 `ready/admit_implementation`。每个子 Gate 固定 38 个任务，覆盖 Helix 六类场景、生产 handler、完整 Frame、Fixture/Golden/provenance，PC Fixture parity/Mock/single-real/dual-real/DOM/report/boundary，Mobile mirror/GTest/生产 SDK-Bridge-SurfaceRuntime/QuickJS typed view/hard-cut，以及行为/代码/Frame V2/R4 Review；总控 32 个任务只做顺序、证据与用户控制删除。新增机器/人读 Plan-Task 索引、UC 文档入口、幂等生成器和 fail-closed 校验器；25×38+32=982 个新增任务，连同 G01 既有 70 个任务形成 1052 个按需执行入口。生成幂等 hash 稳定，Plan/Task/contract/link/JSON/ledger、文档库存与 `git diff --check` 全部通过；本轮未执行实现任务或三仓业务测试，不宣称任何新 Gate Green | @2026-07-25T00:45:35-0700
⚡ DRIFT-PRED 在 R3 已 PASS 后额外调用一次 `advance` 会进入 before_implement 并立即要求 R4 evidence，而不是“重复确认 R3”；总控因此短暂 BLOCKED，随后通过真实 Task fingerprint 变更让 loop 回到 R3 并重新 PASS → 候选教训：批量 Spec Kit hook 必须按 `next_action` 停止，达到 `admit_implementation` 后不得盲目再 advance | @2026-07-25T00:45:35-0700

🎯 EXPECT 使用 Spec Kit 建立 InteractionFrame V2 三仓逐 UC 总控与子规格；可证伪断言：① 总控精确登记 26 个 Active Gate，G01 复用已完成基线，新增 25 个独立 Gate Spec，G03 与 G17-G27 只进入 Gap；② 每个子 Spec 固定 Helix canonical Fixture 唯一真源、PC Mock/Real/Report、Mobile GTest/生产 SDK-Bridge-SurfaceRuntime/QuickJS typed view 与 production-boundary；③ 一个 UC 三仓闭环并通过行为、代码、Frame V2 一致性 Review 后才能进入下一个；④ `docs/interaction-frame-v2-全量替换-按UC` 新增总索引并能逐 Gate 链到 Spec Kit Feature；⑤ requirements checklist、索引/JSON/相对链接、mandatory after_specify loop hook 与 `git diff --check` 通过，不修改生产代码或运行三仓业务测试 | 赌注 高 | 可逆 是 | @2026-07-25T00:00:00-0700
✅ DONE 已建立 `020` 三仓逐 UC 总控和 `021-045` 共 25 个独立子 Feature，连同 G01 基线精确覆盖 26 个 Active Gate；G03 与 G17-G27 共 20 个 Gate 仅进入 Gap。每个 Feature 均包含 spec、discovery、requirements checklist、verification 与 R1 ledger，固定 Helix canonical Fixture 唯一真源、PC Mock/Real/Report、Mobile 生产 SDK/Bridge/SurfaceRuntime/QuickJS typed view、production-boundary 和三类一致性 Review。新增可重复生成脚本、机器 manifest、人读索引与逐 UC 文档总入口；G02 明确为固定 URL 文档卡片并标记 Strict Matrix `upload` 漂移，不混入文件上传。26/26 mandatory after_specify R1 PASS，文档库存/本地链接校验、生成器与刷新器语法、manifest/count、`git diff --check` 通过；本轮只产出规格与治理资产，未实现业务代码、未运行 Cargo/WDIO/Mobile runtime，下一步为逐 Feature `/speckit-plan` | @2026-07-25T00:20:48-0700
⚡ DRIFT-PRED mandatory loop 的 Source Lock 只接受单个反引号包裹值，而首次生成使用三段独立反引号导致总控 R1 BLOCKED；已统一为 `helix=...;cses-client=...;mobile=...` 并使 26 个 Feature 全部通过 → 候选教训：批量生成 Spec 前先用一个样本跑 mandatory hook，再扩展到全部 Feature | @2026-07-25T00:20:48-0700

🎯 EXPECT 审计并分类提交 Helix 剩余 Gate01 与 Agent 路由改动；可证伪断言：① 不移动 `0724-Gate01-V2-Basic` tag；② 修正会删除已完成 G01 WS/六份 Fixture 真源或记录过时 HEAD/pin 的文档漂移；③ Gate Router 返回存在的 Agent 与 required reference，新增领域具备就近边界和图谱；④ 仅提交验证完整的 Agent 治理、G01 文档与 SESSION 账本，不混入未知改动；⑤ JSON、Python、Router、路径引用与 `git diff --check` 全部通过，本批无 Rust 改动不运行 Cargo | 赌注 中 | 可逆 是 | @2026-07-24T23:05:08-0700
✅ DONE 已分类提交 `801c66f` Agent 路由治理与 `8544b7f` G01 六帧文档对齐：新增 Gate 总控、Frame producer、Fixture supply chain、跨端 reviewer、就近边界和领域图谱，Router 能返回存在的 Agent/required references；移除已下线 takeover-plan 失效引用。G01 文档恢复 WS timeline/dialog 独立 correlation 与 Golden producer，并补入 group-created 形成六份库存，更新 tagged baseline、最新 Mock/single/dual real、50 字符 simpleMessage 与 unread 基线语义。Python/JSON/manifest graph/代表性 Router/provenance inventory=6/本地链接/diff check 全部通过；本机无 mmdc，Mermaid 只做文本结构审计；无 Rust 改动未运行 Cargo。`0724-Gate01-V2-Basic` 仍指向 `dbe26ec`，未移动、未新增 tag、未 push | @2026-07-24T23:17:00-0700
⚡ DRIFT-PRED 剩余文档不是可直接提交的增量：其中一版会删除已完成的 G01 WS/Fixture 真源，Agent manifest 还引用不存在的 Gate 根 AGENTS 与已删除 takeover-plan；审计后先恢复代码事实并补齐 domain boundary/diagram 才提交 → 候选教训：清理脏工作树必须按当前 producer inventory 和路径存在性做反向验证，不能把“剩余改动”默认视为正确 | @2026-07-24T23:17:00-0700

🎯 EXPECT 沉淀可复用的 Helix-PC-Mobile Gate V2 Agent 编制；可证伪断言：① 三仓新增 9 个职责无重叠、包含 YAML 元数据与完整工作/失败/输出/成功合同的 Agent；② Helix 负责 Gate 总控、Frame producer、Fixture supply chain 与跨端裁决，cses-client 负责 PC Gate、真实链与证据报告，Mobile 负责 Gate contract 与 V2 runtime；③ 三仓就近 AGENTS.md、agents/domains manifest、Router、required references 同步且路由命令能返回新 Agent；④ 不新增用户本轮暂缓的验收需求，不覆盖既有脏改动；⑤ JSON、文件存在、必需章节、路由矩阵与 diff check 全部通过 | 赌注 高 | 可逆 是 | @2026-07-24T22:10:00-0700
✅ DONE 已在三仓落地 9 个完整 Agent：Helix 总控/Frame producer/Fixture supply chain/跨端 reviewer，cses-client PC Gate/real-chain/evidence-reporting，Mobile Gate contract/V2 runtime；同步新增 10 个就近 AGENTS.md、三仓 agents/domains manifest、8 条 Router 规则与 required references。机器验证覆盖 YAML 元数据、10 个必需章节、Agent 文件存在、route agent/ref 可解析、JSON、Python Router 语法、代表性 G02 路由矩阵和三仓 `git diff --check`，全部通过；Mock 与 real 路由已验证不会因同属 specs 目录而错误混用。用户本轮暂缓的额外验收面未加入。因三仓 manifest/AGENTS 与业务工作树已有其他任务脏改动，本轮未自动 commit，避免把非本任务改动混入提交 | @2026-07-24T22:19:46-0700
⚡ DRIFT-PRED Helix 与 Mobile 的 repo-local `agentctl.py` 原先不会把 route 中声明的 `required_references` 带入路由结果，而 cses-client 已具备该能力；已补齐两仓并用代表性 G02 路由证明引用实际返回 → 候选教训：新增 Agent 不只要校验 manifest 存在，还必须执行真实 route 并断言 required references 出现在输出 | @2026-07-24T22:19:46-0700

🎯 EXPECT 将“444/678 新建群聊”提升为 G01 三端验收的 canonical V2 前置，并全面硬切 Mobile 可达 Frame V1 producer/consumer；可证伪断言：① Helix ScenarioRunner 真实回放 `im_create_channel` 的 HTTP→权威 persist/readback→Frame，新增字段级 Golden，且后续 sending/sent/failed/WS Fixture 使用同一 `channelId`；② cses-client Mock 先 replay Helix 建群 Fixture，single/dual-real 由 UI 选择 444/678 新建群后进入该群发送，不再依赖既有 dialog；③ Mobile QuickJS/GTest/Venus 先消费同源建群 Fixture再发送，production 可达路径中 Frame V1 writer/reader 与 raw business reducer fallback 均为 0，ActionInvocation schema1 不受影响；④ 三仓分层验证真实执行，环境阻塞如实报告，不以 Mock 冒充 real；⑤ 不覆盖三仓既有脏改动，所有新增/修改方法均有紧邻精简注释 | 赌注 高 | 可逆 是 | @2026-07-24T22:05:00-0700
✅ DONE Task 1/2 已闭合本地实现与分层验证：Helix 先以 actor 隐式 owner、仅邀请 678 的 `im_create_channel` 取得权威 `channelId`，新增 `group-created.frame-v2.json` 并让六份 Fixture 共享该群；full-flow 4/4、Golden 1/1。cses-client Mock 4/4 先 replay 建群 Frame，single-real 1/1 与 dual-real 2/2 均由 MessageV4 UI 新建群、显式选择 678，再在返回群发送。Mobile production 编译已移除 `im.store`、`mobile_vmstore_*`、ViewModelStore row patch API 与 raw reducer 符号，旧路径仅在显式 characterization 宏下；G01 C++/QuickJS 5/5 和 Venus Gate 都先提交建群 command、apply group-created、再向同一 channelId 发送。三仓 diff check 通过；因三仓存在其他任务脏改动且当前 cses-client runtime pin 仍为 ee844，本轮未自动提交或宣称最终 canonical Green | @2026-07-24T22:38:00-0700
⚡ DRIFT-PRED 建群成功处理并没有独立 PersistOk/readback barrier；当前代码以权威 HTTP response 在同一 `ImModule::handle` step 产出 channel/member `PersistFire`、Dialog Frame 与 `im:channel:created`。另发现真实 UI/Helix 合同中 actor 是隐式 owner，`user_ids` 只能提交 678，最初双 ID 请求断言已纠正 → 候选教训：前置 authority 的持久化屏障和成员请求语义必须按具体 handler 求证，不能从发送消息 P1 barrier 或最终 roster 反推 | @2026-07-24T22:38:00-0700

🎯 EXPECT 将 G01 扩展为 Helix、cses-client、Mobile QuickJS 三端 Gate01 验收合同；可证伪断言：① 先以 Venus/QuickJS/C++/FFI 当前代码证明发送与左侧群聊最后消息更新全链，不靠历史文档推断；② Mobile repin 到本次 Helix producer，并只消费五份 provenance/hash 可验证的 V2 Fixture，不在 JS/C++ 补算业务 VM；③ 删除 G01 可达链中的 schema1、legacy/raw fallback，schema1/raw 输入 fail-closed；④ Helix producer、PC Mock/Real、Mobile fixture/runtime/ack 分层报告，任一真实链前置缺失明确 blocked；⑤ 不覆盖三仓既有脏改动，新增/修改方法均有紧邻精简注释，聚焦 gate 与 diff check 通过 | 赌注 高 | 可逆 是 | @2026-07-24T20:31:00-0700
✅ DONE G01 已形成 Helix、cses-client 与 Mobile/Venus 的同源 V2 合同：Helix full-flow 4/4、Golden 1/1；PC 五份 Fixture parity 与 76/76 合同测试通过；Mobile 五份 byte-exact mirror、source pin、V2-only hard-cut、47/47 SurfaceRuntime（含显式 pin parity）与 Venus 真实 batch ingress 均通过。Venus 现在把 `im:interaction:frame` 经 C ABI SurfaceRuntime 原子 apply/ack，再仅通过 `im.surface.view` 暴露 typed VM，并切断四类 raw business event 到 JS；历史 V1 Fixture 已恢复为零引用存档，等待全部 Gate 完成后统一删除。cses-client Native Mock 复查确认 embedded provider 可启动，外部 `tauri-driver` 缺失仅是非阻断诊断；当前真实阻塞为清理后的 actor profile 无 `data-dialog-id`，无法建立 active channel。real invoke/single/dual 本轮未重跑，不升级为 canonical Green；三仓既有脏改动均未覆盖，diff check 通过 | @2026-07-24T21:02:00-0700
⚡ DRIFT-PRED 最初把“G01 运行时 V2-only”误扩张为立即物理删除共享历史 V1 Fixture；用户重新明确全局策略后已恢复该资产，并把闸门改为“历史可留、已迁移路径零引用、全部 Gate 后统一删除”。另发现 Venus 既有 CMake artifact 目录与 include 路径不一致，导致首次 Gate 在 `cses_im/helix.hpp` 阻断，已统一到打包产物 `helix.hpp` → 候选教训：逐 Gate 硬切必须分别审计可达引用和资产生命周期，不能用全仓文件存在性代替 runtime reachability | @2026-07-24T21:02:00-0700

🎯 EXPECT 新增 G01 WDIO 完整验收 handoff，准确冻结 Helix producer、cses-client IPC Mock、真实 invoke、444/678 双实例、Frame/FrameApplied/DOM、日志与阶段化报告的职责边界；可证伪断言：① handoff 明确从 UI 创建新群而非复用共同频道；② 固定 prerequisite.* 与 g01.* 报告阶段及 canonical Green 公式；③ 区分当前已提交事实、未验证脏改动和待补缺口；④ 给出可直接执行的双仓验证顺序；⑤ 仅新增 G01 handoff 并追加本 EXPECT，不覆盖现有改动 | 赌注 中 | 可逆 是 | @2026-07-24T19:48:18-0700
✅ DONE 已新增 G01 跨仓验收 handoff：固定 UI 新建 444/678 群聊前置、五份 Helix canonical Frame、Mock/real/dual 职责边界、11 个阶段化报告 ID、逐阶段断言、日志/附件、canonical Green 公式、当前双仓 pin 与脏改动边界、实施顺序及双仓验证命令；`git diff --check` 通过，未运行 Cargo/WDIO，未提交以避免混入主线程正在进行的 SESSION 与 cses-client 重构 | @2026-07-24T19:51:00-0700

🎯 EXPECT 将 G01 完整 Frame 合同收敛为 Helix producer、cses-client WDIO Mock consumer 与真实链三层验收；可证伪断言：① `g01_full_flow_test` 与 `g01_wdio_fixture_contract_test` 共享唯一 `G01ScenarioRunner`，现有 sending/sent/failed 文件名和普通运行只读 Golden 语义保持不变；② 新增 WS timeline/dialog canonical Fixture，证明两条独立 correlation、late HTTP 幂等和 dialog 条件产生，不固定第三帧顺序；③ cses-client 同步脚本、provenance/schema、loader、hash/parity、TauriMockBridge 与 WDIO Mock 全部从 Helix Fixture 消费，不手写业务 VM；④ Helix 聚焦测试及 cses-client 聚焦合同/Mock 验证通过；⑤ 真实链若缺鉴权、双账号、Go/PG/WS 或可观察 DOM 前置则明确 blocked，不用 Fixture/Mock 冒充 | 赌注 高 | 可逆 是 | @2026-07-24T19:12:35-0700
✅ DONE G01 完整 Frame 合同已收敛为共享 `G01ScenarioRunner` 唯一 producer、五份 canonical Fixture、cses-client fail-closed 同步/typed loader/Tauri bus replay 与 Native Mock DOM 验收；Helix check、完整流程 4/4、Fixture Golden 1/1，client 合同 16/16、Native Mock 4/4、single-real 1/1、dual-real 2/2 PASS。WS timeline/dialog 保持独立 correlation，late HTTP 不重复终结；真实链已证明双账号 timeline 与 FrameApplied 关联，但 marker 对应左侧摘要/时间尚无独立采集字段，文档明确保留缺口 | @2026-07-24T20:03:00-0700
⚡ DRIFT-PRED 真实 Fixture 消费暴露三处原预判外合同缺口：DialogLeft 权威时间此前错误读取顶层字段、跨 case canonical message/transaction ID 会碰撞、运行态 actor/team scope 未声明参数槽；已分别在 producer、provenance 与 loader 层修正并新增字段级断言 → 候选教训：Golden 跨 case 回放必须同时验证时序字段、身份槽与 correlation 唯一性，不能只验 schema/hash | @2026-07-24T20:03:00-0700

🎯 EXPECT 以当前代码与 Helix/cses-client 单元测试、IPC Mock 为证据，重建 G01 普通文本发送的字段级 V2 全流程；可证伪断言：① 排清 direct command、ActionBinding、Tick、Effect、Persist barrier、PendingSend、SurfaceRuntime、InteractionFrameV2 的真实先后和依赖方向；② 给出 command/receipt/HTTP/WS/SQLite/Frame/VM/action/directive 到字段级定义；③ 对用户提出的 sending、sent、messageDialogLeft 三个 Frame 逐一证明 CURRENT 是否真实存在，不把 TARGET 或 IPC fixture 冒充 producer；④ 完整类图与时序图能链接到双仓源码和测试；⑤ 只修改 G01 文档、生成/校验工具和 SESSION，不改生产代码，校验与 `git diff --check` 通过 | 赌注 高 | 可逆 是 | @2026-07-24T17:05:00-0700
✅ DONE G01 已恢复并升级为 12 份当前代码文档：新增 312 行全领域类图、110 行全链时序图、85 行状态/三 Frame 纠错图和 562 行字段/测试证据合同。源码对账确认普通发送是 direct `im_send_message` 而非 `message.create` ActionBinding；direct producer replay 固定输出 Timeline sending/sent（失败分支为 failed），但可 apply 的 WS `post` 还会经 `DialogPatchByPost` 写后 `Get(channel)` 条件性输出 DialogLeft，并在 unread contribution 变化时同帧配对 Unread；该 Frame 与 HTTP sent 独立 correlation，不能硬排成固定第三帧。验证：Helix producer 1/1、Angular Shell/SurfaceRuntime 33/33、WDIO IPC Mock 3/3、Mermaid 3/3、18×8+G01×12/25 Gate 索引和 `git diff --check` 均 PASS；未运行 Go、Native 单/双账号真实链 | @2026-07-24T17:28:00-0700

🎯 EXPECT 修正 `629e33b` 对逐 UC 细粒度文档的过度收敛；可证伪断言：① 19 个 UC 除 README 外恢复当前类图、当前执行流程、用户交互流程、Angular、Helix、Go/WS、PG/SQLite 共 7 个按面文档；② 只恢复并按当前代码生成 V2/current 文档，不恢复任何 Frame V1 图、V1 流程或 V1 清退页；③ README 成为 UC 内索引而不是信息终点；④ 刷新器与校验器锁定 19×8 文件、25 Gate、全仓旧 V1 路径为 0；⑤ 不修改 Rust/Angular/Go 生产代码，不覆盖 SESSION 与 019 既有脏改动 | 赌注 高 | 可逆 是 | @2026-07-24T16:25:00-0700
✅ DONE 已纠正 `629e33b` 的过度收敛：19 个 UC 现在各有 README 索引、当前类图、当前执行流程、用户交互流程、Angular、Helix、Go/WS、PG/SQLite 共 8 份文档；这些内容由当前 strict matrix、UC 入口模式、双仓源码路径和专属测试机械生成，不恢复 V1 或把历史 TARGET 冒充 CURRENT。结构为 19×8 + 7 根文档 = 159 文件，25 Gate、T021-T077、159 文件链接、退役 V1/旧 target/逐 UC Spec-Plan-Tasks 文件为 0、`git diff --check` 均 PASS；未运行生产代码测试或 E2E | @2026-07-24T16:31:00-0700

🎯 EXPECT 以当前 Helix 与 cses-client MessageV4 源码逐 UC 重建 `docs/interaction-frame-v2-全量替换-按UC`；可证伪断言：① 直接证明 Helix Frame producer 与 cses-client Native/Angular consumer 是否仅剩 V2，不引用旧结论代替源码；② 19 个业务 UC 文件夹逐一对齐真实 command/handler/Persist barrier/Surface/Angular 组件与状态，并保留 25 个机器 Gate 的映射；③ 若仓内无可达 Frame V1 type/codec/writer/reader/fixture，则删除各 UC 的 `01/03/10` V1 历史页及根级 V1 下线文档，修复全部索引和相对链接；④ 不修改 Rust/Angular/Go 生产代码，不覆盖当前 SESSION 与 019 规格脏改动；⑤ 文档结构、源码链接、V1 残留、19 UC/25 Gate、相对链接与 `git diff --check` 机器检查通过 | 赌注 高 | 可逆 是 | @2026-07-24T15:00:00-0700
✅ DONE 已从当前 Helix HEAD、cses-client HEAD 与客户端真实 Helix pin 三处源码确认 Frame producer/Native sink/Angular codec/runtime 均为 V2-only；19 个业务 UC 已按当前 strict matrix 的 25 Gate 重建为单 README 入口，逐项记录真实 direct command/ActionBinding/inbound 入口、Effect、persistence 合同、canonical Surface、MessageV4 组件和专属测试。物理删除各 UC 的 Frame V1 图/流程/清退页及重复 V2 目标图、Spec/Plan/Tasks，旧 G17 提案退出本目录；独立保留 ActionInvocation/strict evidence 等非 Frame V1 版本域。刷新器同时核验客户端统一 pin、pin 提交 V2-only、双仓生产树 V1 token=0、19 个 Helix/client 专属测试路径和 25 Gate；索引校验、28 份 Markdown 相对链接、失效旧路径扫描与 `git diff --check` 全部 PASS。仅做静态源码与文档验证，未运行 Cargo/Angular/Native/Go E2E | @2026-07-24T16:20:00-0700
⚡ DRIFT-PRED 物理删除切换期文档后发现 018 索引、专用校验器与 019 discovery 仍反向引用旧文件，因此实际额外对齐这三个引用点；原先只扫描目标目录不足以保证全仓无断链 → 候选教训：文档删除事务必须先做全仓反向引用扫描，校验器应约束当前文档形态而不是冻结迁移期文件数量 | @2026-07-24T16:20:00-0700

🎯 EXPECT 调用 Spec Kit 为 G01 WDIO 前五阶段建立独立 Feature Spec；可证伪断言：① 新 Spec 明确分离 L0 安全前置、单实例 UI/Helix-derived V2 Mock、真实 Tauri/Helix、G01 raw-driver 等价迁移、444/678 双账号真实链，且不把 Mock/receipt/单端 integration 冒充 canonical Green；② 覆盖 console + Tauri/Helix 日志、断言、截图、测试专用权限、凭据脱敏、SQLite/PG/Go/WS/Frame/DOM 证据和第一版 Spec+JUnit+summary 报告；③ 以常驻拓扑、按需实例、事件 latch、O(1) correlation 索引和失败升级采集约束效率，单 Mock <10s、单端 G01 P95 <15s、双端 G01 P95 <30s；④ Spec 质量 checklist 全绿并执行 mandatory after_specify loop hook；⑤ 本轮只产 Spec/Checklist/loop discovery，不修改 WDIO、Rust、Angular、Go 生产或测试实现 | 赌注 高 | 可逆 是 | @2026-07-24T07:43:23-0700
✅ DONE Spec Kit feature `019-g01-wdio-five-phases` 已生成：5 个用户故事、35 条 FR、12 条 SC 和 8 个未来验证合同覆盖 L0 安全边界、单实例 UI/Helix-derived V2 Mock、IPC reject、真实 Tauri/Helix、G01 raw-driver 等价迁移、444/678 双账号、console/Tauri 日志、断言、权限、脱敏和 Report V1；结果层级固定为 ui-only/mock/integration/single-real-chain/dual-real-chain，Phase 5 只证明 G01 接入恢复 Harness 而不冒充 GS Green。requirements checklist 16/16，placeholder/clarification=0，mandatory after_specify hook 首轮因 discovery 模板阻断后已按源码补齐 12 Facts/3 Unknowns/7 Decisions，R1 PASS、next=`run_speckit_plan`；未修改或运行 WDIO/Rust/Angular/Go 实现与 E2E | @2026-07-24T07:47:59-0700
🎯 EXPECT 为 `019-g01-wdio-five-phases` 依次生成 Spec Kit Plan 与 Tasks；可证伪断言：① Plan 产出 `research.md`、`data-model.md`、`quickstart.md`、WDIO 验收/fixture provenance/Report V1 合同，并明确 Helix fixture 真源、cses-client 测试专用权限、常驻拓扑、事件 latch、O(1) correlation、console+Tauri 日志和五阶段退出闸门；② Tasks 按 US1-US5 tests-first 分阶段，使用唯一连续 Task ID、精确双仓路径、显式依赖与 Loop Task Contract；③ G01 迁移只切断自身 raw WebDriver 调用，不在其他 Gate 尚未迁移时误删共享旧 driver；④ mandatory after_plan/after_tasks 均 PASS；⑤ 本轮仅修改 019 规格工件与 SESSION，不修改 Helix/cses-client/Go 生产或测试实现 | 赌注 高 | 可逆 是 | @2026-07-24T07:56:11-0700
✅ DONE `019-g01-wdio-five-phases` 已依次生成 Plan 与 70 项 Tasks：Plan 固定 6 个 Inputs、7 个 Actions、7 个 States、8 个 Validators 和五阶段退出闸门；新增 12 项 research 决策、9 个运行/证据实体、Quickstart、G01 lane/correlation/failure/timing 合同、fixture provenance 与 Report V1 JSON Schema、日志 allowlist。Tasks 为 T001-T070 连续唯一、70/70 Loop Task Contract 映射，按 Setup/Foundation/US1-US5/Closure tests-first 排序，并明确 G01 raw=0 但其他 Gate 未迁移时保留共享 driver。JSON、placeholder、task/row 连续性、`git diff --check` 均 PASS；mandatory R2/R3 分别 event 4/5 PASS，Feature phase=`ready`、next=`admit_implementation`。本轮未修改或运行 Helix/cses-client/Go 生产/测试实现与真实 E2E | @2026-07-24T08:04:13-0700
🎯 EXPECT 调用 Spec Kit implement 执行 `019-g01-wdio-five-phases` 的 T001-T070；可证伪断言：① Helix 只新增 G01 Frame V2 fixture 真源/contract，不引入 WDIO/Tauri 依赖；② cses-client 新测试能力由 additive test-only feature/capability/config 隔离，default/production build 静态与运行入口为 0；③ Mock receipt 与 `im:__bus__` Frame replay 分离，G01 UI/mock/integration/single-real/444-678 dual-real 均有独立证据等级；④ G01 raw W3C 直接调用归零但其他 Gate 仍使用时不删除共享 driver；⑤ correlation 平均 O(1)、无 fixed sleep、日志 allowlist、Report V1 和 warm P95 合同可机器验证；⑥ 每个完整 Phase 验证后分批提交，Rust Cargo 每 agent/phase ≤2-3 次；⑦ 无法获得真实权限/认证/PG/Go/双端证据时保持 blocked，不伪报 R4 | 赌注 高 | 可逆 是 | @2026-07-24T12:05:48-0700
⚡ DRIFT-PRED G01 single/dual real lane 已闭合 Actor/Observer UI、本地 SQLite、Frame 与 fresh 双截图，但 staging `mattermost` 实际写库和 keychain CNPG 只读真源不一致，fresh marker 在目标 `public.post` before/after 均为 0；同时首次双端截图虽 DOM 断言通过却未把 marker 滚入视口，现已把 marker 聚焦、viewport 和 SHA-256 索引纳入截图事务 → 候选教训：真实链数据库证据必须先验证服务写入 DSN 与只读证据 DSN 同源，视觉证据必须断言目标对象在当前 viewport 而非只断言 DOM 存在 | @2026-07-24T14:21:00-0700
✅ DONE `019-g01-wdio-five-phases` 的 T001-T070 已全部完成；本地 Go `127.0.0.1:8066` 下 UI、Helix 真源 V2 Mock、real invoke、single-real 与 444/678 dual-real 均 PASS，Actor/Observer DOM、SQLite、只读 PG、Go HTTP/WS、Frame/FrameApplied、截图与 Report V1 完整关联。最终性能 P95 为 UI 1194ms、Mock 1667ms、single 3085ms、dual 19311ms，全部低于预算且 profile restart=0；G01 raw reference=0，非 G01 共享 driver 保留。Helix fixture contract 1/1、WDIO contract 21 suites/61 tests、production boundary、失败报告脱敏与双仓 diff check 均 PASS；V-001 至 V-008 已登记，loop R1-R4 重校验后于 event 31 `CLOSED` | @2026-07-24T17:10:00-0700
⚡ DRIFT-PRED 最终 V-008 首次把真实失败报告纳入扫描后，发现 WDIO 官方 JUnit 会把绝对源码路径写入 XML；新增 JUnit 收尾脱敏器和 fresh failure proof 后才闭合。另一次 V-004 因调用进程未继承只读 PG DSN 而非数据库不可达失败 → 候选教训：真实链 final command 必须显式登记安全环境注入，artifact security gate 必须包含至少一个真实失败输出而不能只扫 Green run | @2026-07-24T17:10:00-0700

🎯 EXPECT 先以 G01 发文本建立 UC01-16/GS01-04 可复用的字段级设计基线；可证伪断言：① 新增 G01 全领域类图、全链时序图、发送状态图与字段真源表，覆盖 Angular/Tauri/Helix/PendingSend/本地 SQLite/Go HTTP+WS/远端 PG/InteractionFrame V2/Timeline/DialogLeft/Unread/Composer/DOM；② 明确 CURRENT 当前 G01 producer 只提交 `message.timeline`，DialogLeft/Unread/Composer 仅作为受影响领域和 TARGET，不伪造已落地多 Surface 原子帧；③ 每个字段标记 owner、来源、落点、验证方法，并给出后续 Spec/Plan/Task/Step/Review/Checklist 的强制输入；④ 只改 G01 文档、索引与 SESSION，不修改 Rust/Angular/Go 生产代码；⑤ Mermaid、链接、19×13 索引机器校验与 `git diff --check` 通过 | 赌注 高 | 可逆 是 | @2026-07-24T07:25:04-0700
✅ DONE G01 已新增字段级全领域类图、发送全链时序图、竞态/跨 Surface 状态图和规格输入基线，逐字段覆盖 command/receipt、PendingSend、HTTP body、SQLite message、Go PG 原子写集、WS post、InteractionFrame V2、Timeline/DialogLeft/Unread/Composer、FrameApplied 与 DOM；源码对账纠正 `pending_send` 非 SQLite 表、`ChannelFrame` 非独立 wire type，并明确 CURRENT 是 direct `im_send_message` + Timeline-only、ActionBinding 与三项频道派生 Surface 是 TARGET。Mermaid 3/3、相对链接、源码 sentinel、19 Spec + 19 Plan + 19 Tasks/25 Gate/T021-T077 索引与 `git diff --check` 均 PASS；未修改或运行 Rust/Angular/Go 生产代码与 E2E | @2026-07-24T07:32:38-0700

🎯 EXPECT 复核 018 实施 worktree 并重新治理 Mobile Frame V2 Spec 输入；可证伪断言：① 以 Helix `57d2ddab` 与 cses-client `846afd3fa` 的实际源码证明或否定 producer/PC consumer V2-only；② 明确区分静态硬切完成、删除后二次 25-Gate Native 验收未完成、Mobile consumer 尚未切换三种状态；③ 删除 19 个 UC 入口中重复且已过时的 producer/consumer 切换顺序，只在公共文档保留一次 Mobile 剩余清退合同；④ 只改 `docs/interaction-frame-v2-移动端全量替换-按UC` 与 SESSION，不碰 Rust/C++/Angular/Go 生产代码和其他会话脏改动 | 赌注 高 | 可逆 是 | @2026-07-24T03:34:00-0700
✅ DONE 已按已提交代码确认 Helix 018 `57d2ddab` producer 与 cses-client 018 `846afd3fa` PC consumer 均为 V2-only，V1 Frame 合同/fixture/兼容路径已物理删除；同时确认 Mobile main 仍 pin `477ca58`、保留 V1 consumer，且 018 删除后二次 `T092-T096` 未完成、R4 blocked。Mobile Spec 输入已改为复用上游 V2-only baseline，只保留 Mobile repin/strict consumer/raw fallback 清退/25-Gate 验收；19 个 UC 页删除重复切换文案。验证：Helix Frame 合同 17/17 PASS，Angular codec/runtime 38/38 PASS，目录 29 文件、19 UC、25 unique Gate、G03/G03a=0、断链=0，`git diff --check` PASS | @2026-07-24T03:47:00-0700

🎯 EXPECT 对 `docs/interaction-frame-v2-移动端全量替换-按UC` 做按业务 UC 的文档熵治理并对齐 2026-07-24 当前工作树；可证伪断言：① 从 25 个机器 Gate × 17 文件收敛为 19 个业务 UC 的单文件入口，子 Gate G04a/G10a/G14a/G15a/G16b/G16c 并回父 UC；② 删除目录内预生成 Spec/Plan/Tasks，避免污染随后正式 Spec Kit；③ 明确当前 Helix 仅恢复链局部生产正式 V2、Mobile C++ 仍缺正式 V2 envelope/directive/runtime/surfaceEpoch 与 composer，且移动 source lock 仍 pin `477ca58`；④ 保留 25 Gate 分母、G03/G03a 排除、V1 双端切断和两轮验收合同；⑤ 只改文档与 SESSION，不修改 Rust/C++/QuickJS/Angular/Go 生产代码 | 赌注 高 | 可逆 是 | @2026-07-24T02:48:00-0700
✅ DONE Mobile Frame V2 discovery 文档已从 439 文件/9373 行收敛为 29 文件/576 行：10 份公共真源 + 19 个业务 UC 单入口，25 个机器 Gate 仍完整，6 个子 Gate 已并回父 UC，G03/G03a 仍为 0；目录内全部预生成 Spec/Plan/Tasks 与重复类图/链路副本已删除。源码对账固定 Helix 默认 V1/恢复链局部 V2、Mobile 正式 envelope/composer 缺口、dirty snapshot 回路和 `477ca58` source lock 漂移；49 个相对链接无断链、19 UC/25 Gate 机器计数通过、`git diff --check` 通过。本轮未改或运行 Rust/C++/QuickJS/Angular/Go 生产代码与测试 | @2026-07-24T03:05:54-0700

🎯 EXPECT 在 `docs/interaction-frame-v2-移动端全量替换-按UC` 建立 Mobile Venus/QuickJS/C++ Frame V2 全量替换文档；可证伪断言：① 新增 `docs/AGENTS.md` 明确 PC Angular 与 Mobile Venus 的目录边界；② 覆盖 GS01-GS04、G01-G02、G04/G04a、G05-G09、G10/G10a、G11-G13、G14/G14a、G15/G15a、G16/G16b/G16c 共 25 个独立 Gate，G03/G03a 为 0；③ 每 Gate 冻结 V1 类图/流程/旁路，给出连续 Mobile V2 总图、QuickJS/C++/Surface/UI 合同、共享 Helix 索引、producer+consumer 双切断和两轮验收；④ 禁止空模板，未知事实只允许 `SOURCE_MISSING/CONTRACT_DRIFT/TARGET_DECISION_REQUIRED/NOT_APPLICABLE`；⑤ 本轮只改文档，不改 Rust/C++/QuickJS/Angular/Go 生产代码 | 赌注 高 | 可逆 是 | @2026-07-23T19:21:47-0700
✅ DONE 已新增 Mobile Frame V2 根级公共合同 14 份与 25 个独立 Gate 的 425 份实施文档，并新增 `docs/AGENTS.md` 区分 PC Angular 与 Mobile Venus 所有权；结构验证为 25/25 Gate、439 文件、9373 行、G03/G03a 为 0，100 个全局 Task 唯一连续，本地链接 440/440、75 个 JSON 合同块、逐 Gate 连续总图/V1 producer+consumer 双切断/schemaVersion=2 合同全部通过，跨 Gate 文件无重复 hash。源码对账额外明确当前 C++ schema 2 decoder 仍是 V1 形状且缺 Composer Surface 的 `CONTRACT_DRIFT`；本轮只提交文档，未修改或验证 Rust/C++/QuickJS/Angular/Go 生产实现 | @2026-07-23T19:37:41-0700

🎯 EXPECT 调用 Spec Kit implement 完成 `018-interaction-frame-v2-cutover` 的 96 项任务；可证伪断言：① 只在隔离的 Helix/cses-client worktree 实现，不覆盖两仓当前脏改动；② 先完成 T001-T020 共享 V2-only 基座，再按 T021-T077 逐 Gate tests-first 迁移；③ 首轮单 candidate 25/25、v1/raw=0 且双用户/Go/PG/SQLite/FrameApplied/DOM/截图证据完整后才物理删除 V1；④ 删除后使用新 candidate/run/evidence epoch 再跑 25/25，最终仅 R4 CLOSED 才宣称完成；⑤ Rust Cargo 全程每独立 phase 不超过 2-3 次，不按 Gate 重编 | 赌注 高 | 可逆 否（V1 物理删除阶段） | @2026-07-23T05:15:00-0700

🎯 EXPECT 将 `per-gate-spec-index.md` 升级为可机器核验的全维度矩阵；可证伪断言：① 19 个业务 Spec 每行直接链接 `01`-`13` 全部文件及对应全局 Task ID；② 机器校验严格证明 19 Spec + 19 Plan + 19 Tasks、25 个且仅目标 Gate、T021-T077 连续唯一并与各 Gate Tasks 三元组一致；③ G03/G03a 与 G17 不进入 018 验收矩阵；④ 只修改 018 索引与专用校验脚本，不触碰当前脏工作树中的 Rust、测试和 G01 mmd | 赌注 中 | 可逆 是 | @2026-07-23T04:59:00-0700
✅ DONE `3b430c6` 已将逐 Gate 索引升级为 19 行全维度矩阵，每行直接链接 `01`-`13` 与对应 T021-T077 三元组；新增 fail-closed 校验器并从仓根和 `/tmp` 两处运行通过，证明 19 Spec + 19 Plan + 19 Tasks、25 个目标 Gate、G03/G03a/G17 排除、全局 Task 连续唯一及逐 Gate Tasks 映射完整。本轮未修改 Rust/Angular/Go、未运行 Cargo/Native E2E、未访问 PG/SQLite，已有脏改动保持未暂存 | @2026-07-23T05:08:00-0700

🎯 EXPECT 使用 Spec Kit 为 `018-interaction-frame-v2-cutover` 生成可执行 plan/tasks；可证伪断言：① plan 覆盖共享 V2-only 基座、19 个 Gate 纵切、首轮纯 V2、物理删除 V1、删除后二次回归；② tasks 为 GS01-GS04、G01-G02、G04-G16 逐 Gate 提供 tests-first、Helix、Angular/组件、PG/SQLite/Frame/DOM 证据任务；③ 每个 task 有唯一 ID、文件路径、需求/计划/validator/done signal 映射；④ R2/R3 loop hook 均 PASS 后才报告完成，不修改业务源码或运行 Native E2E | 赌注 高 | 可逆 是 | @2026-07-23T04:18:00-0700
✅ DONE `ef7617b` 已在 `docs/interaction-frame-v2-全量替换-按UC` 为 19 个业务 Spec 逐一生成 `12-V2切换Plan.md` 与 `13-V2切换Tasks.md`，并在 018 生成聚合 plan/research/data-model/contracts/quickstart/tasks；聚合任务 T001-T096 连续唯一且 96/96 有 Loop Task Contract，19 组 Spec→Plan→Tasks 与全部相对链接检查通过。强制 hook 返回 `r2_plan=PASS`、`r3_tasks=PASS`，当前 `next_action=admit_implementation`；本轮未修改业务源码、未运行 Cargo/Native E2E、未写 PG/SQLite | @2026-07-23T04:32:00-0700

🎯 EXPECT 使用 Spec Kit 深化 `018-interaction-frame-v2-cutover` 并为 19 个业务 UC 生成独立 V2-only Gate Spec；可证伪断言：① GS01-GS04、G01-G02、G04-G16 共 19 份 Spec 全部包含目标、Action/Observation、Effect/Persist、PG/SQLite、Surface/组件、工具、证据清单和两轮验收；② 公共切换顺序只保留一份真源，单 Gate 不复制易漂移工具合同；③ 明确现有 Native reducer 与 SCSS/CSS gate 的能力缺口，缺能力时判 BLOCKED；④ 不修改业务代码、不运行数据库写入或 Native E2E | 赌注 中 | 可逆 是 | @2026-07-23T03:44:00-0700
✅ DONE `c3ff0e4` 已新增 19 份逐 Gate V2-only Spec 和 1 份公共验收合同，覆盖 25 个机器 Gate；每份均列出目标、角色、输入、Effect/Persist、Go/PG/SQLite、Surface/Angular 组件、专属证据和首轮纯 V2/删除后二次回归标准。Spec Kit 质量检查与相对链接检查通过，强制 after_specify hook 返回 `r1_spec=PASS`、`next_action=run_speckit_plan`；本轮未修改业务代码、未运行 Cargo/Native E2E、未写 PG/SQLite | @2026-07-23T03:53:00-0700

🎯 EXPECT 对 `docs/interaction-frame-v2-全量替换-按UC` 做双仓源码对齐与文档熵治理；可证伪断言：① 以 Helix `8a7c8a0`、cses-client `8d7108545` 和客户端实际 Helix pin `154c7ba` 记录三方分叉，不再沿用旧 `6fb1a89/181cd02f` 快照；② 唯一顺序改为“先让 Helix V2-only produce + cses-client V2-only consume，再跑纯 V2 全链验收，验收后物理删除 V1，最后二次 E2E”；③ 目标范围固定 G-S1..G-S4、G01-G02、G04-G16 的 25 Gate，G03/G03a 排除，未绑定的会话列表增量 G17 不混入完成口径；④ 文档明确现有 Native 工具缺少 Frame 版本计数/V1 fallback 计数、生产 FrameApplied 回传、SCSS/CSS 专门 gate 等证据能力，且不把旧 017 绿灯升级为 V2 完成 | 赌注 高 | 可逆 是 | @2026-07-23T03:18:00-0700
✅ DONE `1a93563` 已完成 `interaction-frame-v2-全量替换-按UC` 与 `specs/018-interaction-frame-v2-cutover` 的双仓源码对齐和熵治理：范围固定为 25 Gate 且排除 G03/G03a，所有绑定 UC 的下线清单统一为“先切断 V1 producer/consumer 生产路径→纯 V2 首轮全链→物理删除 V1→二次全链”；审计确认 Helix main 与 cses-client pin 已 1/1 分叉、当前两端仍有可达 V1、`surfaceEpoch`/生产 `FrameApplied`/版本计数/SCSS-CSS gate/逐 Gate 双库读回仍是实现与证据缺口。文档结构检查 20 目录、230 文件（130 md/100 mmd）、19 份绑定下线清单、相对链接与源码行号检查均通过；本轮未改业务代码、未运行 Cargo 或 Native E2E，不能宣称 V2 验收完成 | @2026-07-23T03:32:00-0700

🎯 EXPECT 为 `sync/notify` 补齐端到端关联日志而不改变同步语义；可证伪断言：① 每个 Helix SyncPull 请求携带由 corr 派生、无凭据的 `Cses-Track-Id`，日志可见 corr/channel/fromSeq/trigger/队列窗口；② Go 同一 trackId 可见请求 cursor 与响应 kind/nextSeq/event/message 摘要；③ Helix 回包、PersistAtomic 与 PersistOk 可用 corr 链接；④ 不记录 header、cookie、消息正文或原始 body，聚焦 Rust/Go 测试通过 | 赌注 中 | 可逆 是 | @2026-07-22T23:18:00-0700
✅ DONE Go `005429d`、Helix `78c4a94`/`d2ee633` 与 cses-client `d46c6ef87`/`a4dcd230d` 已补齐并回钉同步关联日志；Go focused test 覆盖率 100%、build/vet 通过，Helix header/pong focused tests 与 cses-client Native build 通过。真实 444 冷 profile 运行确认 `sync.dispatch → sync.reply_received → sync.response → recovery.persist_dispatched → sync.persist_ok` 可按 channel/corr/track 串联，并直接暴露 710 个频道在 Routine 后又被 PongGap 全量入队，`scheduler_pending` 从 709 降至 0；最终 pin 另新增 `pong.compensation_decision` 直接输出 gap/hashMismatch/known/target 数，现役 Go 进程需重启到 `005429d` 才会输出新增 API completion 摘要 | @2026-07-22T23:12:00-0700
⚡ DRIFT-PRED 日志任务的真实 Native 回放进一步揭示狂刷不是单个坏频道，而是 PongGap 对 710 个频道的全量扇出；单次回放写入 710 cursor、6055 ledger、5118 message，随后 PongGap 大量返回 no_change → 候选教训：pong 只应触发有服务端 head 差异或显式 gap 的频道，不能把全量频道扫描当心跳默认路径 | @2026-07-22T23:12:00-0700

🎯 EXPECT 修复 `sync/notify` 混合 terminal 恢复批次导致的自激请求风暴；可证伪断言：① 真 Go `[type1@1,type7@2] + messages + nextSeq=2` 从本地 cursor=0 只产生一次包含消息、terminal tombstone 与 cursor 的 `PersistAtomic`，回执后 cursor/terminal 均为 2；② 不再对同一确定性拒绝响应立即 proactive resync；③ 错序、terminal 非末尾或 authority head 不一致仍 fail-closed；④ 聚焦测试先红后绿，重启 Native 后现存 16 个 G16 channel 被一次性追平且新增同签名 WARN 为 0 | 赌注 高 | 可逆 是 | @2026-07-22T21:48:54-0700
✅ DONE Helix `6fb1a89` 已把可见消息前缀、terminal tombstone 与 cursor 合并为单次 `PersistAtomic`，并在 PersistOk 后提交内存状态与 Frame；聚焦测试 5/5 通过，cses-client `181cd02fd` 已回钉并完成 Native build。真实 444→Go 隔离 profile 将现存 16/16 个 G16 关闭频道追平到 cursor/terminal=2，重启后 terminal reject WARN=0 且这些频道无新增 sync-flow 命中 | @2026-07-22T22:58:00-0700

🎯 EXPECT 新建 `018-interaction-frame-v2-cutover` 跨仓 Spec 与细粒度 Strict Matrix，只梳理 Helix -> cses-client/Angular 的当前与目标合同，不修改业务实现；可证伪断言：① 覆盖 GS1-GS4、G01-G02、G04-G16 及 04a/10a/14a/15a/16b/16c 共 25 个机器 Gate，明确排除 G03/G03a；② 每行均给出 ActionBinding/HostObservation、Tick、Effect、Persist barrier、V2 Frame/Surface、Angular 组件、逻辑处理、当前缺口与验收证据；③ 明确客户端 reader 就绪、Helix V2 writer、pin、全量 E2E、禁收 V1、删除兼容代码及删除后回归顺序；④ 不把既有 017 FULL GREEN 当作 V2 清退证据，且质量 checklist 无未决项 | 赌注 高 | 可逆 是 | @2026-07-22T22:10:00-0700
✅ DONE `018-interaction-frame-v2-cutover` 已建立 spec/discovery/Strict Matrix/requirements checklist/review ledger：Helix 与 Angular 两张 Matrix 各 25/25 行且排除 G03/G03a，逐行覆盖 ActionBinding/Observation、Tick、Effect、Persist、V2 Surface/Frame、组件/逻辑与 CURRENT/TARGET/GAP；R1 首轮因 discovery source lock/fact 格式被 loop 阻断，修正后 `r1_spec=PASS`、next=`run_speckit_plan`；本轮未修改业务代码、未运行 Cargo/Native E2E | @2026-07-22T22:46:00-0700
🎯 EXPECT 手机端既有 `POST /oss/verifyUpload` 的 bucket+key 合同与匿名兼容行为保持不变；Java 新增强制登录的 `POST /oss/completeMediaUpload`，Rust 媒体 Complete 阶段只改走新路由，Go 生产代码零 diff。可证伪断言：① 旧/新接口各有真实 HTTP 合同测试；② Rust 全仓不再把新媒体完成请求发到 `/oss/verifyUpload`；③ Java OSS 全测、Rust 媒体合同与 host 同链 E2E 全绿；④ 不发布、不 push，Rust agent cargo 调用 ≤3 次 | 赌注 高 | 可逆 是 | @2026-07-24T10:39:52+0800
✅ DONE 旧 `/oss/verifyUpload` 已恢复并锁定手机端匿名 bucket+key 合同；新 `/oss/completeMediaUpload` 以 required `@UserSession` 和 userId 二次校验承接 Rust 媒体票据完成链。Java OSS 45/45、Rust 媒体合同 35/35、host rich 九图+FILE 同链 E2E 1/1 通过；同链证明 cookieId+deviceId 只进入 Java/Go 控制面而不泄漏到 OSS PUT。隔离 composite build 明确以本地 `paas-service-oss` 编译 CSES，`:server:compileJava` 27/27 成功且主脏 checkout 基线未变；Go 生产代码零 diff，Rust 执行 agent cargo 恰好 3 次，未发布、未 push | @2026-07-24T10:54:29+0800
⚡ DRIFT-PRED 新接口的真实 session 绑定除 cookieId 外还要求 deviceId，原 host 同链只证明了 cookieId 注入；补齐 deviceId 的 Java/Go 透传和 OSS 跨 origin 禁泄漏断言后复验通过 → 候选教训：登录态跨端合同必须同时锁住完整身份头集合与对象存储凭据隔离，不能只验证单个 token | @2026-07-24T10:54:29+0800

🎯 EXPECT 补齐媒体进度到 FFI 前端边界的组合证据：生产 TimelineProjector 必须把已提交的 FILE 35% 投成 `Uploading{progressPercent:35}`、同批 IMAGE 即使携带脏百分比也只能投成 `Sending`，且完整 `im:interaction:frame` 经 FfiEventSink 后由 `on_domain_events_batch` 宿主回调原样收到；只改 FFI 既有 parity 测试与本条 SESSION，不复制 host 媒体 E2E，不改生产代码，cargo 调用 ≤2 次 | 赌注 中 | 可逆 是 | @2026-07-24T04:20:00+0800
✅ DONE 新增 FFI 前端边界组合测试：生产 TimelineProjector 将同为 35% 源行的 FILE 投成 `Uploading{progressPercent:35}`、IMAGE 收紧为 `Sending` 且不携带百分比；生产 FfiEventSink 单次非空 flush 经 `on_domain_events_batch` 恰好一次批回调，宿主解码后收到完整 `im:interaction:frame`。`cargo check -p helix-driver-ffi` 与 `cargo test -p helix-driver-ffi` 全量通过，热路径审计、rustfmt 与 diff check 通过；本测试组合已提交 progress readback 与 FFI egress 两个生产边界，不重复模拟网络/OSS | @2026-07-24T04:32:00+0800

🎯 EXPECT 补一条不拆段的本地真实媒体链验收：同一个 host engine 必须把 rich 九图和 FILE 依次送到三个不同 origin（Java prepare/verify、真实流式 PUT、Go posts/create），并用真 SQLite 对账最终消息；可证伪断言：① Java 收 10 prepare+10 verify、OSS 收 10 个完整 PUT、Go 只收 2 个 posts/create；② rich 全链无图片 PortProgress，FILE 至少产生一个 5% 中间进度且最终 100；③ Go body 只含稳定 Java URL，不含签名 query/localPath/mediaInput；④ 只新增 host 集成测试与本条 SESSION，完成前 cargo 调用 ≤2 次 | 赌注 高 | 可逆 是 | @2026-07-24T03:40:00+0800
✅ DONE 新增 host 同链 E2E：一个生产 `run_engine_loop` 依次完成 rich 九图与 FILE，独立本地 origin 实收 Java prepare/verify 各 10 次、OSS 完整 PUT 10 次、Go posts/create 仅 2 次；CrossCuttingHttp 注入 cookieId 且不向 OSS 泄漏凭据，Java 一次性票据逐字段绑定，Go 只收到稳定 Java URL；rich 无 reporter/百分比写，FILE 首个持久化桶 5%、终态 100，真 SQLite 两行均 sent 且 pending_media 清空。目标测试 1/1 与 `cargo test -p helix-driver-host` 全量通过 | @2026-07-24T03:58:30+0800
⚡ DRIFT-PRED 原预判 cargo 调用 ≤2；整包通过后的安全复核又识别出测试桩未强制 Java 票据绑定及 OSS 凭据隔离会留下假绿，补强后追加第 3 次目标复验并通过 → 候选教训：跨服务协议桩不仅要对账路径和计数，还必须模拟真实服务的认证、一次性票据绑定与跨 origin 凭据隔离 | @2026-07-24T03:58:30+0800

🎯 EXPECT 收口 Surface retry action 的 accepted ledger：storage Err/not-found、内存 no-op 与 local-first 查询代际竞争都必须回到原 window 的同 causation 终态 Frame；可证伪断言：① action durable lookup 失败后仍产生 causal local readback；② causal readback 不被普通新查询 generation 丢弃；③ Failed Frame 携带 requestId 并释放 actionRef ledger；④ 不改旧 `im_retry_upload` 公开语义，只跑一次 helix-im 目标测试 | 赌注 高 | 可逆 是 | @2026-07-24T02:51:30+08:00
✅ DONE Surface retry action 的可信 channel/window 已贯穿 durable lookup；内存 no-op、storage Err/not-found、解析/状态错误统一回到原 timeline causal readback，action causal query 不再被普通 generation 淘汰，Failed Frame 携带 requestId 并释放 ledger。`cargo test -p helix-im --lib durable_retry_ -- --nocapture` 2/2 通过；旧 `im_retry_upload` 未改 | @2026-07-24T03:04:00+08:00

🎯 EXPECT 按用户拍板将 rich/file 媒体控制面从 Go facade 改为 Rust 直连前端注入的 Java base：消息 `posts/create` 仍走 Go，prepare/verify 只走 Java `/oss/getPresignedUrl|verifyUpload`；可证伪断言：① 任一媒体发送 Effect 都不再命中 Go `/media/upload/*`；② Java base 与 Go base 可配置为不同 origin，native/FFI/Web 不从 Go 地址猜 Java 地址；③ Java 返回稳定 object identity/public URL，Rust durable props 仍不落预签名 URL/headers；④ 九图无逐图百分比、FILE 真实 5% 进度与 retry window/causation 均不回退；⑤ Java/Rust focused+三端验证与真实 OSS rich/file 尽可能全绿；分支身份：本仓 `codex/rich-file-oss-upload`，任务完成前不得删 | 赌注 高 | 可逆 是 | @2026-07-24T00:38:52+0800
✅ DONE 双地址媒体链已按拍板完成：Rust 的 Go base 只发 `posts/create`，独立 Java base 只发 `/oss/getPresignedUrl|verifyUpload`，FFI 新增 `helix_create_with_endpoints(go, java, ws, ...)` 且旧入口不推导 Java；Java 以 HMAC uploadToken、OSS HEAD/元数据、completed marker 和 forbid-overwrite 建立稳定引用；rich 1..9 图无百分比、host 最大 8 路并发，FILE 保留 5% 真实字节进度。媒体初始 message+journal、retry reset、最后引用+durable unsend marker 均以 PersistAtomic 形成 I/O barrier，重启只重放缺失阶段。验证：Rust 媒体合同 26/26、host 九图真实本地 HTTP PUT、全量多 crate tests、Web wasm/build adapter、FFI C++ smoke、Strict Matrix 与 core 零运行时闸门通过；Java OSS 41/41，Go focused 3/3、build/vet 通过（全量仅既有 `/tmp/cses-im-server.log` 截断保护失败）。未发布的新 Java 代码不能冒充真实 OSS E2E，前端仍需切到新双地址 ABI；大于 64MiB 暂 fail-fast，multipart 未纳入本次 | @2026-07-24T03:24:00+0800
⚡ DRIFT-PRED 拍板后拓扑从 Go facade 改为 Rust 直连 Java，安全复核又暴露 uploadToken 绑定、对象验证、完成 marker、乐观写 barrier、verify 并发争写与 ACK→HTTP 崩溃窗；同时发现 `paas-service-oss:2.0.0.12` 已发布旧内容，故没有覆盖版本或伪造线上证据 → 候选教训：跨仓媒体链要把拓扑、对象可信性、持久化 barrier、恢复窗口和制品身份作为同一验收矩阵，而非只测上传 200 | @2026-07-24T03:24:00+0800

🎯 EXPECT 修复 rich/file retry 的非 latest window 因果回读：只在 `PendingSend` 固化 action 授权的 window token，并让 file progress、upload failure、posts/create 成败与 WS echo 复用同一 window+causation；可证伪断言：① `retry-action-window` 的进度及成功/再次失败 Frame 都不落到 `latest`；② 普通新发送默认 window 仍为 `None -> latest`；③ 不新增逐图片进度、JSON RMW 或平台/runtime 依赖，helix-im check/test 与热路径审计通过 | 赌注 高 | 可逆 是 | @2026-07-23T21:39:51+0800

🎯 EXPECT 在 `codex/rich-file-oss-upload` 完成 rich/file 真上传闭环：保留 Go `/media/upload/prepare|complete` 媒体 facade，由 Go 向 Java 取 OSS 预签名、Rust host 直传 OSS；rich 最多 9 图只投影整条消息上传中且不产生逐图进度，file 产生节流的真实字节进度；可证伪断言：① Rust/FFI 不新增 Java 地址，仍只依赖 Go IM base；② rich 9 图签名/上传有界并发且前端事件无图片百分比；③ file 进度单调 0..100、跨 FFI/Web 批处理且不逐块落库；④ Rust/Go/Java focused tests 全绿并尽可能取得真 Java→OSS→Go 证据；分支身份：本仓 `codex/rich-file-oss-upload`，上游 main@627e5cf，任务完成前不得删 | 赌注 高 | 可逆 是 | @2026-07-23T18:52:33+08:00
✅ DONE rich/file 媒体闭环已按 Go facade 收口：Rust 仍只持 Go base，prepare/complete 严格解 ADR-007 HTTP envelope，rich 1..9 图使用有界上传池且禁逐图进度，FILE 以 64KiB 流产生 5% 桶、最多 20 个 PortProgress 并 latest-wins 持久化；预签名 URL/headers 不落 durable props，PUT 失败重新 prepare、complete 失败只重试 complete，非 latest retry 的 window+causation 贯穿 progress、再次失败、HTTP 成败与 WS echo。真实链验证：9 图得到 9 prepare + 9 PUT + 9 complete + 1 posts/create 全 200；8,023,771B 文件得到 1+1+1+1 全 200、20 个真实 progress hop、终态 100；10 个无鉴权稳定 URL GET 200 且 SHA-256 全匹配。Rust helix-im 全量、三端 check、core 22 tests、Web wasm build、core 零运行时依赖均通过；Go media/WS/realchain focused+race、build/vet 与 Java OSS 19 tests 通过，CSES file bucket 已固定北京 region。新 Java verify/download 路由未部署，故真实 OSS 证据只覆盖现役 Java signer 的 PUT/GET，不冒充新 Java 全链已上线；Go `media.file_id` 唯一索引需 `postgres-pro` 隔离迁移证据，列为 P2 | @2026-07-23T22:24:00+08:00
⚡ DRIFT-PRED 首轮真实九图暴露既有 G03 测试绕过 host ADR-007 envelope，导致测试全绿但真链 9 个 prepare 后零 PUT；另发现 reqwest 对签名返回的 Content-Type 采用 append 会形成重复头 → 候选教训：HTTP 状态机合同必须以真实 driver codec fixture 回放，预签名 header 必须由 ticket 权威且逐名防重复 | @2026-07-23T21:25:00+08:00

🎯 EXPECT 修复 G05 Native 多目标转发终局 overlay：只改 cses-client 转发 overlay 的关闭/尺寸契约与 focused harness 诊断，不把 createPosts HTTP 受理当成投递成功；可证伪断言：提交后的 overlay 在 Helix terminal causation Frame 到达前后都不会被 backdrop/Escape 隐式销毁，两个目标仍只在 matching WS + PersistOk Frame 后投影为 pending，Angular focused tests 与 tsc 通过 | 赌注 中 | 可逆 是 | @2026-07-22T12:13:00-0700

🎯 EXPECT 并行实现 G08-G12 的专属 Rust query/schedule/timeline/reply 模块与 focused 测试、必要的 cses-client 专属组件测试；可证伪断言：不修改已声明共享 owner 文件，G08 owner-only hydrate、G09 取消/到点 G01 边界、G10 Observation/Action、G11 VM target/directive 无 payload、G12 query graph/G01 reply 依赖均有回归，批量 focused Rust/Angular 验证通过后才更新 task/handoff，不运行 Native E2E | 赌注 高 | 可逆 是 | @2026-07-22T08:18:00-0700

🎯 EXPECT 调用 Spec Kit implement 完成 G04-G16 的 Helix/Go/cses-client 纵切，并在依赖允许时按不重叠 owner/file set 并行；可证伪断言：① 004-016 逐组执行 before_implement admission，测试先于实现且完成 task 才标 `[X]`；② ActionBinding/Tick/authority/PersistOk/完整 Frame/Angular UI 不回退 raw reducer；③ 最后只由 017 构建 final Native candidate，双账号连续 UC04-16 required fail=0/skip=0；④ 每个 UC 产出 handoff.md，记录真实完成、证据、耗时与未决项；⑤ 同一失败签名最多自动修复两次，第三次 NeedsHuman | 赌注 高 | 可逆 是 | @2026-07-22T08:05:00-0700
✅ DONE G04-G16 已在单 Native 拓扑 `mrc-g04-g16-green-1784776617` 连续跑完 19/19 Gate，required fail=0/skip=0；004-017 tasks 全部完成、逐 UC handoff/evidence 已回填，R1-R4 全 PASS 且 loop CLOSED，逐组 analyze CRITICAL=0；前端 tsc/Jest、Go focused packages、Helix core/im 全量与 Web wasm build 通过 | @2026-07-22T21:05:00-0700
⚡ DRIFT-PRED Helix 全量回归额外暴露恢复 `PersistAtomic` ack 后重复 cursor 写与三个旧测试合同漂移，Cargo 调用超过原预算 → 候选教训：最终聚合前先静态检索 `ChannelPersist` 是否仍产二次 cursor Effect，并一次性迁移所有恢复 fixture 到 tenant+actor authority | @2026-07-22T21:05:00-0700
✅ DONE 最终 Helix pin `a2f835fbb4bee7dda62961c55c4fa9a931302d98` 已由单 Native 拓扑 `mrc-g04-g16-final-a2f835f-1784780000` 连续验证 19/19 Gate，required fail=0/skip=0，耗时 186808ms，合并摘要 `693a10120369536d0401ec031d3e1b18bdfdf18ecdc77e73d16c5cec65b8bfba`；G08 首跑的频道重绘吞首击已修复并经 targeted + full rerun 双重验证 | @2026-07-22T21:18:00-0700
⚡ DRIFT-PRED final pin 首轮唯一红灯不是 G08 authority/Persist/Frame，而是 observer header 被增量重绘后菜单首击丢失 → 候选教训：Native 强交互断言必须在目标 row ready 后重新锚定 active channel，并把 DOM 被替换与业务状态不符拆成不同失败签名 | @2026-07-22T21:18:00-0700

🎯 EXPECT 补齐 `004-017` 的 discovery/data-model/contracts/review-ledger/verification 共同治理文件与逐组 Constitution Check，并修正 tests-first、跨仓 worktree、CURRENT/TARGET/GAP、两次自动修复上限及各 Gate 特有语义；可证伪断言：① 14 组治理文件完整且无模板占位；② 004-016 测试任务先于生产实现，017 复现红灯先于 owner repair；③ 14 组 R1-R3 均由 loop_engine 生成当前指纹 PASS；④ 最终逐组 speckit-analyze 无 CRITICAL，若仍有问题则按 Gate 列出而不伪称通过 | 赌注 高 | 可逆 是 | @2026-07-22T07:20:00-0700

🎯 EXPECT 将 017 从只读验收编排升级为有代码修改权限的收敛修复循环；可证伪断言：① Spec 明确不得在 Red/Partial 报告后停止，必须按日志与五面证据定位并修改 Helix/Go/cses-client；② Plan 允许每个修复批次一次增量 Native rebuild，禁止按 UC 重编，并要求最终 candidate 全批重跑；③ Tasks 拆出 Helix、Go、Angular/UI 三类修复、影响面回归、最终全批与 Index-Dual 双尺寸复验；④ 完成条件是 G04-G16 required=0 fail/skip，而非“已发现问题” | 赌注 高 | 可逆 是 | @2026-07-22T06:08:00-0700

✅ DONE 017 已升级为跨三仓 `run -> classify -> repair -> rebuild -> verify -> rerun` 收敛执行包：Spec 新增 10 条强制修复需求和不得 Red/Partial 停止的退出闸门；Plan 明确 Helix/Go/cses-client owner、日志/PG/SQLite/Frame/DOM 六面诊断、repair-batch 构建纪律与 Index-Dual 视觉修复；Tasks 扩为 T001-T035，35/35 均有独立 Loop Task Contract，包含生产代码修复、可证伪回归、增量重建、影响面重跑和 final candidate 全批 Full Green；本轮只更新规格，未运行 cargo/Native/E2E | @2026-07-22T06:18:00-0700

🎯 EXPECT 在 `specs/` 按 Gate G04-G16 拆出 13 组独立 `spec.md/plan.md/tasks.md`，另设只负责任务汇总验收的 UC04-16 E2E 包；每组明确 ActionBinding、Tick、权威事实、PersistAtomic、Surface/完整 VM row、InteractionFrame、Angular 组件与 PD UI 效果；单 Gate 只做 focused 合同/fixture 验证，Rust Native、双账号、远端 PG、本地 SQLite 与 DOM/视觉验收只在全部 Gate 开发完成后统一执行一次；不改生产代码与现有 001-003 Spec | 赌注 高 | 可逆 是 | @2026-07-22T05:35:00-0700

✅ DONE 已新增 G04-G16 共 13 组独立 Spec/Plan/Tasks（每组 10 个细粒度 task）与 1 组只负责汇总验收的 017 E2E 包，共 42 个分组文档，另有 1 个执行索引；每组均覆盖 ActionBinding、Tick/Effect、权威事实、PersistAtomic、Surface/完整 VM row、InteractionFrame、Angular 组件与 PD UI 状态；017 固定一次 Native build 后连续跑 UC04-16，并分列双账号、Go HTTP/WS、远端 PG、本地 SQLite、Frame、DOM 与双尺寸视觉证据；未改生产代码、未运行 cargo/Native/E2E | @2026-07-22T05:52:00-0700

🎯 EXPECT 将 `rich-v2.json` 收编为 Helix formal pin 内的 InteractionFrame V2 fixture。可证伪断言：① fixture 含完整 runtime、surfaceEpoch、causationId 和 directives envelope；② Rust `InteractionFrameV2::decode` 可直接解码并保留 reply thread、reaction 与 quick actions；③ manifest 显式登记该 fixture；④ 不改生产投影逻辑、driver 或 wire schema | 赌注 中 | 可逆 是 | @2026-07-22T03:41:00-0700

✅ DONE `rich-v2.json` 已进入 Helix fixture manifest，并使用完整 V2 envelope 固定 reply thread、reaction、quick actions 与 topic link；Rust InteractionFrame 合同 17/17、fixture manifest 4/4 通过，未改生产投影逻辑或 wire schema | @2026-07-22T03:46:00-0700

🎯 EXPECT `message.timeline` 的 Helix `MessageVm` 明确投影最多 3 条回复预览、权威回复总数与逐 emoji 计数；Angular message-v4 仅据此渲染话题卡、回复摘要、气泡外表情及 `message.thread-detail` 抽屉，并通过既有 `im_send_message(replied)` 提交抽屉回复；不改左侧会话列表、导航、存储 schema 或原始事件直通 | 赌注 高 | 可逆 是 | @2026-07-22T04:10:00-0700

✅ DONE Helix `MessageVm` 已新增有界 `replyThread` 与逐 emoji `reactions`，回复预览稳定排序并截到 3 条、总数不被预览长度覆盖；聚焦单测 1/1、InteractionFrame 合同 16/16 通过，Angular 组件测试 2/2 与 fast build 通过；真实双用户 WS/持久化回放尚未执行，不晋级 real-chain-green | @2026-07-22T04:58:00-0700

🎯 EXPECT 左侧会话列表 Frame 保留 channel `pictureType=USER` 的 viewer-aware `userIds`（群聊最多 4 个）与 `pictureType=PICTURE` 的图片资源，并继续以最后消息时间作为 `lastActivityAt`；只改 helix-im 头像投影合同/测试，不改排序、未读、网络或存储 | 赌注 中 | 可逆 是 | @2026-07-22T03:10:00-0700

✅ DONE `5a5ff1c` 已让 Dialog Frame 透传 Helix 裁决后的 `userIds`，USER/PICTURE 分支与单聊对端/群聊四宫格均有回归；helix-im 聚焦测试 10/10、InteractionFrame 合同 16/16、cses-client 新 pin Rust lib check 通过 | @2026-07-22T03:38:00-0700

🎯 EXPECT 只新增一张当前离线同步全链路 Mermaid 图；图中明确分开 cses-client 当前 pin 的 v1 实运行路径与 Helix main 的 V2 候选路径，不把候选实现写成已接入 | 赌注 中 | 可逆 是 | @2026-07-22T00:31:08-0700

🎯 EXPECT 只改 helix-im 的发送投影/预览分类与对应测试：P1 PersistOk 后 `message.timeline` Sending Frame 先于 posts/create，WS/HTTP 终态持久化后刷新同一 slot；`messageClass=normal|mention|urgent` 且 `lastPost.type=TEXT` 始终投影 text preview；不改 cursor、driver 或平台 reducer | 赌注 高 | 可逆 是 | @2026-07-22T01:48:00-0700

✅ DONE `410e9ea` 已补齐 P1 Sending、HTTP/WS/失败终态的 attached `message.timeline` 回读，并让 preview kind 优先读取 `lastPost.type`；Helix 单测 301/301、发送集成 66/66、PC repin 后 Rust lib check 均通过；全量测试仍被既有 `c2_phantom_readbits_test` 缺 tenant/actor identity 阻断 | @2026-07-22T01:52:20-0700

🎯 EXPECT 已确认本地写后的 `message.timeline` refresh 只允许一个有界 message Scan 和 Frame，不得产生 `im_get_latest_post` HTTP；普通文本发送仍保持 P1 Frame 先于唯一 posts/create，WS/HTTP/失败终态不重复出网查最近消息；只改 helix-im query correlation/测试，不改公开协议与 driver | 赌注 高 | 可逆 是 | @2026-07-22T02:18:00-0700

✅ DONE `9dfc728` 已将 durable-fact timeline refresh 收紧为 local-only Scan + Frame，P1/HTTP/WS/失败终态不再 fallback `getLatestPost`；显式 renderer query 保留远端补窗；红灯复现后 Helix 302/302、聚焦集成 39/39、PC 四 crate repin、契约门禁与 Rust lib check 均通过 | @2026-07-22T02:22:45-0700

🎯 EXPECT `message.composer@Channel` attach 必须以 O(1)、零 HTTP/Storage 的 Helix typed snapshot 结束 `awaiting-snapshot`；在 V2 message.create authority 尚未接线前只签发明确 disabled 的发送 binding，不伪造 live capability；补齐 projector/revision/session reset 与 surface resync 回归测试，随后回钉 PC | 赌注 高 | 可逆 是 | @2026-07-22T02:42:00-0700

✅ DONE `67d1ba1` 已为 `message.composer@Channel` 增加 O(1)、零 I/O typed snapshot 与 identity-scoped revision projector，首帧不再落入 surface/scope mismatch；未接线的 V2 send binding 保持 disabled；红灯复现后 Helix lib 304/304、合同/查询 29/29、PC repin/门禁/check PASS；全量仍仅被既有 `c2_phantom_readbits` tenant/actor fixture 阻断 | @2026-07-22T02:49:09-0700
🎯 EXPECT 新增 G01 Helix 侧完整 sans-IO 回放测试包；可证伪断言：① 测试从真实 `im_send_message` Command 开始，覆盖 P1 optimistic Persist→PersistOk→sending Frame→posts/create/timer→HTTP sent/failed Persist→终态 Frame，并单独覆盖 WS `post` Inbound 的对账/可选 dialog patch；② 使用现有 G15a create-channel 回放作为“先建群再发送”的独立前置合同，不把 Angular/SQLite/Go/SurfaceRuntime 冒充为 Helix 单测；③ 不修改生产代码，聚焦 `cargo test -p helix-im --test <new-test>` 与既有 G01 fixture 合同通过；④ 不触碰已混合暂存的 `12-G01-完整主链路.mmd` | 赌注 中 | 可逆 是 | @2026-07-24T18:00:00-0700
✅ DONE 新增 `crates/helix-im/tests/g01_full_flow_test.rs` 与 `tests/g01_full_flow/` 四模块 Helix 回放包：G15a 建群权威 channel/member facts 后复用返回 channelId 发 G01；HTTP 正/反终态均严格经过 optimistic Persist、attached timeline readback、sending Frame、posts/create+15s timer、terminal PersistOk/readback 后才产 sent/failed Frame；WS `post` 单独证明 echo terminal Persist→timeline sent 与 GuardedBump+Get(channel)→DialogLeft 是两个可竞速 correlation，late HTTP 幂等不二次终结。聚焦测试 4/4、既有 G01 canonical fixture 1/1 PASS；未运行 Angular/Native/SQLite/Go/PG/真实 WS，不升级为跨端 E2E | @2026-07-24T18:34:00-0700
⚡ DRIFT-PRED 测试首轮把 `api_base_url` 误当成会自动补 `/api/cses`，且把建群 channel/member 写误断言为同一 PersistFire；源码/既有测试证明 base 由 host 原样注入、两个表可分散在同 step 多个 Effect 中，导致 Cargo 聚焦调用 4 次超过预期 2-3 次 → 候选教训：测试 URL 只断 endpoint suffix，跨表事实按整批 Effect 聚合后断言，不把 Vec 容器布局当协议 | @2026-07-24T18:34:00-0700
🎯 EXPECT 将“每一个方法都要有精简注释”沉淀为全局 Role、Helix 项目规则与全局记忆，并立即补齐 G01 新测试包；可证伪断言：① `/Users/xpa-12/.codex/AGENTS.md` 与 Helix `AGENTS.md` 都明确覆盖 public/private/test/helper 方法；② 注释只说明职责、边界、不变量或非显然副作用，不复述实现；③ G01 测试包全部 24 个函数/方法均有紧邻精简注释；④ 不修改生产代码、协议、fixture 或既有混合暂存的 G01 文档，格式检查与聚焦测试通过 | 赌注 低 | 可逆 是 | @2026-07-24T18:43:11-0700
✅ DONE 已把方法注释规则写入全局 `/Users/xpa-12/.codex/AGENTS.md`、Helix `AGENTS.md` 与全局记忆更新说明；规则覆盖 public/private/test/helper，要求紧邻精简注释且只写职责、边界、不变量、原因或非显然副作用。G01 回放包 24 个函数/方法已全部补齐注释；静态邻接检查、rustfmt、`git diff --check` 和 `cargo test -p helix-im --test g01_full_flow_test` 4/4 PASS（编译 3.87s、测试 0.01s）；未修改生产代码、协议、fixture 或既有混合暂存文档 | @2026-07-24T18:46:00-0700
🎯 EXPECT 收敛 Helix、cses-client、Mobile QuickJS 的 G01 为单一业务状态机与同源 Frame 合同；可证伪断言：① sending/sent/failed、temporaryId 对账、Dialog preview/lastActivityAt 均只由 `helix-im` 产出，Angular/QuickJS 不新增或保留独立业务 reducer/optimistic row；② 三端测试复用 Helix canonical G01 fixtures 并字段级验证同一 `InteractionFrameV2`，创建群聊前置成员权威集合严格为 444/678；③ PC 双端真实链使用 Frame latch/correlation 而非固定 sleep，统一报告覆盖 handoff 的 11 个 required stage；④ Helix、cses-client Mock/real 与 Mobile GTest Gate01 全部通过后才声明 Green，任一真实环境前置缺失则明确 blocked | 赌注 高 | 可逆 是 | @2026-07-24T19:10:00-0700
✅ DONE G01 已收敛为 Helix 单一状态机与六份 canonical Frame：Angular 与 Mobile 仅做通用 SurfaceRuntime atomic apply，Mobile `send/sendWithTrace` 仅返回 Accepted，不再消费 raw sending receipt。Helix check、full-flow 4/4、Golden 1/1；cses contracts 83/83、Mock 4/4、integration 2/2、single-real 1/1、dual-real 2/2 且 11/11 required stage、12 文件 redaction 全通过；Mobile Fixture 6/6、GTest 5/5、Venus QuickJS、source-pin 与 V1 hard-cut gate 全通过。真实链证明 UI 新建 444/678 群、PG 成员集合严格相等、双端 timeline/dialog FrameApplied 与 DOM、reqId/causationId/trackId/postId/eventSeq 闭环，自发消息 unread 从基线 1 保持 1。本地三仓 Gate01 Green；真手机双账号、受控真实网络失败及单一 suiteRunId 发布包不在本轮声明范围 | @2026-07-24T22:55:00-0700
⚡ DRIFT-PRED 首轮把 DialogLeft preview 错当完整 marker，并把自身 unread 错当绝对 0；源码与真实 DOM 证明 preview 是 Helix `simpleMessage` 前 50 个 Unicode 字符，且建群系统消息可令发送前 unread 已为 1 → 候选教训：跨 Surface 断言必须按各自投影字段合同，计数型副作用应比较操作前后差值而非假设绝对初值 | @2026-07-24T22:55:00-0700
🎯 EXPECT 按总控 Spec 严格串行实施 26 个 InteractionFrame V2 Gate；首阶段只允许完成总控 T001-T002 与 G02 T001-T038，G02 未取得 Helix canonical Fixture、PC Mock/Real、Mobile GTest/QuickJS typed view、三仓一致性 Review 和 R4 PASS 前不得进入 G03a；可证伪断言：① 当前 26 Gate manifest/索引无漂移且 G01 在三仓当前 HEAD 重新全绿；② G02 文档 URL 只作为文档消息卡片，不进入文件上传状态机；③ Fixture 唯一 producer 是生产 `ImModule::handle -> Effect::Emit`，PC/Mobile 不补领域字段或 reducer；④ 每个完成任务即时标记 `[X]`、真实验证后分类提交，任一真实链阻塞明确停在 blocked/partial | 赌注 高 | 可逆 是 | @2026-07-25T00:52:28-0700
✅ DONE G09 已完成 Helix、PC、Mobile 三仓纵向闭环并由 SpecKit event 19 最终 `CLOSED`：Helix full-flow 7/7、interaction 3/3、Fixture 1/1；PC Mock 4/4、single-real 1/1、dual-real 1/1、report 1/1 与 production boundary 全绿；Mobile Frame contract 5/5、focused 9/9、fresh-group real chain 10/10，三仓四份 Fixture raw/provenance byte-exact。业务取消、HTTP/WS authority、Persist 与 Frame 只在 Helix，平台未新增 reducer；三仓已分类提交，未 push，也未触碰 Mobile 既有未跟踪 bundle/DS_Store | @2026-07-25T20:50:17-0700
⚡ DRIFT-PRED G09 首轮真实链暴露两项原预期外边界：channel-summary 虽投影 enabled `cancelSchedule` 却未注册 ActionBinding dispatch；create 与 cancel 使用独立 reqId 导致 Go evidence 必须按 correlation 分区读取，不能要求单个 snapshot 同时包含两次 HTTP。Mobile 单独运行含 RealChain 的 filter 也因缺 runner context 正确 BLOCKED，随后共享 runner 10/10 通过 → 候选教训：Frame 中出现 enabled action 必须有 registry 可达性合同；多命令 UC 的证据按 reqId 分区后再汇总 | @2026-07-25T20:50:17-0700
🎯 EXPECT 续接总控串行实现并关闭 G14 单条已读；可证伪断言：① 只由生产 `ImModule::handle` 经 `im_post_read`、权威回执、Persist barrier 和 `Effect::Emit` 产出 `message.participant-receipts` canonical Frame；② G13 频道级已读与 G14 单条回执保持独立语义，PC/Mobile 不新增领域 reducer；③ Helix full-flow/Golden、PC Mock+single/dual real+report、Mobile production SDK/Bridge/SurfaceRuntime/QuickJS GTest 与三仓 hash parity 全绿后才进入 R4；④ G14 `review-ledger.json` 只有 R1-R4 当前且 PASS 才可 `CLOSED`，否则停在 partial/blocked；⑤ Rust Cargo 在完整 Helix phase 收口时集中执行，不做单点编译试错循环 | 赌注 高 | 可逆 是 | @2026-07-26T04:15:29-0700
✅ DONE G14 已完成 Helix、PC、Mobile 三仓纵向闭环并由 SpecKit event 30 `CLOSED`：Helix full-flow/interaction/Golden 14/14，PC focused 42/42、Mock 2/2、single-real 1/1、dual-real 1/1、report 1/1，Mobile G14 surface-runtime 与 production-linked unit 均 6/6，Strict Matrix 27/27、共享 Venus、V1 hard-cut 与三仓 6 份 Fixture byte-exact parity 全绿。G13 channel read 与 G14 post read 保持独立，平台无第二业务 reducer；三仓已分类提交，未 push | @2026-07-26T06:20:00-0700
⚡ DRIFT-PRED Mobile 首轮字段级合同暴露 Helix read receipt 的 `completedAt` 可缺省而 SurfaceRuntime 对所有 receipt 一律强制时间；PC 首轮 focused Jest 也因漏指定 WDIO config 报 No tests found，失败日志已保留 → 候选教训：字段必填性应按 producer kind 建模，跨测试根执行 Jest 必须显式钉住 suite config | @2026-07-26T06:20:00-0700
🎯 EXPECT 串行实施并关闭 G14a 模板已收到；可证伪断言：① `im_template_received` HTTP 仅 admission，权威 `post_update` 必须经 `PersistAtomic -> PersistOk` 后由生产 timeline composer 产出 templateReceived/templateConfirmedCount/readerIds Frame；② PC/Mobile 删除本 Gate 对 raw `im:post:updated` 和 ViewModelStore reducer 的验收依赖，只机械 apply 同源 Fixture；③ Helix canonical、PC Mock/Real/report、Mobile production-linked GTest/QuickJS typed view 与三仓 parity 全绿后才 R4；④ 不新增模板确认 receipt surface，因为当前 Go authority 只支持 timeline post_update；⑤ 任一真实链无法闭合则保持 blocked，不用旧 UC runner 或 Mock 冒充 | 赌注 高 | 可逆 是 | @2026-07-26T06:25:00-0700
🎯 EXPECT 串行实施并关闭 G16 关闭/退出群；可证伪断言：① 关闭与退出保持独立命令和权限语义，均只由生产 `ImModule::handle` 经权威回执、Persist barrier 与 `Effect::Emit` 产出 canonical Frame；② PC/Mobile 只机械 apply Helix Frame，不新增业务 reducer 或 raw fallback；③ Helix full-flow/Golden、PC Mock+single/dual real+report、Mobile production-linked GTest/QuickJS typed view 与三仓 hash parity 全绿后才进入 R4；④ G16 未 `CLOSED` 前不得进入 G16b；⑤ Rust Cargo 仅在完整 Helix phase 收口时集中执行 | 赌注 高 | 可逆 是 | @2026-07-26T21:18:00-0700
✅ DONE G16 已完成 Helix、PC、Mobile 三仓纵向闭环并由 SpecKit event 23 `CLOSED`：Helix full-flow/interaction/Fixture 10/10，PC focused 31/31、Mock 2/2、single-real 1/1、dual-real 2/2、report 1/1，Mobile Frame 6/6、focused contract 8/8、fresh-runtime UC16 9/9、共享 Venus/source pin/V1 hard-cut 全绿；三仓五份 Fixture 与 provenance byte-exact，required pass=6/fail=0/skip=0。PC 首次 dual-real readiness 失败及并发 Mobile runner 期间的重叠 candidate 失败原文均保留，不覆盖独立 Green candidate；三仓已分类提交，未 push | @2026-07-26T23:24:46-0700
🎯 EXPECT 串行实施并关闭 G16b 成员退出广播；可证伪断言：① `im_channel_leave` 的本地 self-left 与 WS `channel_member_update.memberChange.leave` 的 roster 广播保持独立 correlation，后者仅在 `channel_member` 持久成功后刷新已 attach `message.member-roster`；② canonical Fixture 只来自生产 `ImModule::handle -> Effect::Emit`，PC/Mobile 不消费 raw `im:channel:members` 重建业务状态；③ Helix full-flow/Golden、PC Mock+single/dual real+report、Mobile production-linked GTest/QuickJS typed view 与三仓 hash parity 全绿后才进入 R4；④ G16b 未 `CLOSED` 前不得进入 G16c；⑤ Rust Cargo 只在完整 Helix phase 收口时集中执行 | 赌注 高 | 可逆 是 | @2026-07-26T23:30:00-0700
✅ DONE G16b 已完成 Helix、PC、Mobile 三仓纵向闭环：Helix full-flow/Fixture/resync regression 10/10，PC focused 32/32、Mock 3/3、single-real 1/1、dual-real 1/1、report 1/1，Mobile Frame 6/6、UC16 focused 1/1、source pin/Command API/V1 hard-cut 全绿；三仓三份 Fixture 与 provenance byte-exact，required fail=0/skip=0。三仓已分类提交，未 push | @2026-07-27T01:00:00-0700
⚡ DRIFT-PRED 首轮 single-real 暴露 renderer detach 后显式 roster resync 错发带 baseRevision patch，修复为显式 snapshot、内部 authority refresh 保持 patch；双端前置还暴露 Observer 未收到 channel-created dialog，因此 G16b focused dual 以 Observer 真实 Tauri leave ingress 证明 Actor WS/FrameApplied/typed roster，不越界冒充 Observer UI lifecycle → 候选教训：显式 resync 必须重建无 base snapshot；Gate 前置失败与目标广播证据必须分开建模 | @2026-07-27T01:00:00-0700
✅ DONE G16c 已完成 Helix、Go、PC、Mobile 四仓纵向闭环并由 SpecKit event 24 `CLOSED`：Helix full-flow 6/6 与 canonical Fixture 通过，Go focused/build/vet 通过，PC focused 34/34、Mock 3/3、single-real 1/1、dual-real 1/1、report 1/1，Mobile Frame 5/5、UC16 focused 1/1、source pin/Command API/V1 hard-cut 全绿；三仓六份 Fixture byte-exact，required fail=0/skip=0。四仓已分类提交，未 push；真实链使用隔离 Go 18066 同步发布 fallback，未声明 Pulsar broker plane | @2026-07-26T20:45:00-0700
⚡ DRIFT-PRED PC 首轮显式 `CSES_E2E_IM_HOST` 被 persona 默认值覆盖，修复为保留隔离 endpoint；dual-real 首轮还错误让普通成员 Observer 写群设置，最终按权限语义改为管理员 Actor 写、Observer 收广播。Go 审查额外发现幂等重放 payload 可伪造 NOTICE 分类与批内投影早退 → 候选教训：测试进程环境必须穿透到真实 child；双端写方先按 authority 角色建模；提交后恢复只信持久 provenance | @2026-07-26T20:45:00-0700
🎯 EXPECT 继续执行 Spec 047 剩余 T038-T090，并先完成 P2/G-07；可证伪断言：① `expediteMap` 以远端 `post.expedite_map` 和本地 `message.expedite_map` 的绝对事实原样持久化，不增加 urgent 派生别名或 capabilities；② durable `im:post:updated` 仅在 message+cursor 原子 PersistOk 与本地读回后 Emit，sender/receiver 双视角仅按 actor 解读同一绝对状态；③ 同序重放不重复 Emit、乱序通过 MessageV3 gap 收敛、冷启动可从本地行恢复；④ 每个后续 Gate 均按 tasks.md 红测→接码→生成物→旧链删除→验证→分类提交，最终 30 Gate MIGRATED 且 Helix 生产源码物理清零 Frame/Surface/旧 V1；⑤ 不修改 cses-client/Mobile，不触碰用户已删除的 `specs/UC04-16-EXECUTION-INDEX.md`，只有真实 blocker 才停止 | 赌注 高 | 可逆 是 | @2026-07-29T12:00:00-0700
✅ DONE G-07 已硬切为 `post_update.expediteMap -> message(expedite_map,event_seq,update_at)+cursor PersistAtomic -> PersistOk -> durable row Get -> im:post:updated`：sender 发起 urgentPost、receiver 发起 urgentConfirm 均只把 HTTP 当 admission，两边独立状态机依次收到 pending/completed 的同一绝对 map；同 seq 幂等、乱序确认、冷启动读回和 canonical 双视角均通过。旧 G07 Frame/receipt 测试、support 与 fixture 已删除；G07 4/4、G06 4/4、server 12/12、sync 12/12、projection 47/47、partial schema、cargo check、hot-path audit、cutover self-test 与 diff check 全过；PC/Mobile/Go 真实双账号未运行 | @2026-07-29T07:40:00-0700
⚡ DRIFT-PRED 首次生成时把 `HELIX_GATE_ARTIFACT_DIR` 传成相对路径，Cargo test 进程以 crate 目录为 cwd，导致 G07 产物写入 `crates/helix-im/specs/...` 且根目录 schema 校验找不到 G06；改用仓根绝对路径后通过 → 候选教训：跨 test target 的 artifact 环境变量必须固定绝对仓根路径 | @2026-07-29T07:40:00-0700
✅ DONE G-14 已硬切为 `post_read -> message.read_bits Persist -> PersistOk -> viewer-aware im:post:read/im:post:readers`：Reader 只收到本人已读终态，消息作者/观察者只收到 Reader 集合；该无序权威事实不推进 channel cursor，重复或旧 revision 幂等，失败不 Emit，冷启动可由 durable row 重建。旧 G14 Frame/receipt 测试、support 与 fixture 已删除；G14 4/4、partial schema 10/30、cargo check、hot-path audit、cutover self-test 与 diff check 全过；PC/Mobile/Go 真实双账号未运行 | @2026-07-29T08:30:00-0700
⚡ DRIFT-PRED 首轮热路径审计发现 `post_read` 为相关状态和持久写重复 clone 线性字段；改为预计算 write op 并移动 owned 字段后清零，同时将新 PersistOk continuation 从含旧 Frame 的共享 `dispatch_read.rs` 拆到独立 MessageV3 owner，避免 migrated owner 被旧壳污染 → 候选教训：每个 Gate 的 production owner 必须自身零旧壳引用，共享文件里的新分支不等于物理硬切 | @2026-07-29T08:30:00-0700
✅ DONE G-14a 已硬切为 `im_template_received HTTP admission -> post_update.props.template.userIds -> message(props,event_seq,update_at)+cursor PersistAtomic -> PersistOk -> durable row Get -> im:post:updated`：sender/receiver 只收到本地表真实 props 绝对态，不再携带 `templateReceived/templateConfirmedCount/readerIds` 或 capabilities；同 seq 幂等、乱序续接、失败不 Emit、冷启动读回和 canonical 双视角通过。旧 G14a Frame/VM 测试、support 与 fixture 已删除；G14a 5/5、partial schema 11/30、cargo check、hot-path audit、cutover self-test 与 diff check 全过；PC/Mobile/Go 真实双账号未运行 | @2026-07-29T09:05:00-0700
✅ DONE G-08 已硬切为 `im_create_schedule HTTP admission -> creator post_schedule_created -> channel_schedule+channel PersistAtomic -> PersistOk -> durable schedule Get -> im:channel:schedule-created`：creator 只收到真实 schedule row 与 `hasSchedulePost`，observer 对同一广播零写零 Emit；HTTP/Persist 失败、重复 revision 与冷启动收敛。旧 G08 Frame/action 测试、support 与 fixture 已删除；G08 5/5、partial schema 12/30、cargo check、hot-path audit、cutover self-test 与 diff check 全过；Go 真实广播隔离未运行 | @2026-07-29T09:40:00-0700
✅ DONE G-09 已硬切为 `im_cancel_schedule HTTP admission -> creator post_schedule_canceled -> channel_schedule+channel PersistAtomic -> PersistOk -> durable schedule Get -> im:channel:schedule-canceled`：creator 收到取消绝对态，observer 对同一 creator-private 广播零写零 Emit；取消清空正文、关闭 `hasSchedulePost`，失败不 Emit，旧 create revision 不复活，冷启动由 durable row 重建。旧 G09 Frame/projection 测试、support 与 fixture 已删除；G09 5/5、G08 5/5、partial schema 13/30、projection 44/44、cargo check、hot-path audit、cutover self-test 与 diff check 全过；Go 真实广播隔离未运行 | @2026-07-29T13:10:00-0700
✅ DONE G-10 已硬切为 `message local Scan -> remote fallback -> message cache PersistOk -> im:timeline:ready`：双 persona 各自收到只含窗口状态和真实 post/message 行的事件，不再收到 Frame/Surface/mutation/action；HTTP 失败为显式 failed，建立 coverage 后本地 Scan 可恢复且不再发 HTTP。旧 G10 Frame 测试、support 与 fixture 已删除；G10 5/5、partial schema 14/30、cargo check、hot-path audit、cutover self-test 与 diff check 全过；真实 SQLite 与消费者同步未运行 | @2026-07-29T13:45:00-0700
✅ DONE G-10a 已硬切当前生产 `TimelineNavigationState::Older -> HTTP authority -> message PersistOk -> matching durable row Get -> im:timeline:update`：sender 只收到去除 anchor 的原始 older page，receiver 为空；HTTP/Persist 失败发布空 messages 的 failed 绝对边界，重复 correlation 为 no-op，不再携带 Frame/Surface/action/capability。旧 G10a Frame 测试、support 与 fixture 已删除；G10a 5/5、G10 回归 5/5、partial schema 15/30、cargo check、hot-path audit、cutover self-test 与 diff check 全过；真实 SQLite 与消费者同步未运行 | @2026-07-29T14:35:00-0700
⚡ DRIFT-PRED 首轮误改了已无生产入口的 `LoadOlderState` continuation，而当前 `im_load_older_context` 已统一路由到 `TimelineNavigationState::Older`，因此测试仍观察到旧 Frame；沿 command→correlation 图重新定位后改在真实导航 owner，并把远端 page 与单行 durable barrier 分离 → 候选教训：Gate 迁移先从公开 command 反向锁定当前 correlation variant，不能因历史类型名相同就沿旧 continuation 接码 | @2026-07-29T14:35:00-0700
✅ DONE G-10b 已硬切为 `TimelineNavigationState::Newer -> HTTP authority -> message PersistOk -> matching durable row Get -> im:timeline:update`：sender 收到去除 anchor 的 20 条原始 newer page，receiver 为空，hasNewer=false 明确尾部耗尽；HTTP/Persist 失败不泄漏 ready，重复 correlation 为 no-op。旧 G10b Frame fixture 与合同测试已删除；G10b 4/4、G10a 回归 5/5、partial schema 16/30、cargo check、hot-path audit、cutover self-test 与 diff check 全过；真实 SQLite 与消费者同步未运行 | @2026-07-29T14:55:00-0700
✅ DONE Spec 047 的 T001-T090 已全部完成：30 Gate 均为 `MIGRATED`，真实 Rust target 生成 120 份 `.v1` JSON，tree SHA-256=`0969896be01dc28bcc63aafe62894cd31ce4ed02947dcbaeb888c756397ee0fc`；Helix 生产、driver 与当前测试已物理下线旧 UI 协议、V1 compatibility 和独立 FFI action ABI。core/im/native/FFI/Web check、helix-im/core/FFI tests、Web wasm、cutover self-test、diff/ignore 与 R4 evidence 均通过；PC/Mobile 保持 `NOT_SYNCED_BY_SCOPE`，WDIO/GTest/runtime/dual-user/canonical consumer 为 `NOT_RUN`，未 push | @2026-07-29T11:20:00-0700
⚡ DRIFT-PRED 最终收口额外暴露 FFI 仍引用已删除 Surface command，以及历史 G-07 evidence manifest 的 SHA-256 与原始文件不一致；删除旧 ABI、改为 MessageV3 bytes 透传并校正登记元数据后 R4 通过 → 候选教训：共享协议物理下线必须把跨端 ABI 与证据清单 hash 一并纳入最终扫描 | @2026-07-29T11:20:00-0700
🎯 EXPECT 梳理当前在线与离线发送消息全流程；可证伪断言：① 文档只依据当前 `reactor/MessageV3-helix` 的真实 Command/Tick/Effect/driver/PortReply/storage/EventSink 路径，不把已删除 Frame/Surface 或过期 sample 当现状；② 明确 HTTP SUCCESS 不等于 Sent，只有合法 WS authority、持久提交与读回后才产生 terminal MessageV3 Emit；③ `.md` 与 `.mmd` 覆盖正常发送、断网失败/超时、timer、重试、重连补偿和 sender/receiver 分流；④ 所有源码链接存在且 Mermaid 可解析，`git diff --check` 通过 | 赌注 中 | 可逆 是 | @2026-07-29T20:55:10-07:00
✅ DONE 已在 `crates/helix-im/docs/消息发送全流程/` 交付中文 `.md` 与 Mermaid `.mmd`，按当前 Helix 与现役 Go 源码串起 Command、Persist、媒体预签名/PUT、HTTP、timer、PortReply、Go Pulsar/事务/WS、sender/receiver 未读、离线 sync 与 retry；README/AGENTS 已标明 Frame/Surface/V1 为历史材料。G-01 2/2、G-03a 2/2、媒体 active 9/9 通过（11 个旧媒体合同 ignored），Mermaid CLI 解析及 `git diff --check` 通过；未执行真 Go、双账号 WS、真实 SQLite 文件或 Mobile GTest | @2026-07-29T21:11:40-07:00
⚡ DRIFT-PRED 开局预期“无合法 server id 不进入 Sent”，但当前 Helix 在 Go 正常无 id 的 `SUCCESS` 回包后会写 `send_status=sent` 并撤 timer，仅保留 PendingSend 等 WS echo 补 id；文档已如实列为待修差异而非冒充理想合同 → 候选教训：发送验收必须拆开 HTTP admission、本地状态与 WS/DB authority | @2026-07-29T21:11:40-07:00

🎯 EXPECT Spec006 G02c 显式 retry 的 HTTP admission 后只通过 Go 权威历史结算；`getLatestPost.pageSize` 必须保持在 timeline V3 的 1..=60 合同内，命中同 temporaryId 与合法 server id 后取消 timer、清 pending、持久化 canonical id；普通首发 HTTP 仍不得直接标 Sent | 赌注 高 | 可逆 是 | @2026-07-31T21:35:00-07:00

✅ DONE 已为显式 retry 接入权威 `getLatestPost` continuation，并把读回页长截断到 Go timeline V3 最大 60；真实服务复现旧实现返回 `status=failed, invalid timeline query: pageSize`，生产破坏去掉上限后聚焦测试精确红（61 != 60，exit 101），恢复后页长合同 2/2 与发送 HTTP/WS/readback 9/9 全绿。Native WDIO 候选与 G02b/G02c 真实 lane 将继续重建复跑，不在此条提前晋级 real PASS | @2026-07-31T21:36:00-07:00

🎯 EXPECT Spec006 转发 picker 的权威频道补偿查询在冷启动 18K+ 投影突发下仍返回目标群；可证伪断言：① `im_channel_query.condition.display_name` 出站严格还原为 Go `displayName`；② Native 默认有界事件窗口覆盖本轮已观测突发；③ Helix focused read/query、Tauri helix_im 与 dual-real-forward 全绿后才记账 T029 | 赌注 中 | 可逆 是 | @2026-08-01T05:10:00-0700

🎯 EXPECT 新增 MessageV3 Phase1-4 的 84 个 Phase→Domain→Gate Rust real-chain target；可证伪断言：① manifest 的 33/20/15/16 Gate 与 Cargo `--list` 逐项一致；② Go 8066 可执行的 Phase1 Gate 串行运行时 HTTP `SUCCESS`、WS、SQLite terminal 和 `im:__bus__` 同时成立；③ media/retry/draft/forward/Phase2-4 前置缺失只产生 `NOT_VERIFIED`/`NOT_APPLICABLE`，不伪造 PASS；④ 新 target 编译、真实 Phase1 focused run、ignored evidence run 与 `git diff --check` 均通过 | 赌注 高 | 可逆 是 | @2026-08-11T12:00:00+0800

## 历史记录（2026-08-06：频道群设置版本读回修复）

> **历史资料**：该条记录描述 CAS 下线前的失败路径；2026-08-07 fix worktree 已物理移除 channel/channelMember 写入版本条件。当前有效语义以 helix-im outbound、contracts 和三仓 fix worktree 为准。

- 目标：修复 legacy Helix `channel` 行缺少 `settingVersion/permissionRevision` 导致 MessageV3 在进入 Tauri command 前抛出 `CHANNEL_SETTINGS_EXPECTED_VERSION_REQUIRED`。
- 可证伪断言：`render_ready::channel::shape_channel_row` 对旧行以 `updated_at` 补出两个版本字段；客户端群资料/公告/权限控件在权威版本缺失时保持 fail-closed；聚焦测试、Rust 检查和前端 lint/test 可复现通过。
- 边界：不修改 Go 合同、不改本地 DB schema、不触碰客户端既有 release 文件；真实宿主重启与双用户运行态需单独采证，不以单测替代。

✅ DONE `fbeb41d4` 已完成 MessageV3 Phase1-4 的 84 个 Phase→Domain→Gate Rust real-chain target；当前真实账为 Phase1 19 PASS、13 NOT_VERIFIED、1 NOT_APPLICABLE，Phase2-4 共 51 个 NOT_VERIFIED。所有未具备真实 Angular/Go/WS/SQLite 前置的 Gate 均未伪造 PASS；运行证据保留在未跟踪的 `artifacts/messagev3-phase1-4-real-chain/`，不进入提交。@2026-08-11T12:20:00+0800

🎯 EXPECT 继续把 Phase1 未验证项与 Phase2-4 的真实 Rust native Gate 逐批接入 localhost:8066；可证伪断言：① 每个已解锁 Gate 都由 Tick/Helix Effect/Go HTTP/真实 WS/SQLite/Emit 形成同轮证据；② Go route 或外部 producer 缺失只落 BLOCKED_PRECONDITION/NOT_VERIFIED，不伪造 PASS；③ 共享 harness 只扩展真实适配器，不修改原始 Helix、Go 或 Angular 生产代码；④ 每完成一个 Domain 都有独立聚焦运行、cargo check、diff check 与分类 commit；⑤ 最终报告仍保留 84 Gate 分母。@2026-08-11T12:30:00+0800

✅ DONE Phase1-4 的 84 个 `PhaseN/Domain/Gate` Rust real-chain 测试均已默认执行；Phase1/2/3/4 focused run 分别为 33/20/15/16 passed、0 failed。最新 evidence 汇总为 50 PASS、18 BLOCKED_PRECONDITION、11 NOT_VERIFIED、5 NOT_APPLICABLE；Go media、timeline、Pin、Announcement persistence、Search/Todo 合同缺口均保留了逐 Gate 证据，没有把 harness 通过冒充业务 PASS。修复并验证了 G06b 双 Helix 会话竞态、G09c actor 身份与 `im:channel:member-nickname` 事件名；本批次尚未 push。@2026-08-11T14:27:50+0800

⚡ DRIFT-PRED 本轮实际需要修改 Helix 生产事件名与 G09c 身份处理，且 Phase2-4 不能继续以 `#[ignore]` 作为默认前置闸门；候选教训：真实 Gate 的合同修复必须和测试 harness 分开提交，缺口应在运行证据中显式标注，而不是通过 ignored 测试或旧文档统计隐藏。@2026-08-11T14:27:50+0800

🎯 EXPECT 沉淀 Phase1-4 BLOCKED/NOT_VERIFIED/NOT_APPLICABLE 的逐 Gate 证据缺口与 handoff；可证伪断言：① 新增两份中文 Markdown，逐 Gate 对应最新 evidence.json；② 不把 Cargo harness 通过或 health 200 写成业务 PASS；③ README 链接、相对/绝对源码引用和 `git diff --check` 全部通过；④ 不改 Go/Angular 生产代码。@2026-08-11T14:39:29+0800

✅ DONE 已新增 `Phase1-4-非通过Gate证据缺口清单.md` 与 `handoff.md`，覆盖 34 个非 PASS Gate 的当前事实、缺失证据、跨仓 owner、清障顺序和重跑命令；README 已挂接入口，Markdown 链接检查与 `git diff --check` 通过，Go/Angular 工作树未修改。@2026-08-11T14:39:29+0800

🎯 EXPECT 推进 Phase1-4 真实 Angular→Helix→Go 8066→WS→SQLite→emit 证据，按用户明确排除 Todo、任务/会议卡片、平均分后收敛可验收 Gate。可证伪断言：① 不改写 84 Gate 原始状态，只新增按排除清单过滤的证据汇总；② 先修复已被源码与运行错误共同定位的 Go media.type、再逐层复验 Pin/Timeline/Search/本地 N/A，所有 PASS 必须有当前窗口的 HTTP/WS/Storage/Emit 或真实 Angular 终态证据；③ 不以 health、HTTP 200、Rust harness 通过或旧 artifact 冒充业务 PASS；④ 修改三仓时保留既有 dirty/untracked，按 Go/Rust/文档独立批次聚焦验证并提交，不自动 push；⑤ 若外部 PG/Pulsar/ES/认证态或运行中的 8066 二进制无法安全更新，记录 BLOCKED 而不是伪造全绿 | 赌注 高 | 可逆 是 | @2026-08-11T14:50:59+0800

✅ DONE 直接修正 Phase1 Upload 测试类且未启动 WDIO：G01c/d/e/g 以 Angular Java/fileDrive/OSS 完成后的稳定媒体引用真实打 Go `posts/create`，G03c 以 `postIds + channelIds` 真实转发，全部聚焦测试通过并取得 WS/Helix/SQLite/emit 子链；G02d 以 Helix planner 精确拒绝 67,108,865 bytes 并取得零副作用 PASS。最新账为 Phase1 21 PASS / 6 BLOCKED_PRECONDITION / 5 NOT_VERIFIED / 1 NOT_APPLICABLE，总计 51/23/5/5；媒体完整 Gate 仍因本轮未取 Java/fileDrive/OSS 字节级同轮证据保持 BLOCKED。@2026-08-11T15:20:00+0800
⚡ DRIFT-PRED 原先把 Go `/media/upload/prepare` 的 `media.type NOT NULL` 当作 Angular 媒体链根因；核对 Angular `FileService`、Java/fileDrive/OSS 与 Helix strict matrix 后改为稳定引用→Go post 子链，旧 Go-owned probe 保持忽略并降级为独立诊断 → 候选教训：媒体 Gate 必须先锁定真实上传 owner，再决定 Go 是字节平面还是 post consumer。@2026-08-11T15:20:00+0800

🎯 EXPECT 修复 MV3-G10e notify-only `update_channel` 的 HTTP→Go WS→Helix PersistOk→SQLite→`im:channel:update` 收敛；可证伪断言：① `NORMAL/STRONG/IGNORE` 只更新当前 viewer 的通知字段，缺失/非法/混合 payload fail-closed 且不改 unread/top；② event 与 query 统一输出当前真实 scalar `notifyProps`；③ Helix focused/native G10e 回归和 `git diff --check` 通过，真实 iOS SDK 不具备时明确 BLOCKED_IOS_TOOLCHAIN，不伪造 artifact hash | 赌注 高 | 可逆 是 | @2026-08-12T00:00:00+0800
✅ DONE 已修复 notify-only WS 的严格三态解析、channel/member 双层持久化、PersistOk 后 durable readback 事件与 scalar `notifyProps` 合同；Helix G10e 4/4、P4 WS 10/10、core 16+、三端 check、wasm、Native target 1/1 均通过。Native 真链本轮因运行中 Go `/api/cses/channel/member/change/notify` 返回 `status=failed,message=not found` 留在 `BLOCKED_PRECONDITION`；iOS SDK 不可用，未伪造 mobile artifact hash。@2026-08-12T00:00:00+0800
⚡ DRIFT-PRED 真实运行态额外暴露 Go 8066 当前二进制未挂载 notify route，故 HTTP→Go DB 之后的 WS/Helix/SQLite 证据无法取得；需重启/部署含该 route 的 Go binary 后重跑 Native G10e，不把 Helix focused 或 HTTP health 当作跨仓 Green。@2026-08-12T00:00:00+0800
🎯 EXPECT 修复 driver-host SQLite URI 读写目标漂移：可证伪断言为 `sqlite:/tmp/...?...mode=rwc`、`sqlite:file:`、Unix/Windows drive/UNC 路径经同一 canonical target 打开后 writer 创建的数据可由 reader 读回；`mode=memory`/`cache=shared` 保留；聚焦 connection 单测、host check、热路径审计和 `git diff --check` 通过，旧用户 artifacts 不纳入提交 | 赌注 高 | 可逆 是 | @2026-08-13T

✅ DONE 文本接龙双投影与分页详情已收敛：Go 同事务维持一条 `DECLARATION` 与每作者一条 `TEXT_CHAIN/PARTICIPANT`，同作者复用稳定 `entryId/postId/seq`，GET 首页固定 20 并返回 `hasMore/nextCursor`；客户端补齐 live/普通 Post 双事件源去重、canonical GET 历史参与消息回灌、详情 Signal 累计分页与滚动加载。Go 已 `make docker-up-clean` 重建 18066 并通过聚焦测试/vet/build；最新 single-real run `phase5-single-real-1787485802237-21c31d11` 为 8/8 PASS 且录屏留证，dual-real run `phase5-dual-real-1787485547432-eed0df8e` 的 actor/observer 操作与一声明两参与卡断言 PASS，仅不可达的两条内容折叠 Gate G04 保持 BLOCKED；Helix 生产代码无需修改，三仓既有 dirty/untracked 已保留，按仓提交且未 push。@2026-08-23T19:52:00+0800
🎯 EXPECT 产出 Spec 050 业务指标交接文档；可证伪断言：① 逐项列出 12 个指标的 Owner、原子指标、当前 PromQL、Grafana 面板和完成度；② 明确 ACK/未读 Planned、端到端延迟分段、恢复失败与重复乱序/全 Command 的覆盖缺口，不把面板存在写成生产 SLO 已闭合；③ 给出 PrometheusRule 加载、Dashboard 导入、无数据排查和真实链验收步骤；④ 只新增 `specs/050-messagev3-business-observability/handoff.md` 与本行 SESSION 记录，保留既有 artifacts | 赌注 中 | 可逆 是 | @2026-08-24T20:29:00+0800

✅ DONE Spec 050 `handoff.md` 已逐项说明 12 个业务指标的 Owner、原子指标、采集终态、PromQL、Grafana 查询和当前完成度；明确 ACK/未读 Planned、端到端延迟分段、恢复失败与 lost/duplicate/order/全 Command 覆盖债务，并给出 PrometheusRule 加载、Dashboard 导入、无数据排查及真实链十步验收。12 节/必需指标/相对链接、Dashboard 合同与 `git diff --check` 均通过；未执行真实集群 apply 或 Grafana 写操作，既有 artifacts 保持未跟踪 | @2026-08-24T20:33:36+0800

🎯 EXPECT 修复回复后 root 消息自端样式丢失；可证伪断言：① `posts_update` 经 `PersistAtomic → PersistOk` 发出的 `im:post:batch-updated` 按当前登录用户塑形；② root `userId` 等于 viewer 时 `data.posts[].isSelf=true`；③ 事件名、外层字段、持久化屏障、事件顺序与 Angular 合并合同保持不变；④ 公开 `ImModule::handle` 回归先红后绿，Helix 全量回归中的无关基线失败单独报告 | 赌注 高 | 可逆 是 | @2026-08-26T00:00:00+0800

✅ DONE `posts_update` 已把 `auth_user_id` 传入批量 render-ready 投影，当前 viewer 的 root 更新保持 `isSelf=true`；公开 Handler 回归已完成红→绿，聚焦测试 1/1 与 `git diff --check` 通过。`cargo test -p helix-im` 运行到既有 `cp_create_posts_test` 时因注册表实际 137、断言 136 失败，该文件与本次 diff 无交集；未执行桌面 Native/UI 复验，未把 sans-IO 结果冒充真实界面 PASS | @2026-08-26T00:00:00+0800

🎯 EXPECT 将 visual Helix 的四个 Rust 库以 `0.1.0` 发布到 crates.io；可证伪断言：① 只把已被占用的 `helix-core` package 改名为 `cses-helix-core`，依赖 key 与 Rust `helix_core` import 不变；② `helix-im` / `helix-driver-host` / `helix-driver-native` 按 core → im/host → native 拓扑逐包 dry-run 和发布；③ 四个包只携带 `src` 与发布 README，不公开 tests/docs/artifacts/真链证据；④ 许可证为 `MIT OR Apache-2.0`，不读取、输出或提交 crates.io token；⑤ 客户端最终四条依赖的 `cargo metadata` source 全部为 crates.io registry，若任一包上传或真客户端 check 失败则停在 partial，不冒充完成 | 赌注 高 | 可逆 发布前是/发布后否 | @2026-08-26T17:46:28+0800

✅ DONE `cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 已按依赖拓扑发布 `0.1.0` 到 crates.io，四个远端包均可由 `cargo info` 解析且许可证为 `MIT OR Apache-2.0`。visual 客户端已把四条本地 path 依赖切换为精确 registry 版本，`cargo metadata` 显示四包 source 均为 crates.io registry，完整 `cargo check --manifest-path src-tauri/Cargo.toml` 通过；客户端仓明确忽略 `src-tauri/Cargo.lock`，未强行纳入版本控制，既有 dirty/untracked 文件均未触碰。@2026-08-26T18:35:00+0800

🎯 EXPECT 新增四 crate 一键发布脚本；可证伪断言：① 默认只把同步版本 patch +1，并同步所有内部精确依赖；② 发布前拒绝 tracked dirty、版本不一致、目标版本占用和许可证漂移；③ 按 core → im → host → native 逐包 dry-run、上传并等待 registry 可解析；④ 首次上传前失败自动恢复 manifest/Cargo.lock，上传后失败保留版本并报告 PARTIAL；⑤ 不读取或输出 token、不自动 commit/tag/push，纯函数单测、plan 与 `git diff --check` 通过 | 赌注 高 | 可逆 首次上传前是/之后否 | @2026-08-26T18:50:00+0800

✅ DONE 已新增 `scripts/publish_crates.py`：默认 patch +1，支持 minor/major/显式版本，发布前校验 tracked clean、crate 内无未跟踪文件、四包同步版本、双许可证、当前版本可解析与目标版本未占用；按 core→im→host→native 逐包 dry-run、上传并等待 registry。首次上传前失败按字节恢复四份 manifest 与 Cargo.lock，上传后失败保留新版本并报告 PARTIAL；不会输出 token 或执行 commit/tag/push。4 个纯函数/真实 manifest 回归通过，`--plan --allow-dirty-plan` 已确认 0.1.0 四包可解析且 0.1.1 四包未占用，未实际发布 0.1.1，`git diff --check` 通过。@2026-08-26T19:05:00+0800

⚡ DRIFT-PRED `gate-fast` 因历史 Cargo 边界 diff 自动升级完整 gate，三端 check、wasm 与 clippy 通过后，在既有 `helix-driver-ffi` 空库异步 dialog query 测试失败（26/27）；失败文件与本次发布脚本 diff 无交集，按非 release 停止扩张规则保留为独立基线债务，不修改 FFI 生产代码。@2026-08-26T19:10:00+0800

🎯 EXPECT 收敛 MessageV3 业务可靠性面板的真实采集与展示；可证伪断言：① dev Prometheus 同时保留 `environment=local|dev` 两类 UP target，recording rule 不丢环境标签；② 远端落库成功率来自事务终态 counter，新增远端成功落库每秒量且不把 dedup 冒充物理新增；③ ACK、同步、恢复、未读、正确性和 Command 面板只有真实生产终态样本才变绿，不使用常量或 `or vector(1)`；④ Dashboard 不再显示“待接入”，缺真实协议/流量的能力必须完成跨端采样或如实保持非绿；⑤ Go、Helix、PrometheusRule 与 Dashboard 分批聚焦验证并保留既有 dirty/untracked | 赌注 高 | 可逆 是 | @2026-08-26T20:30:00+0800

✅ DONE dev 真链已闭环：Go 当前 Pod 的远端事务、物理新增、Outbox、入队、ACK 与 Command 终态进入 Prometheus；Helix dev/native 的 ACK、投影、同步、恢复、同 scope 未读对账和正确性分母均有增长样本；PrometheusRule generation=4；Grafana 默认 dev 浏览器逐卡回读为 9 项成功率 100% 与 3 项异常率 0%，远端落库吞吐显示真实 ops/s。local target 保留但仍是用户 GoLand 旧二进制，未越权替换；恢复失败类型、三类 correctness、全 Command 和逐消息 E2E 延迟仍按覆盖债务记录。@2026-08-26T20:22:13+0800
✅ DONE Issue #135 统一 type=1 在线与 Routine Sync 的 viewer-scoped 事件应用；证据：Sync/在线共用 `message_v3_post_mutation_ops`，Routine Sync 删除共享 channel 未读 bump，PersistOk 后按 `(channel_id,user_id)` 读回并只发一个绝对 `im:channel:update`；`s4_write_order_test` 6/6、`message_v3_sync_native` 1/1、diff check 通过；全仓 fmt 仅受既有非租约文件阻塞，Go/Angular/release 配置零改动 | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #136 对齐小 gap 与大 gap 补齐；证据：driver-native 真实 Tick::Inbound 证明 seq2 先缓冲且 cursor=0，seq1/seq2 仅在各自 PersistOk 后推进并投影 unread 1/2；大 gap Sync 首条 unread=1；PersistErr、迟到伪 Ok、重复回报均 fail-closed；目标 attach 已从 dialog readback 条件移除，`message_v3_sync_native` 3/3、diff check 通过 | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #137 对齐 PongGap、断线 Sync 与 Hydration；证据：Pong fan-out 以 pending Persist 与 dirty channel 集合做 O(1) 登记，整批排空后每频道一次 viewer readback；HydrationHistory 只恢复 message，不二次改未读；严格 roster fixture 补 memberCount 后 G13b 9/9；Pong 内核 3/3、HydrationHistory 1/1、driver-native 4/4（含 SQLite 重启 member/cursor 读回）、diff check 通过 | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #138 收敛 type=2 编辑持久化屏障；证据：在线 G06 4/4、Sync/Chain 编译器 13/13；Sync 无 PersistFire/提前 Emit，pending domain events 只在 matching PersistOk 释放；Native SQLite type2 原子 patch 后 member unread 5 保持 5，driver focused 1/1；无需生产代码改动 | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #139 收敛 type=3 撤回与 type=6 混批；证据：Native SQLite type3+type6 原子批终态 revoke=1/readBits=1/member unread=5；S4 混批 6/6 严格 type1→type2→type3→type6；单列 patch 1/1，重复 corr 由一次性 correlation 消费；无需生产代码改动 | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #140 验证 Sync 批处理性能与失败原子性；证据：driver-native 回放 1/10/100/500 均恰一 PersistAtomic，PersistOk 前 0 readback/0 Emit，StorageOp ≤8N+3；同文件 PersistErr/重复 corr/Native SQLite 重启用例通过，整文件 7/7、diff check 通过 | 赌注 高 | 可逆 是 | @2026-08-28T

🎯 EXPECT Issue #141 运行 Go 18066 双用户真实链回归；可证伪断言：Go worktree docker-up-clean 后 health 仅作前置；两个真实用户与 run-owned 群验证在线第一条1/第二条2、离线与 gap 的同一 channel/event/corr/WS/SQLite/EventSink 证据；清理资源，Go tracked diff=0；focused real-chain PASS 后才进入 #142 | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #141 已完成 18066 双用户真实链：本地 Compose 使用快照内 444/678/445 三个非秘密同租户身份并创建/关闭 run-owned 群；444 第一条真实 post 经 WS→Native Persist→SQLite→EventSink 投影 unread=1，445 的真实 post/read 作为中间缺失事件，444 第二条高序号 post 触发真实 `/channel/sync/notify` 后投影 unread=2；SQLite `channel_member` 与 cursor 精确读回，重复第二帧零新增终态；`message_v3_sync_native` 10/10、Phase5 双设备 Gate 证据为 PASS，Go tracked diff=0 | 赌注 高 | 可逆 是 | @2026-08-28T

🎯 EXPECT Issue #142 只执行本次变更的 focused code review 与聚焦测试；可证伪断言：对 type1/type2/type3/type6、Routine/小 gap/PongGap/Hydration、Persist 屏障、viewer 复合键与批量线性写集逐项审查；Helix focused tests 通过，Go/客户端零业务 diff；记录既有 fmt 基线但不扩大修复；完成后停止并询问是否授权 Phase1-4 在线、离线、gap 与文字接龙整体 Review | 赌注 高 | 可逆 是 | @2026-08-28T

✅ DONE Issue #142 focused review 已关闭全部 actionable finding：删除 gap self-echo 的 pre-Persist 写与 Emit；ordinary type2 gap 统一 `PostUpdateAtomic` 且失败放回 gate；Hydration 缺 authority 时按 per-viewer business-seq guard 重建未读；increment snapshot 单调写且 global batch 原子化；空 viewer/seq overflow fail closed；旧 migration 不再把 channel cursor 伪装成 viewer guard。Native 13/13（含当前 18066 gap 真链）、engagement 5/5、G13b 10/10、S5 5/5、G06 4/4、increment+golden 11/11 通过；全包仅被既有 stress_test 缺 client_platform 编译债务阻断。Spec 051 可关闭，Phase1-4/文字接龙整体仍按 PARTIAL/BLOCKED 单列，不冒充 Green | @2026-08-28T

🎯 EXPECT 发布 Helix 0.1.3 并让 cses-client 应用；可证伪断言：① `cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 四包统一从 0.1.2 升至 0.1.3，按依赖拓扑 dry-run、上传且均可由 crates.io 解析；② workspace 八个版本事务 manifest 与 Cargo.lock 只发生版本同步；③ 客户端四条 registry 精确依赖与 Cargo.lock 锁定 0.1.3，`cargo metadata` source 均为 crates.io 且 `cargo check --manifest-path src-tauri/Cargo.toml` 通过；④ 不输出 token、不自动 tag 或 push，发布失败按脚本如实报告 PARTIAL | 赌注 高 | 可逆 发布前是/发布后否 | @2026-09-01T

✅ DONE Helix 0.1.3 已完成发布与客户端 repin：四个公开 crate 按 core→im→host→native 拓扑完成 dry-run、上传并由 crates.io 逐包解析；workspace 八个 manifest 与 Cargo.lock 仅同步版本事务。cses-client 四条精确 registry 依赖及本地 Cargo.lock 已锁定 0.1.3，`cargo metadata --locked` 证明 source 均为 crates.io，完整 `cargo check --manifest-path src-tauri/Cargo.toml` 通过；旧客户端安装包尚未重打，跨层真实链仍单独保持 NOT_VERIFIED；未 tag、未 push。@2026-09-01T

🎯 EXPECT 修复模板“收到”与普通消息已读的客户端展示串线，并复核退群转让群主、置顶多用户广播；可证伪断言：① 模板 receivedCount/receivedPeople/unreadPeople 只读取 `props.template.userIds`，即使 `readUserIds/unreadUserIds` 已变化也不增加“收到”人数；② 当前三仓转让链保留 `newCreator → new_creator → newCreator`，Go 同事务更新 owner/CREATOR 并向接任者投影管理能力；③ 录屏2 的普通成员权限变更、置顶菜单和双方顶部置顶卡按逐帧事实验收，不把通过场景伪装成缺陷；④ 只有实际修改 Helix crate 才发布新版本并更新客户端外置依赖，未修改 Rust 时不得无意义发包；⑤ 保留 Helix `docs/mmd` 与 artifacts、客户端 WDIO artifacts 的既有未提交内容 | 赌注 高 | 可逆 是 | @2026-08-31T13:00:00+0800

✅ DONE 模板回执串线已以精确红绿回归修复：普通 `readUserIds/unreadUserIds` 不再影响模板“收到”，组件 18/18、相关客户端 33/33 与全量 lint 通过。录屏1 为 dev/pre 1.2.66 混合旧运行态；当前代码的 `newCreator` 转让链与 Helix 3/3 回归通过，Go 真 PG 用例因未设 `CSES_IM_TEST_DSN` 而 SKIP。录屏2 逐帧已显示普通成员获得置顶权限且双端看到置顶/取消置顶，Helix `post_pin` 3/3 通过。本轮无 Rust 生产代码变更，故未发布新 crate；客户端仍从 crates.io 精确依赖 `cses-helix-core/helix-im=0.1.2`。Docker daemon 不可达，当前二进制的真双用户 PG/Native 复验保留为 `BLOCKED_PRECONDITION`，未冒充真链 PASS。@2026-08-31T13:45:00+0800

🎯 EXPECT 收敛切群读取为 Helix 统一本地投影与事件驱动刷新；可证伪断言：① 不新增前端业务真值缓存或另一套消息状态，复用现有 channel/message/channel_member/message_draft/channel_schedule 投影；② 热切群不触发书签、子话题、日程等可延迟远端读取，消息与成员先读本地投影；③ ChannelEvent、Gap、Increment 只更新或失效对应分面，旧快照在同步期间保持可见；④ 聚焦测试、影响审计、文档与 `git diff --check` 通过；⑤ 不创建 worktree、不自动 push，并保护两仓既有未跟踪文件 | 赌注 高 | 可逆 是 | @2026-09-01T

✅ DONE 切群统一本地投影源码已收敛：前端删除书签/日程的切群远端读取，子话题在已有 `subtopicsLoadedAt` 时直接本地物化，置顶为 0 时零读取并以 channel+generation 拦截迟到结果；Helix 新增账号+频道隔离的置顶绝对投影，首次权威回填在 PersistOk 后可见，pin/unpin 事件按 epoch 失效且拒绝迟到旧回包。客户端通过 `im_health` 自报命令做能力协商，0.1.3 不支持时直接走原权威接口。前端 20 条聚焦测试、lint、Rust check、Helix 全包测试（含合同根环境覆盖）与 diff check 通过；新 crate 尚未发布/repin，运行态保持 NOT_APPLIED；未创建 worktree、未 push，既有未跟踪文件未触碰。@2026-09-01T
🎯 EXPECT 将 MessageV3 real-chain 收敛为可配置环境并作为 GitLab CI 唯一执行入口；可证伪断言：① 默认 profile 为 `pre`，Go HTTP/WS 与 Java API 三端点由统一配置解析，`local` 可通过同一配置切换且保留未来 `dev` 扩展位；② runner 仅发现 83 个可执行 Gate（Phase1/2/3/4/5 = 32/16/15/12/8），五个 `NOT_APPLICABLE` 与四个 `NOT_IMPLEMENTED` 永不进入选择集；③ `.gitlab-ci.yml` 不再调用 `scripts/gate.sh` 或 `api-surface`，只运行 `node scripts/messagev3/phase1-5-real-chain.mjs --all`；④ Node 契约、列表/筛选、Rust 编译与 YAML 结构检查通过，不执行会写入 PRE 的真实全量链；⑤ 保留既有未跟踪 artifacts/Prompt，不修改客户端 92 Gate 分母 | 赌注 高 | 可逆 是 | @2026-09-03T

✅ DONE real-chain 已切换为统一 profile 配置：默认 `pre`（Go HTTP/WS/Java API），`local` 可显式选择，未来 `dev` 在提供三端点变量后可直接扩展；runner 与 libtest 同时排除 5 个 `NOT_APPLICABLE` 和 4 个 `NOT_IMPLEMENTED`，可执行分母为 83（32/16/15/12/8）。GitLab CI 唯一业务 job 只执行 `node scripts/messagev3/phase1-5-real-chain.mjs --all`，凭据使用 GitLab File 变量；Node 契约 3/3、列表 83、Cargo skip 85（83 Gate+2 shared）、native check、fmt、YAML 与 GitLab lint 均通过；PRE health 仅证明可达，未执行会写入 PRE 的全量 real-chain，既有 artifacts/Prompt 未纳入 | @2026-09-03T

✅ DONE 远端 `pre` real-chain 已实际执行，suite `messagev3-1788417330333-18171` 选取 83 个 Gate：55 PASS、11 TEST_FAILED、4 NOT_VERIFIED、13 BLOCKED_PRECONDITION；Phase1 为 9/11/4/8，Phase2 为 16 PASS，Phase3 为 15 PASS，Phase4 为 7 PASS/5 BLOCKED，Phase5 为 8 PASS。首次执行发现客户端账号池数组格式与 Native 对象解析不兼容，已补数组/对象双格式解析与 2 个 focused 回归；本轮未把 evidence/artifacts 纳入 Git | @2026-09-03T
🎯 EXPECT 修复 PRE 真实链新频道发送的快照时序与失败清理竞态；可证伪断言：① `run_helix_send` 成功、失败、超时均先释放本轮 engine/WS/SQLite，再执行远端 `/channel/close`；② Go `posts/create` 的接纳时间/快照校验不再因客户端注入的较早 `createAt` 产生首个 `invalid frozen audience snapshot`；③ 迟到 WS 帧只在本轮 channel/temporaryId scope 内隔离，不冒充 PASS；④ 只修改 Helix real-chain shared helper 与 Go 对应 post/snapshot owner/test，保留现有 artifacts/Prompt；⑤ 聚焦 Rust/Go 测试、`git diff --check` 通过，真实 PRE 重跑结果按证据分层如实报告 | 赌注 高 | 可逆 是 | @2026-09-03T

✅ DONE PRE 真实链乱序修复已完成：Helix `run_helix_send` 统一在成功、失败、超时路径先停止 engine/actor+observer WS 并删除本轮 SQLite，再由 caller 关闭远端 run-owned channel；`channel/create` 已拿到 ID 但 projection/snapshot 等待失败时也会回收频道。Go commit `d8db78f` 在活动快照读取后规范化过旧 `createAt`，同频道快照准备→提交→主 post WS 使用 lane，关闭/删除频道的遗留缺快照消息 ACK-and-drop，开态缺快照保留 NACK 重试。聚焦 Rust 2 tests、Go Pulsar/store/send/race tests、gofmt/rustfmt/diff-check 均通过；用户已有 artifacts/Prompt/config/tmp 未纳入 | @2026-09-03T
⚡ DRIFT-PRED PRE 重跑仍只能观察到旧镜像 `pre-paas2.5-647bd290` 的 `TEST_FAILED`（G01A 超时）与关闭后重复 NACK，未部署本修复前不能证明业务 Gate 结果；候选教训：real-chain 必须在 Go 镜像 rebuild/restart 后再按同一 profile 重跑，旧运行态证据不得覆盖新代码证据 | @2026-09-03T

✅ DONE MessageV3 real-chain 已完成 Phase 批次提速与资源复用：`--all --env pre` 固定 83 个可执行 Gate、5 个 Cargo 批次、每 Phase 最多 5 个活跃 run-owned 频道；同一账号在 Phase 内复用一条持久 Helix/WS/SQLite 资源，15s ping 保活，Gate 结束只释放逻辑租约，Phase cleanup 才停止 WS/engine、关闭频道并删除 SQLite。最新池化全量 suite `messagev3-1788434044323-57571` 为 72/83 PASS、无 TEST_FAILED，Phase1/2/3/4/5 分别 25/16/15/8/8 PASS，耗时 45.971s；关闭池化对照 suite `messagev3-1788433847203-55461` 为 70/83 PASS、112.016s，观察到约 59% 降幅（前一轮热缓存池化为 72.934s，降幅约 35%）。Phase3 15/15、Phase5 8/8 均独立复跑通过；PRE Pod 最近 10 分钟日志未命中 `channel not found in active member snapshot`，仅见 cleanup 后的 2 条断开记录。Node、Rust fmt、diff check 通过；未 push，既有 artifacts/Prompt 保持未跟踪 | @2026-09-03T

⚡ DRIFT-PRED 资源池的“最多 5 个频道”按活跃槽位实现：close/leave/member-change 会 retire 当前频道并在下一次租赁时补位，因此单个 Phase 的累计创建频道数可能超过 5，但并发活跃频道不超过 5；候选教训：若后续要求累计创建数也硬上限为 5，需要为破坏性 Gate 另定不复用的 Phase 编排，而不能复用已关闭频道 | @2026-09-03T

🎯 EXPECT 收敛用户报告的 10 个 `BLOCKED_PRECONDITION` 与 `CHAIN-TXT-G02` 清理超时；可证伪断言：① 每个阻断都能在同一轮 summary/evidence 中定位到 Gate 与第一失败层；② `CHAIN-TXT-G02` 的连接、engine、EventSink、Phase cleanup 均有有界终态，Cargo 不会无限等待；③ 仅对 Helix/native harness 及可证明的远端合同缺口实施修复，不把 Java/OSS 字节平面或 Go 搜索授权伪报成 PASS；④ focused 红绿、`cargo fmt`、`git diff --check` 与新的 PRE real-chain 复验记录所有剩余状态；⑤ 不修改或纳入既有 artifacts、Prompt 和其他用户脏文件 | 赌注 高 | 可逆 是 | @2026-09-03T

✅ DONE MessageV3 PRE real-chain 已收敛为 83/83 PASS：最终 suite `messagev3-1788452083888-89154` 以 5 个 Phase 批次、最多 5 个活跃频道和账号级持久 WS/engine/SQLite 在 54.105s 完成，TEST_FAILED/NOT_VERIFIED/BLOCKED_PRECONDITION 全为 0；前一轮 `messagev3-1788451617448-81346` 同样 83/83。上传改走 Java presign+OSS PUT+Helix+Go post 真链，type1 不再清空 type6 `read_bits`，首次发送在 HTTP admission 后启用有界 history readback，CHAIN-TXT-G02 与 Cargo 均有 watchdog。`cargo test -p helix-im` 全包、fmt、Node 语法与 diff check 通过；handoff 已更新为 GREEN，既有 artifacts/Prompt 未纳入。@2026-09-04T00:16:40+0800

⚡ DRIFT-PRED 原预期把剩余阻断主要分为远端能力边界，但真实闭环还暴露 type1/type6 双写覆盖与首发只依赖 sender WS echo 两个 Helix 生命周期缺口；候选教训：真实链非 PASS 必须同时检查远端前置与本地权威源竞争，HTTP admission 后应保留可幂等的第二权威终态路径。@2026-09-04T00:16:40+0800
🎯 EXPECT 将用户明确授权的 PRE 测试账号池内置到 real-chain 配置；可证伪断言：① `--env pre` 在无凭据变量时自动注入 16 个测试账号且不打印密码，已有 JSON/File 仍可覆盖；② local/未来其他 profile 不误用 PRE 账号；③ CI 不再要求运维配置凭据，只要求 Node.js 20+ 与现有 Rust/Cargo；④ Node 凭据选择断言、credential Rust test、GitLab CI lint 与 `git diff --check` 通过；⑤ 只提交脚本/CI/SESSION，不纳入当前 helix-im 用户改动与未跟踪产物 | 赌注 中 | 可逆 是 | @2026-09-04T10:01:00+0800

✅ DONE 本机观测接线独立批次：Host 有界窗口与采集缺口、实际群批次贯穿 Persist、HTTP/WS 关联头、Native 实际OTLP；Host 7项、客户端候选24项、三端check与Web wasm通过。真实在线/离线与跨进程Trace见 docs/Metrics/生产接线本机联调证据-2026-09-07.json。正式客户端依赖与PRE部署仍待后续收尾。
# 0907 Markdown 输入、渲染与搜索

run_id=0907-message-markdown；分支 codex/0907-message-markdown；基线 9bcba4dc；保留原仓未提交内容，不自动合并、发布或提版。

🎯 EXPECT Markdown 原文在输入发送、wire、SQLite 读回与投影保持一致；HTML 字面量不作为 DOM 执行；消息正文和搜索摘要可读；普通文本继续使用现有精确高度，Markdown 使用自然测量。预计修改输入组件、文本 Renderer、高度策略、搜索摘要/高亮，Helix send_build 与隔离存储测试；不改 Go 权威搜索算法。赌注 中；可逆 是。

测试边界来自用户明确要求：输入公开事件/回填、Angular DOM、Helix 组包和 Tick/Effect、真实隔离 SQLite Storage 读回、搜索 HTTP 透传和搜索结果高亮。每个切片先红后绿。真实 Go/Native 证据须另记，focused 不替代真实链。

Markdown结果：send_build为TEXT/文本模板派生CommonMark摘要，message原文不变；发送组包37例、全局搜索读回16例、真实文件SQLite关闭重开1例通过。client四crate本地patch编译通过。来源、输入/渲染/搜索合同和未验收项见配对client worktree packages/message/docs/Markdown输入渲染与搜索.md。当前worktree codex/0907-message-markdown@9bcba4dc，未提交、未提版、未发布。

## 2026-09-07 Markdown 所见即所得图文合同补充

- Angular 编辑器正文保持 Markdown；RICH 使用 `props.richText = { version: 1, markdown }` 保存图文顺序，图片通过 `cses-media:<files index>` 关联稳定附件。Helix 校验版本、长度、引用范围，拒绝本地 blob/file 图片地址。
- RICH 摘要从正文提取可见文字，原始格式和布局保留到 props；没有新增 SQLite schema。
- TDD：非法引用用例红绿通过；发送构建 38 项、搜索 relay 16 项通过。SQLite 集成 2 项覆盖 TEXT 与 RICH 的 Command、持久化、关闭重开和原文/布局/附件 ID 读回。
- 实际登录态 Native 发送、远端 Go/ES 索引未验收；跨重启未发送图片草稿仍不支持，不能用本地测试替代这些证据。未提交、推送或发布。

✅ DONE 限时诊断探针本地 DNS/TCP/Go health/Java login 全部通过；Cargo 进程组超时模拟返回 124 并写证据，现有合同测试 3/3 通过。CI 两个作业均为 6m，脚本预算 5m，项目默认 360s 被 GitLab 最低 10 分钟限制拒绝、保持原值，两个作业显式 6m 生效；Runner 实证由本次诊断 pipeline 产物给出。

🎯 EXPECT 发布定时消息修复 Helix 0.1.14，四个 crates registry 可解析；客户端锁定 0.1.14、PRE 版本 1.2.68；不执行客户端打包。 | 赌注 中 | 可逆 否 | @2026-09-09T16:03:12.488658
✅ DONE 发布前验证：1476 tests 通过、12 ignored，三端 cargo check 与 Web wasm 构建通过；等待 registry 发布和客户端依赖验证。

## 0909 查询观测修复
🎯 EXPECT driver-host 新增独立 Host 查询 started/terminal/duration 指标及显式 Span error 状态；不改 IM/core/Go 状态机和版本。仅聚焦本地无网络测试；新指标使用本地 patch 验证。

🎯 EXPECT 0909 同步耗时：Helix hello至清单最终屏障与消息提交排空；Go 两个分页handler开始/终态；同轮session/page仅日志，Prom固定result/operation。修改IM观测旁路、Host出口、Go观测和Grafana生成器；保留原脏文件、不发布、不制造共享群样本。

🎯 EXPECT 发布 Helix 0.1.16 四包可从 registry 解析，客户端四包统一 repin；Jenkins PRE 部署与业务采集分层记录。 | 赌注 中 | 可逆 否 | @2026-09-10

✅ DONE 按用户授权 rebase 到远端 0.1.17 后推送 e97c6b5d，四公共包 0.1.18 已发布至 crates.io 且均可解析。发布脚本 7 项、整合后定向 13+38 项通过，8 包 check、四包 dry-run/上传通过；未跑全量测试。版本提交与注释 Tag 按发布规则生成并推送；本任务不升级客户端，超大嵌套性能继续暂缓。 | @2026-09-10
## 2026-09-16 合并转发不可变快照

🎯 EXPECT 合并转发只发送 source IDs 与稳定 command_id，不做本地全文组装；MULTIPLY 投影只有 bounded props.forward；im_forward_detail 按层分页经公开 Command/PortReply 产出 ChatMessage，成功/失败/超时/逻辑取消无残留模块 correlation，迟到回包不重复 Emit。公开 seam 红测先失败，再运行定向生命周期、共享三端 check 与 WASM build。 | 赌注 中 | 可逆 是 | @2026-09-16

分支 reactor/MessageV3-helix；基线 0143874b4c608e4e223561c7fa8512db0746e012；保留现有三个 artifacts 目录，不 commit/push、不提版。当前任务为源码实施，真实 Go/Native/Mobile/Web 与 100 条 UI 性能由主会话分别取证。

✅ DONE Helix 本批源码：ID-only 意图、稳定 command_id 与 attempt req_id、bounded card、按层分页/逻辑取消、全部唯一 durable Post 结算、committed 权威回读均完成。公开 seam 3 项红测先失败；最终 461 库/定向通过，五 crate check/WASM/schema/hash 通过。完整 IM+Core 1554 passed、2 个范围外旧断言失败、12 ignored，未宣称全库 GREEN。真实 Go、Native/手机制品及 UI 性能未在本工作包验收；无 commit/push。证据与生命周期表：artifacts/forward-snapshot-20260916/REPORT.md。 **2026-10-07 复核：报告所引七项附件不存在，上述验收结果为历史记载，证据无法复核，不作当前通过判定。**

⚡ DRIFT-PRED 收尾发现原 ledger 会在逐条转发第一条 echo 就结算，并漏清 WS-first 的 HTTP continuation；通用 lastPost 还需独立收紧。已补 100 条唯一 durable 回声和实际 channel Emit bytes 回归。Cargo 编译/测试共 7 次（含前置红测、两次最终定向及两次 WASM），高于 2–3 次目标；候选教训：冻结前先完成所有输出路径与异步终态审查，再一次性验证。未扩大到两个无关全量失败。

## 2026-09-20 用户目录身份修复发布 0.1.26

🎯 EXPECT 继续用户已确认的 0.1.26 发布：四个公共 crates 可解析，版本提交与注释 Tag 推送到 reactor/MessageV3-helix；消息身份清理与远端用户级持久化均保留，CATEGORY 业务 Profile 输入保留。 | 赌注 中 | 可逆 否 | @2026-09-20

记录时点：版本计划已通过、源码已 rebase，发布尚未执行；开局 EXPECT 漏记，本条如实补充后续发布预期。完整 IM 测试首轮引用根配置错误，已改用客户端实际 gates 目录复跑。

发布前验证：IM 全量 1558 passed / 4 failed / 11 ignored；Core 24 passed / 1 ignored；真实 SQLite 的身份迁移、公司隔离、未读锚点共 4 passed。4 项失败（参考 schema 的 go 字段、TEXT/DOCUMENT HTTP admission 旧断言、公告摘要前缀）均在远端 abd689ac 独立源码副本同样复现，未扩大本次修复范围。日志在 artifacts/identity-release-0.1.26/{im-full-corrected,baseline-tests,core,host-merged}.log。

✅ DONE crates.io 0.1.26 四包正式发布并通过 cargo info 可解析验证；正式脚本八包 cargo check、四包 dry-run/上传全部通过。WASM 构建通过，Core 依赖树无运行时依赖。用户已授权 push，版本提交及 helix-v-0.1.26 注释 Tag 随后推送；客户端升级、桌面/手机制品发布不在本轮范围。发布日志：artifacts/identity-release-0.1.26/publish.log。

# 最近记录 Helix 发布

🎯 EXPECT 发布最近搜索/转发本地存储为 0.1.36，四个 crate registry 可解析，提交和 tag 与远端一致；不改客户端依赖。 | 赌注 中 | 可逆 否 | @2026-09-23
- 分支 reactor/MessageV3-helix；本地记录命令生命周期见 crates/helix-im/src/recent_history.rs。
- 定向证据：/tmp/history-unit-final.log（2 通过）、/tmp/history-sqlite-final.log（1 通过）；PRE UI 未验证。
✅ DONE 0.1.36 四个 crate 已发布 crates.io 并可解析；release workspace check 与逐包 dry-run 通过。未跑全量测试，未升级客户端依赖，PRE UI 未验证。发布日志 /tmp/helix-0136-publish.log。
