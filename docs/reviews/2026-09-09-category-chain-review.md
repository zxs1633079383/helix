# SESSION_REPORT/v1 — 分类接龙 Rust/Helix 独立审查

- 日期：2026-09-09；run_id=`category-review-20260909`。
- 结论：**BLOCKED_FOR_CLIENT_ACCEPTANCE**。当前客户端实际 0.1.9 可编译，但存在已复现的旧消息覆盖新版、权限缓存过期、已确认操作无法完成对账问题；专项通过不足以批准客户端接入。
- Owner/范围：Helix/Rust review；客户端与 Go 由 review 主任务协调。遵循当前必填 `maxCategories`、完整 `selections` 原子替换合同，不按旧 PRD 改产品语义。
- 本轮只追加本文与 SESSION，测试/probe/日志写 `/tmp/category-review-20260909`；未改业务源码或仓内测试，未提交、发布、改版本、操作真实账号/群/远端 DB。
- corr/req_id：本地公开 seam 使用合成 mutation/req_id；channel_id 为测试生成 ID；expected_event=`im:category_chain:{operation,projection}` 与普通 Post event。无真实账号运行证据。

## 1. Severity 排序 findings

### F1 — P1：客户端实际 0.1.9 的普通 Post 写入能覆盖分类 HTTP 已提交的更高 revision

**已用真实 SQLite + 公开 ImModule seam 复现：HTTP revision=10 完成后，收到 event_seq=1、card revision=9 的 `post`，持久 message 变成 9。** 频道事件序号尚未消费，不代表这份业务快照比 HTTP 更新；两种时序必须分别约束。

- 实际副本 [channel/gate/storage.rs:40](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/channel/gate/storage.rs:40) 的 Post/sync upsert 没有 category revision 守卫；[message_v3_post.rs:61](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/port_reply/message_v3_post.rs:61) 的普通 edit 直接执行 content patch + cursor 写集；[post_update.rs:159](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/ws/handlers/post_update.rs:159) 对 CATEGORY 也走此入口。
- 专用分类 runtime 的 props CAS 只约束它自己的写入；普通 Post/sync writer 没有反向保护。可出现取消/关闭/撤回的新卡片被旧事件恢复、UI 与私有状态不一致。
- 正式仓已有 [post.rs:18](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/post.rs:18) 的固定宽度版本守卫、[post_events.rs:15](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/post_events.rs:15) 的提交后读回与 Host/Web 实现；实际副本完全缺少这两份文件及 `version_column/category_revision` 接线。
- 证据：[loaded-storage-reconcile-probes.log](/tmp/category-review-20260909/loaded-storage-reconcile-probes.log) 中 `category_review_http_then_older_post_keeps_durable_revision`：实际 `9`，期望 `10`。该测试复制正式现有回归，**只在临时副本的 Card/Mine 测试输入补必填 maxCategories=2**，没有修改生产依赖。测试在首个 `post` 已失败，不能声称本轮已跑到后续 `post_update/posts_update` 分支。
- 准入修复：在实际加载的四包同版依赖图迁入分类单调写入和持久读回；分别重跑 HTTP↔post/post_update/posts_update/sync 交错，断言 message、事件和恢复后的投影同版。禁止用正式仓测试替代实际副本。

### F2 — P1：实际 0.1.9 将时间/访问权限绑定到 participationRevision，重新读取仍返回旧权限

