# 分类接龙 Helix 客户端接入合同

当前正式源码合同：`max_categories` / `maxCategories` 已改为必填（活动态），草稿上限必须为 1～分类数的整数，撤回卡省略该字段。历史五表 Native PASS 不覆盖此后协议与修复；客户端当前准入为 **BLOCKED_FOR_CLIENT_ACCEPTANCE**，原因与验证范围见 [2026-09-09 独立审查](reviews/2026-09-09-category-chain-review.md)。本地聚焦测试通过不代表当前客户端、真实断线恢复或 UI 验收通过。

旧独立URL阶段状态（历史）：**LIVE_NATIVE_SMOKE_PASS（限定单Native客户端+独占Go fixture）**；正式0.1.1回迁聚焦验收通过，真实证据对应0.1.9候选。UI DOM、Web/FFI、跨Pod/Pulsar、日常部署仍未验证。独立 CATEGORY_CHAIN / props.categoryChain 接缝；不改变 TEXT_CHAIN、CHAIN、props.chain、普通发送合同。设计真源为 cses-client/docs/superpowers/specs/2026-09-07-category-chain-extension-design.md；HTTP/WS目标真源为 cses-im-server/docs/category-chain-contract.md。

## 公开入口与目标路由

沿用 `helix_im::client_api::build_command(public_name, payload)` 与现有 Native/FFI/Web Command 入口；无新增业务ABI。顶层 camelCase 转 snake_case，nested audience/categories/selections 保持 camelCase。所有HTTP为POST，当前 URL 由既有 `api_base_url` 拼接 `post-chain/<action>?mode=CATEGORY`；下表列出 action 与读侧 view。请求体同时由 Helix 注入 `mode: "CATEGORY"`；`view` 由查询命令派生，不能由调用方指定。req_id仅用于本地waiter，不进入Go请求体。源码真源为 `crates/helix-im/src/category_chain/wire.rs::wire_body`；旧 `/category-chain/*` 路由只属于下方历史候选记录。

| public alias | canonical Command | action / view | 终态 |
| --- | --- | --- | --- |
| im_category_chain_capabilities | category_chain_capabilities | capabilities | im:read:result |
| im_category_chain_create_draft | category_chain_create_draft | create | operation |
| im_category_chain_update_draft | category_chain_update_draft | update-draft | operation |
| im_category_chain_publish | category_chain_publish | publish | operation + canonical Post |
| im_category_chain_get | category_chain_get | get / card | im:read:result |
| im_category_chain_get_mine | category_chain_get_mine | get / mine | im:read:result |
| im_category_chain_set_participation | category_chain_set_participation | append | operation + 原Post更新 |
| im_category_chain_entries | category_chain_entries | get / entries | im:read:result |
| im_category_chain_close | category_chain_close | close | operation + 原Post更新 |
| im_category_chain_retract | category_chain_retract | retract | operation + 原Post更新 |
| im_category_chain_reconcile | category_chain_reconcile | reconcile | im:read:result + operation |

读族必须带非空req_id；变更必须带clientMutationId；创建以外提供chainId（reconcile可未知）。CAS分别为expectedDefinitionRevision、expectedParticipationRevision、expectedRevision。actor/tenant/entryId/count/operationId等权威字段不得由UI注入。publish的temporaryId是Helix transport-only身份，按tenant/user/channel/clientMutationId确定性生成，先PersistAtomic再HTTP；UI传入该字段会被拒绝。重试复用原clientMutationId，reconcile不传commandKind。

## 事件与持久化生命周期

统一bus `im:__bus__`，信封严格 `{event,data}`。两个独立topic：

- `im:category_chain:operation`：认证HTTP操作结果、对账与错误；成功data保留Go MutationResult（含私有draft/myParticipation），增加channelId、viewerId、command、state以及可选req_id。channelId取捕获的Request.channel，viewerId取Request.viewer（运行态身份），在持久读回成功且read CommonRes生成之后才加入operation，不能由UI或Go回包覆盖。fail()的REJECTED/RECONCILING也携带同一viewerId。operationId保持服务端值。超时/传输不确定/Persist错误/过期authority为RECONCILING，明确业务失败为REJECTED；accepted不是成功。
- `im:category_chain:projection`：仅公开WS Card投影；不含draft、myParticipation、完整audience或私有操作状态。未知schema/字段和私有状态混入均拒绝。