- [runtime.rs:726](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/runtime.rs:726) 把完整 Mine（含 canParticipate/canEdit/canCancel/denialReason）存为 participationRevision 文档；[runtime.rs:365](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/runtime.rs:365) 同版本标记不写，最后从旧文档读回。
- 当报名条目没变、接龙关闭/到期/访问权限变更时，服务端新 Mine 的 revision 可以仍是 `1`。本轮复制正式现有回归，输入三项权限 false 与 `CATEGORY_CHAIN_CLOSED`，实际返回三项 true 与 null。Go 仍应拦截写入，但客户端已不能依据重新读取结果正确锁定动作。
- 正式仓 [runtime.rs:884](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/runtime.rs:884) 已将条目事实与本次响应权限拆开，正式对应回归通过；实际副本未同步。
- 证据：[loaded-regression-probes.log](/tmp/category-review-20260909/loaded-regression-probes.log)，`category_review_same_participation_revision_refreshes_permissions` 失败。
- 准入修复：同步正式修复，覆盖同 participationRevision 的关闭、到期、撤权及重新打开，保留当前权威权限、不本地推导。

### F3 — P1：新投影到达后，历史 mutation 的 CONFIRMED reconcile 被降成 RECONCILING，无法收敛

- 两套 runtime 都把 reconcile 的历史 `result` 拆成当前 projection 待写文档：[正式 runtime.rs:728](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/runtime.rs:728)。只要本地公共 head 更高，[正式 runtime.rs:364](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/runtime.rs:364) / [实际 runtime.rs:365](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/runtime.rs:365) 就以 `STALE_AUTHORITY_RECONCILE` 拒绝整项工作；错误最终回为 RECONCILING。
- 具体触发：操作 A 已在服务端成功但响应丢失；随后收到 B 的 revision=10；以 A 原 mutation key 核对，服务端明确返回 CONFIRMED + A 的 revision=9 receipt。本轮实际副本 probe 得到 `im:read:result.error=STALE_AUTHORITY_RECONCILE` 和 operation RECONCILING。重复同 key 查询仍命中更高 head，冻结的已提交意图没有终态出口。
- 这是 operation receipt 与当前公共快照混用，不能靠生成新 key 解决。应持久确认 A 的操作终态，同时抑制其陈旧 Card/Post 覆盖当前投影；具体响应消费合同由主任务与 Go/客户端共同冻结。
- 证据：[loaded-storage-reconcile-probes.log](/tmp/category-review-20260909/loaded-storage-reconcile-probes.log) 的 `category_review_confirmed_historical_operation_remains_terminal`。此项为**新增临时审查 probe**，复用正式公开 seam helper；实际 0.1.9 已动态复现，正式同逻辑为静态确认，未冒充正式动态回归。

### F4 — P2：实际 0.1.9 fact 查询先限 4096 行再筛 revision，旧路径能挤掉新快照

- [实际 facts.rs:119](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/facts.rs:119) 各版共用 snapshot_key/path，旧路径保留；[facts.rs:153](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/facts.rs:153) 按 scope/path 先 LIMIT 4096，[facts.rs:211](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/facts.rs:211) 解码时才过滤 revision。
- 两份各 3000 个不同路径、各自满足单文档上限的合法快照，第二份可缺失尾部当前事实。本轮回归实际进入 `canonical Post identity/type` 错误并发 RECONCILING，不能交付完整 terminal。
- 正式 [facts.rs:157](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/facts.rs:157) 已按 version_key 先过滤再 limit，并有 [schema.rs:561](/Users/cses-11/Desktop/helix/crates/helix-im/src/schema.rs:561) 迁移/索引；实际副本没有此列，迁移 probe 也确认为缺失。
- 证据：`category_review_old_fact_paths_cannot_truncate_current_snapshot`，见 [loaded-regression-probes.log](/tmp/category-review-20260909/loaded-regression-probes.log)。正式现有回归本轮通过。

### F5 — P2：严格合同与当前可运行验证基线未同步，不能复现“全分类已通过”

- 两套 [正式 wire.rs:2175](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/wire.rs:2175) / [实际 wire.rs:2186](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/src/category_chain/wire.rs:2186) 库测试仍给缺少 maxCategories 的 Card；本轮各失败一项。
- [正式 category_chain_storage.rs:325](/Users/cses-11/Desktop/helix/crates/helix-driver-host/tests/category_chain_storage.rs:325) 的 Card 及 [Mine:377](/Users/cses-11/Desktop/helix/crates/helix-driver-host/tests/category_chain_storage.rs:377) 缺字段，使两项 SQLite 生命周期/竞态测试在到达预期存储屏障前失败。
- 实际副本 lifecycle 的 [fixture reader:1050](/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix/crates/helix-im/tests/category_chain_lifecycle.rs:1050) 所需 `../../docs/fixtures/category-chain-operation` 不存在；实际直接运行结果为 31 passed / 5 failed，无法复现验证文档中的 36/36。没有生成/拷贝 fixture 到实际副本来改变这个结论。
- 准入修复：将所需测试资产、当前合同样例和明确的执行根目录纳入候选制品；保留首次失败与真实复跑证据。不能把 `/tmp` runner 或另一份 0.1.9 副本的结果当作当前客户端目录通过。

## 2. 版本与实际加载来源

| 项目 | 本轮实际事实 |
| --- | --- |
| 正式 Helix | `/Users/cses-11/Desktop/helix`，分支 `helix-message-test`，HEAD `caf20f3bac9cf72bf2cc7509d2db8571c5055e4c`，workspace 0.1.1，分类及后续修复是 dirty/staged 工作区，不是 HEAD 发布内容 |
| 客户端依赖声明 | [src-tauri/Cargo.toml:87](/Users/cses-11/Desktop/cses-client/src-tauri/Cargo.toml:87)：四包固定 `=0.1.9` |
| 客户端 patch | [.cargo/config.toml:3](/Users/cses-11/Desktop/cses-client/.cargo/config.toml:3)：四包指向 `.cargo/category-chain-helix/crates/*` |
| 实际解析 | 在客户端 src-tauri 运行 `CARGO_HOME=/tmp/helix-category-cargo-home cargo metadata --offline --locked --format-version 1` 成功；四包均为上述绝对 path、version=0.1.9、source=null，无另一份 Helix 版本混入 |
| 可运行性 | 正式/实际 IM lib 和专项 test binaries 均成功编译后运行；有测试失败。不是客户端 Tauri binary 的本轮构建或运行证明 |
| 逐文件保护 | 开局记录正式369份、实际332份 Rust production source SHA256；收尾全部一致，未动任何一方业务源码 |

证据：[client-metadata.json](/tmp/category-review-20260909/client-metadata.json)、[source-hashes-before.json](/tmp/category-review-20260909/source-hashes-before.json)、[source-hash-check.json](/tmp/category-review-20260909/source-hash-check.json)。客户端的 0.1.9 保留较新普通 Post/观测基线，正式 0.1.1 保留较新分类修复；**不能按版本数字大小推导功能包含关系，也不能整仓互盖**。

## 3. 命令与数据合同

公开别名前缀 `im_category_chain_` → canonical `category_chain_`；[client_api/command.rs:16](/Users/cses-11/Desktop/helix/crates/helix-im/src/client_api/command.rs:16)、[outbound/posts/category_chain.rs:40](/Users/cses-11/Desktop/helix/crates/helix-im/src/outbound/posts/category_chain.rs:40)、[module/trait_impl.rs:138](/Users/cses-11/Desktop/helix/crates/helix-im/src/module/trait_impl.rs:138) 真实注册/派发。未知分类命令 fail-closed，不进入 TEXT。

所有路由都是 **POST**（读分类不改变 HTTP verb），共同带 query/body `mode=CATEGORY`。来源：[wire.rs:69](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/wire.rs:69)、[registry.rs:162](/Users/cses-11/Desktop/helix/crates/helix-im/src/outbound/registry.rs:162)。