查询沿现有 `im:read:result` 的req_id/body/error：body为 `{status:"SUCCESS",data}`。get保留chain/card/draft/post四字段，my-participation和entries直接返回各自对象，reconcile保留四态外层及nullable result/error。PENDING/NOT_FOUND不能授权更换幂等键。

业务成功必须经过HTTP/WS authority校验→durable head版本检查→PersistAtomic→持久读回→terminal Emit。SQLite用category_chain_head/category_chain_fact保存有界标量树，revision为精确十进制u64字符串；按viewer/channel/chain与查询scope隔离。Card、chain、Post和本人state分别持有head，避免Card-only WS擦掉HTTP字段；旧版本不得覆盖公共 Card/chain/Post；confirmed reconcile 的历史回执可持久化私有 receipt，并在所需私有快照读回后完成确认，普通旧 authority 则转 RECONCILING。无版本号的reconcile观察按state隔离，避免PENDING遮住后续终态。

完整CanonicalPost复用现有presence-aware message upsert和Post EventSink。HTTP及Card-only WS写入前读取message并比较props.categoryChain.revision；新增通用UpsertSpec.update_guard默认None，分类Post使用旧props作为CAS expected，比较失败使PersistAtomic整笔回滚。Host与Web均兑现该原语，Native/FFI继续复用Host；没有新Storage trait或业务ABI。最终再从message读回核对Card（HTTP要求相等，WS拒绝消息已存在的更高版本），防止只凭分类head忽略普通消息流已提交的新Post。旧WS拒绝后释放队列，当前Card仍可正常交付。同一锚点保持id/temporaryId，参与/编辑/取消不生成普通参与消息。WS action为`im:category_chain:projection`；eventSeq为字符串，与配对CanonicalPost共用channel_event。卡片本身绝不提交channel cursor或增加未读；游标由既有完整消息链提交。超时移除HTTP correlation，重复/迟到回包忽略；stop/account reset取消timer并清除队列和pending。

## 能力与真实验收前置

2026-09-07复核Go合同：已从IMPLEMENTATION_BLOCKED恢复为 **TARGET / IMPLEMENTATION_IN_PROGRESS**；独立模型/HTTP模块/手动迁移已落盘，应用层、事务存储和投递装配继续实施，新增HTTP模块已接根路由、未部署，`runtimeSupported=false`。本地命令注册或编译通过不代表服务上线。

只有服务端capabilities实际返回 `{categoryChain:{schemaVersion:1,supported:true}}` 才开放真实入口。404、失败、未知schema、字段缺失或false均禁用。Helix本地capabilities_json列出命令只说明客户端适配能力。

本轮不改版本、不提交、不push、不部署；主任务可在临时构建配置中使用本仓path/patch，不能将开发路径写入正式依赖。

## 分层证据

focused是ImModule公开seam回放，SQLite是HostStorage真实本地数据库；两者不能替代真Go/WS、Native运行态、双用户和UI/DOM证据。测试只创建本轮memory数据库，不清理共享业务库。