| alias/canonical 后缀 | HTTP path（均前缀 post-chain/，后缀 ?mode=CATEGORY） | 关键输入 / 结果 |
| --- | --- | --- |
| capabilities | capabilities | 读；req_id → supported/schemaVersion |
| create_draft | create | title/description/example/deadline/audience/categories/max_categories/client_mutation_id → draft receipt |
| update_draft | update-draft | 同上 + chain_id/expected_definition_revision |
| publish | publish | chain_id/expected_definition_revision/client_mutation_id/user_snapshot；Helix 派生 temporaryId |
| get | get | channel_id/chain_id，view=card，完整 get result |
| get_mine | get | channel_id/chain_id，view=mine，私有完整 Mine |
| set_participation | append | expected_participation_revision + 完整 selections + client_mutation_id/user_snapshot |
| entries | get | view=entries，category_id + 可选 opaque cursor/limit（≤100） |
| close | close | expected_revision + client_mutation_id |
| retract | retract | expected_revision + client_mutation_id |
| reconcile | reconcile | 原 client_mutation_id，chain_id 可选，req_id 必需 |

- maxCategories：两套当前生产 wire 都必填，无可选/默认兼容；草稿整数 1～分类数，分类数 2～10；活跃 Card/Chain/Mine 必须有值，RETRACTED Card 省略。公共入口只转换顶层 camel→snake，内层 selections/userSnapshot 保持 camelCase。发布接口不接收 maxCategories，发布后上限由 Go 固定。
- selections：一次提交目标全集，空数组取消全部；Helix 校验形状/重复分类/有界文本，不逐项 append 或替用户补计数。旧选择的释放、权限与 CAS 由 Go 事务决定。首次空选择和未改动不提交属于客户端当前交互合同。
- revision：公共 Card/Chain 的业务版本；definitionRevision：草稿 CAS；participationRevision：当前用户报名 CAS；categoryRevision：分页对应分类版本。保持 canonical 十进制字符串/u64 比较，不经 JS number；entries cursor 是 opaque pagination token，不能当作 channel cursor。
- eventSeq/channel cursor：通道重放连续性；不能用于比较 Card 业务新旧。专用分类 WS 在 [runtime.rs:935](/Users/cses-11/Desktop/helix/crates/helix-im/src/category_chain/runtime.rs:935) 只做缺口 admission，不提交 cursor。canonical Post 才拥有 message/cursor 原子提交。

## 4. admission、操作终态与跨层收敛

Tauri [engine.rs:61](/Users/cses-11/Desktop/cses-client/src-tauri/src/features/helix_im/engine.rs:61) 队列接收仅是 admission。六个 mutation 由调用方持有稳定 key；Helix 不替换 key。publish 将 tenant+viewer+channel+key 确定性散列为 temporaryId 并先持久化，客户端不得自带 temporaryId。

HTTP success 必须为合法 CommonRes + 完整业务形状，不能把 2xx 当 CONFIRMED。私有回执核对命令、channel、chain、captured viewer、mutation、publish identity；经 Visibility/Head/MessageHead → PersistAtomic → facts/message readback 后才发 operation。读族通过 req_id 发 `im:read:result`。PENDING/NOT_FOUND 为未确认观察，RECONCILING 为超时/传输/持久化/对账失败状态；都不授权生成新 key。F3 是当前终态收敛缺口。

HTTP timer 30s；超时移除 corr，迟到响应不再消费。stop/身份切换清空队列/corr/timer，并发送 CATEGORY_CANCELLED 的 RECONCILING，不能把取消消费当作远端回滚。资源回放覆盖 stop/timeout/duplicate；没有真实网络 worker 释放或跨登录的本轮证据。

| 场景 | 本轮确认 / 边界 |
| --- | --- |
| 复用 post/post_update | 应复用现有消息传输、锚点与落库读回；隔离依据是 CATEGORY_CHAIN 类型、categoryChain props、独立 command/wire/operation，不能进入 TEXT 业务 handler。专用公开 card WS 不承担锚点 ACK；私有 Mine 不通过广播 |
| HTTP/WS/sync 单调 | 正式有 category_revision guard 接入共享消息 upsert/edit 与提交后读回；实际缺失，F1 已失败。两套都有分类 HTTP 自身的队列/head/CAS，不能因此声称共享恢复已单调 |
| cursor | 正式 `post::queue_edit` 将 edit 与 cursor 放同原子写集；sync matching PersistOk 后提交 gate cursor。card-only WS 明确不推进 cursor。持久屏障机制存在，但 category live 漏事件恢复未验证 |
| 断线期间他人修改→重连 | **UNVERIFIED**。本轮共享 RecoverySession/scheduler 单测证明局部状态机，未断真实 WS、未制造离线漏事件、未检查 real sync/SQLite/EventSink 全链 |
| 退出重登 | **UNVERIFIED**。本轮 synthetic 新建 ImModule + 同存储只证明旧 head 不倒退。历史 Native“取消成功后同 SQLite 重启”不等于断线漏事件恢复 |
| 多账号 | [runtime.rs:709](/Users/cses-11/Desktop/cses-client/src-tauri/src/features/helix_im/runtime.rs:709) 默认 SQLite 路径为 company/user；分类事实 scope 含 viewer/channel/chain，tenant 隔离依赖 host 分库。根模块切换身份取消 pending。真实切账号/回灌隔离未运行；debug HELIX_IM_DB override 可破坏分库，不能用共库运行宣称隔离通过 |
| 取消/关闭/撤回 | 专项覆盖完整 selections→空集、close/retract、迟到私有读拒绝；不代表 F1/F2/F3 的异步组合已通过。撤回 tombstone 独立公共版本，防旧私有回执重新露出 |
| 普通消息/TEXT | TEXT 五个专有源文件与 HEAD 无 diff；两套 TEXT九路由兼容测试通过，当前实际 lib 的文字接龙局部测试通过。共享存储与恢复有变更，不能只凭独立 command registry 声称零影响 |

## 5. 本轮测试命令与真实结果

统一离线缓存：`CARGO_HOME=/tmp/helix-category-cargo-home`，`--offline --locked -j2`（临时 standalone probe 首次从复制的实际 lock 离线解析自身 package，之后 locked）。未启动外部服务。

| 运行面 / 测试 | 结果 | 原始日志 |
| --- | --- | --- |
| 正式 IM `--lib` | 370 pass / 1 fail（旧 Card 样例缺 maxCategories） | [formal-im.log](/tmp/category-review-20260909/formal-im.log) |
| 正式 category_chain_lifecycle | 39/39 PASS | [formal-im-integration.log](/tmp/category-review-20260909/formal-im-integration.log) |
| 正式 gs01_04_v2_sync_recovery / p4_sync_scheduler / post_action_inbound / posts_update_viewer_projection | 7/7、4/4、7/7、1/1 PASS | 同上 |
| 正式 s5_post_echo | 4 pass / 1 fail：self echo under gap 在 durable gate commit 前 emitted received | 同上 |
| 正式 Host category_chain_storage | 4 pass / 2 fail：当前严格合同拒绝旧测试输入，未到预期 PersistAtomic | [formal-host.log](/tmp/category-review-20260909/formal-host.log) |
| 正式 Host storage_conformance / upsert_guard | 21/21、3/3 PASS | [formal-host-neighbors.log](/tmp/category-review-20260909/formal-host-neighbors.log) |
| 实际 0.1.9 IM `--lib` | 408 pass / 1 fail（同 F5） | [loaded-im-available.log](/tmp/category-review-20260909/loaded-im-available.log) |
| 实际 0.1.9 category_chain_lifecycle | 31 pass / 5 fail：fixture 目录缺失 | [loaded-lifecycle.log](/tmp/category-review-20260909/loaded-lifecycle.log) |
| 直接请求实际副本邻接集成 target | Cargo exit101：只有 category_chain_lifecycle 已注册 | [loaded-im.log](/tmp/category-review-20260909/loaded-im.log) |
| 临时 probe → 实际生产 path：正式已有权限/fact 回归 | 0 pass / 2 fail，复现 F2/F4 | [loaded-regression-probes.log](/tmp/category-review-20260909/loaded-regression-probes.log) |
| 临时 probe → 实际生产 path：SQLite旧Post、迁移、历史reconcile | F1 revision10→9、version_key缺列、F3 RECONCILING；权限/fact在复制helper测试中再次失败，不能重复计数为新问题 | [loaded-storage-reconcile-probes.log](/tmp/category-review-20260909/loaded-storage-reconcile-probes.log) |
| 临时复制正式邻接 tests → 实际0.1.9 | Recovery7/7、scheduler4/4、viewer1/1；post inbound2/7，echo3/5 | [loaded-neighbor-probes.log](/tmp/category-review-20260909/loaded-neighbor-probes.log) |