- TDD：首次公开Command路由红测见 `/tmp/helix-category-red.log`。后续真实复现旧HTTP覆盖新message的失败断言见 `/tmp/helix-category-message-race-red.log`；修复后该断言通过。Card-only WS同根因的红测见 `/tmp/helix-category-ws-race-red.log`；修复后同时验证拒绝旧Card与队列继续处理当前Card。
- focused：`cargo test --offline -j2 -p helix-im --lib --test category_chain_lifecycle --test send_test --test f1_send_http_reconcile_test`，370 lib + 21 category lifecycle + 9 reconcile + 14 send，共414 passed。最终日志 `/tmp/helix-category-final-im.log`。
- SQLite：`helix-driver-host` 的 category_chain_storage 4/4、upsert_guard 2/2、im_event_persist 13/13、storage_conformance 21/21、metrics_host_hops 3/3，共43/43 passed。包含生产ImModule → HostStorage → Persist/read-back → EventSink，guard miss可证伪整笔事务回滚。日志 `/tmp/helix-category-cas-host-focused.log`；Card-only WS修复后，category_chain_storage 4/4与upsert_guard 2/2再次通过，最终日志 `/tmp/helix-category-final-sqlite.log`。
- core完整测试：23 passed、1个既有doc example ignored；Web完整测试37/37 passed（含guard插入/匹配/拒绝/原子回滚）。日志 `/tmp/helix-category-cas-core-web.log`。
- Host完整测试保持PARTIAL：先被未改动 `tests/media_pipeline_e2e.rs:794` 缺 `ImConfig.client_platform` 挡住编译；聚焦集中的lib为88 passed/1 blocked，未改动的上传测试在 `src/dispatch_tests.rs:287` bind 127.0.0.1:0遭沙箱PermissionDenied。按仓库验证停止条件未扩修。日志 `/tmp/helix-category-cas-platform-tests.log`、`/tmp/helix-category-cas-host-focused.log`。
- 三端Rust：core/im/native/ffi/web五crate最终check通过；wasm32-unknown-unknown的Web最终build通过。日志 `/tmp/helix-category-final-check.log`、`/tmp/helix-category-final-wasm.log`。core依赖树没有tokio/sqlx/reqwest/web-sys/tauri，证据 `/tmp/helix-category-cas-core-tree.log`。cargo-public-api未安装，API工具检查按现有gate规则SKIP。
- 热路径审计：新增runtime/facts/wire均通过。Host write/upsert SQL与Web session store的脚本告警分别为5/1/4项，与HEAD基线逐项一致，guard路径未新增clone告警；基线对照日志 `/tmp/helix-category-hotpath-audit.log`。未将既有告警称为全仓审计Green。
- REAL_GO / HTTP+WS / Native运行态 / 双用户 / UI-DOM：NOT_RUN。服务端合同仍为 IMPLEMENTATION_IN_PROGRESS、未部署、runtimeSupported=false，不把fixture或编译通过计为端到端Green。

验证使用临时 `CARGO_HOME=/tmp/helix-category-cargo-home`，复用已有crate档案并将缺失源码展开到临时目录；没有变更正式依赖、Cargo.toml、Cargo.lock或版本。

## 交接范围与限制

主任务继续负责Angular/Tauri消费者，Go任务继续负责事务存储、路由与事件投递；本仓仅提供runtime适配及上述证据。外部客户端如直接构造 `UpsertSpec { ... }`，需补 `update_guard: None`，或使用现有 `UpsertSpec::new`；这是Rust源码字段增量，不是新业务ABI。仓内既有调用只补None，分类业务才启用guard。

本地完成不构成发布授权；保留当前工作树供主任务审阅，未创建commit、未push、未发布、未部署。

自动审批曾拒绝向主任务发送跨任务风险通知，理由为目标/内容授权不足。本地文档保留交接事实，未改用其他通道绕过发送限制。


## SESSION_REPORT/v1（初始本地切片历史快照，当前状态见文首及文末）

- status: PARTIAL（Helix本地切片完成；真实跨仓链NOT_RUN）
- branch/head: reactor/MessageV3-helix / caf20f3b，保持开局身份。
- delivered: 11个Command、独立HTTP/公开WS合同、normalized持久化及read-back终态、publish稳定身份、私有scope、超时/取消、HTTP与WS旧版本保护、Host/Web原子更新guard。
- validation: focused 414 passed；SQLite相关43 passed（最终分类链与guard 6项复验通过）；core 23 passed/1 ignored；Web 37 passed；五crate check与wasm build PASS；git diff --check PASS。
- blockers: Host全量既有media_pipeline_e2e缺字段；既有上传lib测试受沙箱端口限制。真Go/WS、Native运行态、双用户和UI/DOM尚无证据，服务端未部署且runtimeSupported=false。
- compatibility: UpsertSpec新增可选字段，外部直接字面量需补None；本轮只读搜索cses-client/src-tauri未发现此类直接构造。
- publication: 未commit/push/publish/deploy，Cargo.toml/Cargo.lock/版本未改变。交付为本仓未提交工作树。