正式主命令：

```bash
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline --locked -j2 -p helix-im --lib --test category_chain_lifecycle --test post_action_inbound_test --test s5_post_echo_test --test p4_sync_scheduler_test --test gs01_04_v2_sync_recovery_test --test posts_update_viewer_projection_test
# 上条 lib 失败后，原命令去掉 --lib 运行六个既定集成 target。
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline --locked -j2 -p helix-driver-host --test category_chain_storage --test upsert_guard --test storage_conformance
# category target 失败后，既定另外两组独立执行：
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline --locked -j2 --no-fail-fast -p helix-driver-host --test upsert_guard --test storage_conformance
```

实际副本在 `/Users/cses-11/Desktop/cses-client/.cargo/category-chain-helix` 运行：

```bash
CARGO_HOME=/tmp/helix-category-cargo-home CARGO_TARGET_DIR=/tmp/category-review-019-target cargo test --offline --locked -j2 -p helix-im --lib --test category_chain_lifecycle
# lib 失败后去掉 --lib，单独执行 category_chain_lifecycle，结果仍非 Green。
```

临时 probe 的生产依赖直接指向实际副本；独立 target 在 probe 自身目录，不跨 worktree 共享 target。复制测试来源/适配说明见 [probes/README.md](/tmp/category-review-20260909/probes/README.md)。

```bash
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline --locked -j2 --no-fail-fast --manifest-path /tmp/category-review-20260909/probes/Cargo.toml --test category_chain_storage --test reconcile_review category_review_
CARGO_HOME=/tmp/helix-category-cargo-home cargo test --offline --locked -j2 --no-fail-fast --manifest-path /tmp/category-review-20260909/probes/Cargo.toml --test post_action_inbound_test --test s5_post_echo_test --test p4_sync_scheduler_test --test gs01_04_v2_sync_recovery_test --test posts_update_viewer_projection_test
```

邻接失败限制：正式 echo 失败没有做干净 HEAD 对照，不能断言由分类 diff 引入。正式测试复制到较新 0.1.9 后，旧 helper/消息回放假设与新 admission/echo 基线不兼容（例如期望 PersistAtomic、实际空 effect 或 Persist+CancelTimer），不能把这些失败直接定性为生产回归。按有界 review 规则停止扩张，不修普通消息或调整其协议；完整普通消息/真实恢复准入仍需主任务在实际候选上验收。

## 6. 架构、热路径和证据边界

- 业务规则保留 helix-im；core 新增通用 UpsertGuard/version_column/StorageConflict，未引入 IM schema/网络 runtime；Host/Web 执行事务与单调比较。版本守卫默认 None，非 CATEGORY 不启用。未运行本轮三端 check/wasm 构建、FFI 或真实浏览器，不能声称三端验收。
- 热路径脚本：两套分类 runtime/facts 通过，正式 post/post_events 通过；两套 Host storage/write/upsert_sql 报 multi-clone 命中。它们属于共享既有路径/匹配规则，不能把分类局部通过写成整仓零分配。日志 [hot-path-audit.json](/tmp/category-review-20260909/hot-path-audit.json)。本轮不做性能优化或承压。
- facts 写法有明确单文档字节/叶子/深度上限，normalized scalar rows 无整数组 RMW；整体操作仍随 snapshot 叶子数增长，不能宣传每次业务操作总成本严格 O(1)。正式版解决读限截断，但未发现本轮测试证明长期旧路径/operation receipt 存储回收。
- `git diff --check`（unstaged）通过；`git diff --cached --check` 在既有 docs/*.patch 的空白补丁行报错。本文/SESSION单独检查，不清理他人 staged patch，不声明全 dirty 树 whitespace clean。
- 本轮真实 Go/Native/WS/SQLite全链、UI DOM、断线漏事件、退出重登、多账号、跨Pod均 **NOT_RUN**。本文 SQLite 是内存独立库 + 真实 HostStorage 的 LOCAL_INTEGRATION；HTTP/WS 输入由测试回灌，不是真远端。
- 历史 Native cancel→同SQLite重启和早先隔离 Native PASS 仅作定位资料；未使用这些结果填补本轮漏事件恢复缺口。没有访问真实账号资料、群或真实 DB；所有新增测试资源归本轮 `/tmp`，内存 SQLite 随测试进程释放，日志/probe为审查证据保留。

## 7. 客户端对接准入条件

1. 对实际 `.cargo/category-chain-helix` 完成 F1/F2/F4 对应修复与依赖迁入，冻结四包 path/lock/SHA；保留其原有 0.1.9 普通消息与观测基线，不整仓覆盖。
2. 与 Go/客户端冻结 F3：旧操作 receipt 的 CONFIRMED 与最新 Card/Post 投影分别收敛；同 key 的历史成功必须有可消费终态，不得永久 RECONCILING 或换 key 重试。
3. 修复当前 required-maxCategories 测试输入与 fixture 制品完整性，使实际候选所有分类生命周期、SQLite、失败路径与单调交错回归可复跑；公开/私有字段与当前合同逐跳一致。
4. 在真实隔离环境明确制造离线窗口：先记录 committed cursor→断开接收→另一成员取消/改类/关闭→重连，证明 sync 补缺→原子持久化→cursor推进→EventSink/客户端投影同版；另跑退出重登与账号切换/迟到回灌。
5. 对 TEXT、普通消息发送/echo、频道摘要、共享恢复及混合批次运行实际候选的匹配基线；本轮失败与缺失不得忽略，不以单库重启/HTTP200替代。

Review 已完成；实现与客户端准入未完成。后续修复、真实账号/环境操作、提交或发布由 review 主任务按用户授权另行协调。

## 8. 同日提交整理复验

用户随后授权提交工作区与暂存区。正式仓只在 wire 库测试 Card 与 Host lifecycle Card/Mine 输入补齐必填 `maxCategories=2`，生产逻辑未调整；上述首次失败记录保留。正式 IM lib 371/371、分类生命周期 39/39、Host 分类 SQLite 6/6、storage_conformance 21/21、upsert_guard 3/3 通过，日志 `/tmp/helix-submit-im.log`、`/tmp/helix-submit-host.log`。canonical JSON 经普通模式逐字比对通过，provenance 的源文件/fixture SHA-256 同步到本次已验内容。此复验解决正式仓旧样例失败，不改变实际客户端副本 F1～F5、普通 echo 或真实恢复验收的结论。

五 crate（core、IM、native、FFI、Web）离线 locked `cargo check` 通过，耗时 39.57s（`/tmp/helix-submit-check.log`）。代码提交 `1ed15f08`；本次未重跑 wasm、全量测试或真实跨端链。历史补丁经 `git apply --numstat` 解析，字节保持原样；`docs/.gitattributes` 只允许 `.patch` 数据的行尾空格，使 unified diff 空白上下文前缀不被误清理。最终提交内容 diff check 通过。