## 2026-09-07 有界修正：私有operation请求上下文

首次create尚无请求chainId时，成功operation仍带请求channelId、运行态viewerId及Go返回的chainId。完整Go MutationResult保持扁平，含outcome/commandKind/clientMutationId/operationId/chainId/revision/eventId/eventSeq/draft/card/post/myParticipation，state仍为CONFIRMED。Helix没有增加scope或mutationResult嵌套；客户端Host adapter可按其合同归一化，但必须保留完整receipt，拒绝只有CONFIRMED的裸状态。

以下是**真实ImModule Emit产物**，HTTP输入使用目标Go fixture、存储通过公开Effect/PortReply回灌；证据等级为FOCUSED，不冒充真实Go或TS解码通过：

| 场景 | 完整实际输出JSON |
| --- | --- |
| 首次create，入参没有chainId | [create-draft.json](fixtures/category-chain-operation/create-draft.json) |
| publish，Helix持久身份先于HTTP | [publish.json](fixtures/category-chain-operation/publish.json) |
| reconcile确认成功，同时保持read CommonRes原样 | [reconcile-confirmed.json](fixtures/category-chain-operation/reconcile-confirmed.json) |
| 业务拒绝 | [rejected.json](fixtures/category-chain-operation/rejected.json) |
| HTTP超时、不确定终态 | [reconciling.json](fixtures/category-chain-operation/reconciling.json) |

生成命令、源码/fixture SHA-256及证据限制见 [provenance.json](fixtures/category-chain-operation/provenance.json)。普通测试只比较fixture；仅显式 `HELIX_UPDATE_CATEGORY_OPERATION_FIXTURES=1` 可覆盖。主任务可直接读取单个JSON作为其Host adapter的输入；该文件不是归一化后的private wrapper。

本次红测准确失败于首次create operation的channelId为Null；日志 `/tmp/helix-category-context-red.log`。修正后22/22聚焦测试通过，包含create/publish/reconcile的完整receipt比较、REJECTED/RECONCILING身份、恶意reply.viewerId拒绝、WS原形与read CommonRes原形比较；生成日志 `/tmp/helix-category-context-generate.log`、普通比较日志 `/tmp/helix-category-context-green.log`。cargo check -p helix-im通过（`/tmp/helix-category-context-check.log`），热路径审计及git diff --check通过。本次没有重跑上一节无关全量；旧全量证据仍属于上一轮实现快照。

## 0.1.1源码与客户端0.1.9的最小临时path验证

已只读核对：本仓四个公共crate为0.1.1，内部path依赖精确要求=0.1.1；客户端src-tauri/Cargo.toml四个直接依赖精确要求=0.1.9。单加 `[patch.crates-io]` 或CLI patch不能让0.1.1满足=0.1.9；Cargo仍要求被选版本满足依赖约束，见 [Cargo官方说明](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#testing-a-bugfix)。不要伪改Helix package.version成0.1.9。

最小改动是**在/tmp内创建客户端当前工作树的隔离副本**（保留本轮未提交修改及src-tauri相对路径依赖；排除.git、node_modules和target），仅替换该副本src-tauri/Cargo.toml的四个直接依赖为下面的path声明。占位根目录替换为本仓实际绝对路径，只出现在临时副本：

```toml
helix-core = { package = "cses-helix-core", path = "<HELIX_ROOT>/crates/helix-core" }
helix-im = { path = "<HELIX_ROOT>/crates/helix-im" }
helix-driver-host = { path = "<HELIX_ROOT>/crates/helix-driver-host" }
helix-driver-native = { path = "<HELIX_ROOT>/crates/helix-driver-native" }
```

四个声明不保留version=0.1.9；既有features/default-features如有则保留。无需改本仓任何Cargo文件，四个crate的内部0.1.1 path图自然统一。在临时副本src-tauri目录运行 `cargo metadata --format-version 1`、`cargo tree` 和所需 `cargo check`，只允许它更新临时Cargo.lock，首次解析不加 `--locked`。核对四个crate的manifest_path都指向本仓、均为0.1.1且没有混入0.1.9的Helix core类型。正式客户端Cargo.toml/Cargo.lock保持逐字不变，也不要将临时lock回拷。

这是**验证候选源码接入的方法**，不等于本仓0.1.1与已发布0.1.9全部API兼容；实际客户端编译仍由主任务执行并记录，遇到新旧API差异要按实际报错处理，不能以Cargo解析成功代替客户端PASS。本轮未修改或构建客户端，未做版本伪装、提交或发布。


## 客户端临时path复核：观测API基线不兼容（2026-09-07）

结论：四个观测API的缺失是**本地0.1.1与已发布0.1.9的既有源码差异**，不是新增分类接龙改动删除或隐藏API。四个path能正确解析到本仓，只证明依赖替换成功，当前替换仍不能作为客户端编译通过证据。

证据链：

1. `/tmp/category-chain-local-helix-check.log:1059` 的E0432报告前三个导入缺失；`:1068` 的E0425报告record_seq_observation缺失，客户端调用分别在 `src-tauri/src/features/helix_im/commands.rs:3` 和 `lifecycle.rs:198`。
2. 本仓开局HEAD为 `caf20f3b`（chore(release): 发布 Helix 0.1.1）。`src/lib.rs`、`src/metrics/mod.rs`、`src/metrics/business.rs` 三个Host文件与该HEAD逐字相同，四个符号在HEAD和当前工作树都不存在；本轮Host生产diff只涉及storage/write.rs、upsert_sql.rs。
3. 本机Cargo缓存 `helix-driver-host-0.1.9.crate` 的SHA-256为 `9037aec1143d86b4b989a554304c6002fad4ba421d1e60df3033bfa8f9b7c439`，与正式客户端Cargo.lock的registry包checksum一致。上述三个已解包源码文件也与该归档逐字一致，排除了仅凭可能修改过的缓存目录作判断。

| API | 已发布0.1.9实现位置（包内） | 本仓0.1.1 HEAD/工作树 |
| --- | --- | --- |
| ClientTerminalObservation | src/metrics/business.rs:9 | 均无 |
| record_message_client_terminal | src/metrics/business.rs:19 | 均无 |
| record_message_e2e_view_updated | src/metrics/business.rs:37 | 均无 |
| record_seq_observation | src/metrics/business.rs:55 | 均无 |

四个符号在0.1.9的src/metrics/mod.rs:17–19及src/lib.rs:49–51公开re-export，没有本轮条件编译变化。发布包 `.cargo_vcs_info.json` 记录SHA `537170620ae237e935277017bfc36ee4824f4605`，同时为 `dirty: true`；不能声称只checkout该SHA就能重建精确发布源码，发布归档/匹配checksum才是本次已核实依据。

安全接入建议：保留客户端正式0.1.9依赖和锁文件。后续由基线owner提供与发布归档一致的完整Helix源码集合，在独立验证副本中迁入本次分类接龙切片（包括core/Host/Web的UpsertGuard合同），对齐后再跑四crate依赖图和客户端编译。不要只混搭0.1.9 Host与0.1.1 core/im/native，不补空壳/no-op观测API，不通过伪改版本号或删除客户端观测调用掩盖差异。本轮仅确认限制，不执行基线迁移或扩大实现。

主任务回报：五份Emit fixture已原样复制，全部通过生产TS Host adapter解码，覆盖首次create/publish/reconcile-confirmed/rejected/reconciling。此为**主任务提供的客户端解码证据**，本轮未重新运行TS；不等同于客户端原生构建、真实Go或UI/DOM通过。日志内两项临时副本资源缺失由主任务补齐复核，独立于上述API缺失，不据此修改本仓。本轮只读核查并更新本合同，未改生产代码/客户端/版本，未提交或发布，未运行Cargo构建。


## 当前交接入口：隔离0.1.9 Native验证候选（2026-09-07）

**NATIVE_CANDIDATE_READY / WEB_BASELINE_MISSING**。候选目录 `/tmp/helix-category-0.1.9-candidate-kortuqqf`，README及NOT_FOR_PUBLICATION.md明确不可发布，四个package均publish=false。它由客户端Cargo.lock校验和匹配的四个0.1.9归档恢复，保留发布包观测实现；没有将正式工作树切换基线，也没有混用0.1.1生产crate。

| 客户端依赖 | 候选path |
| --- | --- |
| helix-core（package=cses-helix-core） | `/tmp/helix-category-0.1.9-candidate-kortuqqf/crates/helix-core` |
| helix-im | `/tmp/helix-category-0.1.9-candidate-kortuqqf/crates/helix-im` |
| helix-driver-host | `/tmp/helix-category-0.1.9-candidate-kortuqqf/crates/helix-driver-host` |
| helix-driver-native | `/tmp/helix-category-0.1.9-candidate-kortuqqf/crates/helix-driver-native` |

候选包含分类Command/HTTP/WS/持久读回/私有operation上下文，以及core+Host的UpsertGuard。以0.1.1 HEAD为三方合并基底，只迁入分类delta；两处冲突按已发布0.1.9解决：保留其完整Post写入/游标逻辑，只补UpsertSpec默认None；保留原31个WS handler并新增分类handler，分母为32。新增字面量补None不改变非分类调用的存储行为。

**基线行为差异**：0.1.9未改动的projection_message_effects.rs:69对本人消息发 `im:post:sent`。候选publish测试按此正确合同调整，不把生产事件改回0.1.1的received；原始失败证据category-precompat.log保留。五份private operation fixture逐字与本仓/主任务已解码版本一致，没有重写为新形状。

验证结果：四crate `cargo check --workspace`通过；分类生命周期22/22、真实SQLite分类链4/4、原子回滚2/2、原发布观测API实际调用1/1通过。最后一项捕获并核对真实counter/histogram输出，不是只验证符号可导入。Host src/lib.rs、metrics/mod.rs、metrics/business.rs与发布归档逐字一致；依赖图恰好只有四个本地0.1.9 Helix包，core无tokio/sqlx/reqwest/web-sys/tauri依赖。新category runtime/facts/wire热路径审计通过。详细机器证据见 [category-chain-0.1.9-candidate-evidence.json](category-chain-0.1.9-candidate-evidence.json)，候选内的assembly.json、archive-delta.patch、validation.json保留归档checksum、逐文件SHA与迁移diff。

主任务在**客户端隔离副本**中保留四项registry依赖=0.1.9，然后从该副本src-tauri目录执行：

```sh
cargo --config /tmp/helix-category-0.1.9-candidate-kortuqqf/client-patch.toml check
```

首次运行允许更新临时Cargo.lock，不使用--locked，不能在正式客户端目录执行这条验证命令。若现有副本仍有上一轮0.1.1的path声明，须先仅在副本恢复原四项=0.1.9声明，或把四个path全部替换成上表候选路径；registry patch不会替换已有path依赖。无需调整任何正式版本号或正式锁文件。客户端最终编译、资源补齐和UI运行仍由主任务负责，本轮未修改cses-client。

重跑候选自身验证的准确命令：

```sh
CARGO_HOME=/tmp/helix-category-cargo-home cargo check --offline -j2 --manifest-path /tmp/helix-category-0.1.9-candidate-kortuqqf/Cargo.toml --workspace
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline -j2 --manifest-path /tmp/helix-category-0.1.9-candidate-kortuqqf/Cargo.toml -p helix-im --test category_chain_lifecycle
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline -j2 --manifest-path /tmp/helix-category-0.1.9-candidate-kortuqqf/Cargo.toml -p helix-driver-host --test category_chain_storage --test upsert_guard --test category_019_api_compat
```

**精确缺失**：本机缓存没有Web/FFI归档，四个发布包均只携带各自src与README；scripts/publish_crates.py:43–74将Web/FFI列为不上传的消费者。四包 `.cargo_vcs_info.json` 都标dirty=true，不能凭其Git SHA猜造完整Web/FFI同版源码。已将本次Web UpsertGuard合同保留为候选内 `pending-web-upsert-guard.patch`，明确UNAPPLIED；候选没有Web/FFI源crate或wasm验收结果。需要对应平台真实源码后才能继续这些边界验证；本轮交付足以供Native客户端path/patch编译验证，不能称为三端发布候选。未提交、发布、修改正式Cargo文件或还原其他任务改动。


## Native真实smoke准备（BUILD_READY_NOT_RUN）

主任务已回报：客户端隔离副本使用0.1.9候选完成cargo check --offline（4m44s），metadata四包单一0.1.9，正式Cargo/lock hash未变；这属于主任务客户端编译证据，不由本轮重复执行。

本轮只在 `/tmp/helix-category-0.1.9-candidate-kortuqqf/harness/category-native-smoke` 新增harness，四包生产源码/manifest逐文件SHA与此前validation.json完全一致。复用shared RealHelixSession的参数化装配方法及真实 `run_engine_loop_with_transports_and_trace`，所有业务走 `build_command→Tick::Command→生产HTTP/WS/Storage→NativeEventSink`，不存在直接curl或模拟PortReply成功。生产trace hooks记录关联HTTP回包/WS入站/存储dispatch，NativeStorage在终态后独立读回scoped head及原anchor/card。

离线build通过，3/3离线测试通过（端点限制、证据脱敏、缺失/改变anchor拒绝）；已运行--help，未执行--run、未连接HTTP/WS/PostgreSQL、没有live runs目录。机器证据见 [category-native-smoke-readiness.json](category-native-smoke-readiness.json)。

完整运行说明与最小Go fixture/保活/清理约定：[候选harness README](/tmp/helix-category-0.1.9-candidate-kortuqqf/harness/category-native-smoke/README.md)；输入模板：[config.template.json](/tmp/helix-category-0.1.9-candidate-kortuqqf/harness/category-native-smoke/config.template.json)。模板故意没有可用端点或身份，须由Go/主任务填入实际资料。专库声明固定127.0.0.1:55441/cses_im_test，runner不读取、验证或修改该库，端点绑定由Go owner保证。

收到真实端点及隔离身份、确认服务将保持开启后，在主任务协调下执行：

```sh
/tmp/helix-category-0.1.9-candidate-kortuqqf/target/debug/category-native-smoke --check-config /absolute/path/to/private-config.json
/tmp/helix-category-0.1.9-candidate-kortuqqf/target/debug/category-native-smoke --run /absolute/path/to/private-config.json
```

配置必须chmod 600，显式提供HTTP/WS完整地址、端口、user/company/channel、真实认证头、唯一run_id、timeout_seconds及未来deadline_ms。仅允许127.0.0.1，拒绝URL内凭据/query；不自动发现账号或推断端点。Go需提供独占隔离channel、成员/权限、完整WS hello与生产启动/sync依赖、11条category路由和真实outbox投递，并保持服务直到Native graceful cleanup结束（预留最多30分钟）。

流程覆盖11路由：能力→创建/修改草稿→get→发布→本人状态→同人两类参与→两类entries→对账→整集合取消→close→retract；发布后的每个写终态需固定anchor、持久读回及相同revision/eventSeq的WS入站+EventSink。输出仅在候选runs/<run_id>新目录：脱敏evidence.json、私有资源清单与本轮SQLite；不清理其它任务数据或远端schema。Go owner随后按run_id及私有资源清单处理本轮数据。真实HTTP/WS/Native运行、Go DB/outbox、故障注入恢复和UI仍为NOT_RUN，不能把本轮编译当成live PASS。未向busy Go任务发送消息，未改客户端或正式版本/锁文件，未提交发布。


## Native 首次真实阻断后的候选修复

首次真实运行分类参与 HTTP SUCCESS 后因 Go 空资料快照与 Helix displayName 下界不一致进入 RECONCILING。候选完成公开 seam 红绿、25 项生命周期/4 项 runner 离线回归及重建后，新独占run已通过限定Native实跑验收，当前状态 LIVE_NATIVE_SMOKE_PASS。准确生产 delta、源码依据、旧失败与新 SHA 见 [修复报告](category-native-profile-compat-report.md) 及 [机器证据](category-native-profile-compat-readiness.json)。该诊断阶段正式生产源未改；后续正式0.1.1窄回迁见下一节，版本/锁文件始终不改。新独占fixture已由主任务/Go owner协调执行并清理。


## 空资料兼容已回迁正式 0.1.1 分类工作树

主任务明确授权本项窄回迁后，确认正式 `wire.rs` 与候选修复前逐字一致，只同步 `validate_entry_preview` 的 displayName 下界 1→0及说明注释。该字段仍必填且为有界字符串，身份/内容/canonical revision/card-Post一致性与Persist屏障不变；空资料原样透传，展示兜底仍由shell负责。

公开 seam 新增三个回归：同人双类空资料经持久读回后唯一CONFIRMED、分页及共享WS保持空值、八类非法/伪造输入继续fail-closed。原0.1.1测试完整前缀逐字保留，尤其 publish 的 `im:post:received` 断言未改；未迁入0.1.9普通消息或观测代码。正式树先红（2 fail/1 pass），修复后25/25通过，`cargo check --offline --locked -j2 -p helix-im`与热路径审计通过。

[窄补丁](category-profile-compat-0.1.1.patch) 相对于本轮开始时已有分类工作树生成，**不是可直接覆盖干净HEAD的整功能补丁**；[机器证据](category-profile-compat-0.1.1-evidence.json) 记录前后hash、命令、日志及边界。所有其他预记录Rust源/manifest/lock哈希不变，正式版本/锁文件未改。正在由Go owner重跑的/tmp候选源码/manifest/lock/binary哈希均未变。无提交、发布、Go消息或新live运行。


## 最终限定 Native 实跑验收

当前验收：**LIVE_NATIVE_SMOKE_PASS**，限定本地单 Native 客户端 + 独占 Go fixture。修复后 run `category-native-1788793161209345000` 已由 Go 独立 Quality accepted；`TestCategoryChainNativeSmoke` PASS（包 7.214s）。Native PASS、result/cleanup 均 null、550 hops、overflow=false、11 路由无缺失；revision/eventSeq 3–7 逐次 WS ingress、EventSink 与 SQLite 对应。Go 终态为 1 Post、7 mutation、5 事件/outbox 全部 delivered、2 CANCELLED tombstone、0 活跃、RETRACTED revision7，安全 Card/anchor 一致。Go 独立只读清理后验确认专用分类 schema 0、六核心表均 0 行；本任务未连接或查询 Go 数据库。

实跑 binary SHA256：`b651e69f204dd523accfce72a2c89255ab6894e945ade090047224a1e9f8f766`，与正式回迁阶段校验值一致；空 User/Profile fixture 未填充姓名。另有非分类后台 HTTP NOT_SUCCESS，Go Store 全 scope 覆盖率 95.511% 仍 partial，本 smoke 不改变其门禁状态。正式 0.1.1 窄回迁已有 25 项分类回归与 check 证据；真实运行证据属于该 0.1.9 候选 binary。

证据：[Native脱敏记录](/tmp/helix-category-0.1.9-candidate-kortuqqf/runs/category-native-1788793161209345000/evidence.json)、[Go报告](/Users/cses-11/Desktop/cses-im-server/docs/reports/0907-分类接龙Native跨仓Smoke.md)、[Helix机器汇总](category-native-profile-compat-readiness.json)。旧失败和修复红绿记录保留如下。

本次仅更新Helix文档/证据状态，未重新跑测试、修改代码/版本/锁文件/候选binary，也未触发其它任务。
