# 固定消息 seq 分页：Helix 实施交接

状态：共享内核实现与定向回归已通过，五 crate 编译、WASM 与 FFI clippy 通过；广域测试非全绿，具体边界见验证证据。真实 Go/Tauri/WDIO 由本次客户端主任务验收，本文不把合成 HTTP 回放等同真实服务。

基线：在原分支 `reactor/MessageV3-helix` 实施，主任务经用户授权 fast-forward 到 `e6bcb80c`（0.1.86）。未修改版本、提交或发布；历史 untracked 制品未清理。

## 行为与边界

- `im_query_messages_by_channel`、`im_load_older_context`、`im_load_newer_context`、`im_locate_context` / `im_locate_message` 仅在请求显式 `sequencePagination: true` 时进入新路径。未启用的命令走原实现。
- 默认目标 20 条可见消息，页长允许 1–60。先以最多 128 个 seq 的主键集合读取 SQLite；可见行正文缺失同样是缺口。只补第一个连续缺段，写入后重新读回，不能用“有 20 行”证明范围完整。
- 隐藏 slot 只保留 seq、可见性与完整性版本，不含消息身份或正文。页面按 seq 升序返回；cursor 按实际扫描过的 slot 推进，包括隐藏项。新页面行额外携带 `postSeq` 十进制字符串，避免消费者按可能倒序的 `createAt` 重排；它不等于 `eventSeq`，也不添加到旧 Post/WS 协议。
- 撤回与权限不可见分开处理：对当前 viewer 可见的撤回消息保留提示行和定位锚点，计入页长。seq 路径在 HTTP 合并前和本地读回后清空撤回正文的八类字段；Go `omitempty` 省略正文不能使旧缓存回填。撤回标记和回应清空与正文使用同一 event_seq 守卫写入，较新 WS 状态保持优先；真正不可见项仍只存 slot。
- 每个命令最多扫描 2048 个 slot、16 次 HTTP（含 metadata）、总 deadline 10 秒；最多 32 个 pending。达到扫描预算可返回不足一页甚至空页，但只要尚有范围就带 `hasOlder/hasNewer` 和能推进的 cursor。空页不等于历史到头。
- `window: "forward"` 从目标开始向后补，目标下标为 0；默认 center 在两侧分配，短侧额度补给另一侧。历史末尾与 `i64::MAX` 已有回归。
- warm 页仍有一次 metadata HTTP，保证正文/权限版本新鲜；完整本地范围不再请求正文 range。不能宣称“零网络”。普通追加只改变 head，不使旧范围 revision 失效。
- latest / locate 成功读回后默认登记原 `TimelineState`，保留实时编辑、发送回读等既有刷新入口；locate 使用锚点模式，后续刷新发 anchored `im:timeline:update`。older/newer 不重建窗口。仅新 locate 可显式 `attach:false`，用于框选端点读取 postSeq，此时不创建或替换窗口，也不更新窗口时间观察；无 windowToken 本身不等于只读。
- 不写 `channel_event_cursor` / `channel_event_ledger`，不推动 gate，不使用页水位补齐 event 流。原 WS、同步、已读提交逻辑不变。

## HTTP 合同

`POST {api_base_url}/posts/sequenceRange`：metadata 不带范围，可带 `anchorPostId`；range 带含两端的 `fromSeq/toSeq`、`snapshotHeadSeq`、`membershipEpoch`、`expectedRevision`。所有序号、epoch、revision 为规范十进制字符串，拒绝 JS number、负数、前导零和 i64 溢出。

响应 `SUCCESS.data` 包含 `supported/ready/headSeq/snapshotHeadSeq/firstSeq/membershipEpoch/revision`，可选 `anchorSeq`；range 每一个 seq 都必须有一个 slot。可见 slot 带原 Post 投影与字符串 `postEventSeq`；隐藏 slot 只能是 `{seq,isVisible:false}`。漏 slot、正文不属于请求群、版本不一致、隐藏项夹带正文均拒绝，不产生完整性证据。

仅以下情形允许回旧协议：metadata HTTP 404；旧 Go router 精确响应 `{"status":"failed","message":"not found"}` 且 data 缺省/null；明确 `supported:false` 或 `ready:false`。鉴权、网络、其他业务错误不降级。已持有 v2 cursor 后服务不再支持时明确失败，不从别的位置悄悄继续。

Go 已在 PRE 发布 f85109e7，seq 是默认读取能力，不再读取全局开关；未就绪会话仍返回 `supported:true,ready:false` 并回落旧查询。新协议不要求旧 HTTP/WS 携 seq。

## 存储、版本与升级

导出 `IM_TIMELINE_SEQ_SCHEMA: &[&str]`、`IM_TIMELINE_SEQ_SCHEMA_VERSION: u32 = 1`，独立创建 `message_sequence_slot`，不改变 message 主键 `temporary_id`。slot 主键为 channel+seq，另有 `(channel_id,seq)` 唯一约束；包含 `is_visible/temporary_id/membership_epoch/revision/body_event_seq`。

原 `IM_SCHEMA` 与 `IM_SCHEMA_MIGRATIONS` 文本与当前 HEAD 完全一致，哈希证据在 `artifacts/post-sequence-20261023/helix-verification/schema-fingerprint.json`。客户端必须独立事务升级新 DDL，成功后以账户+Runtime generation 打开能力；失败保留旧库与原同步，不重放正文清洗。FFI pump 添加独立 DDL 尝试；Web session storage 沿用共享 sparse descriptor，不增加业务 port。

正文与 slot 同一 `PersistAtomic`。正文沿用现有投影/合并规则并以 event_seq 防旧 HTTP 覆盖较新 WS；每个 slot 使用 revision CAS，使同 eventSeq 的私有正文变更也能挡住旧 HTTP。隐藏确认会用正文版本 CAS 清掉既有正文；并发更新令整笔事务拒绝，不能留下半份覆盖证据或假 message 行。

同频道最新 epoch/revision 内存围栏拦截跨不重叠范围的旧响应；账号切换清理围栏并取消查询。会话 epoch 改变、频道已终止时拒绝旧结果。服务端 `SEQUENCE_SNAPSHOT_CHANGED` 最多两次重取 metadata；客户端 cursor 的 epoch/revision 变化返回 `CURSOR_EXPIRED`，消费者应清掉旧 cursor 并重新定位/进入，不能继续使用已扫位置。

## 消费合同

成功事件为 `im:read:result`，body 含 `reqId/channelId/operation/messages/hasOlder/hasNewer/pageSize/olderCursor/newerCursor`；每个 messages 行在既有整形后按 msgId 稳定回填 `postSeq` 字符串；locate 额外含 `targetPostId/targetIndex`。仅新路径返回 `sequencePagination:true`，用来区分 legacy 降级；不能只根据请求 flag 推定已经登记新窗口。latest/locate 可传 `windowToken` 与前端窗口对应。新 locate 的 `attach:false` 在 legacy 降级前剥离，不进入旧 strict parser。

实时旧投影不带 postSeq。逆时间真实 SQLite + WS 回灌已确认：latest 原序列 p41..p60，legacy update 返回 p60..p41，direction 为 append，且 hasOlder 可能从 true 变 false；定位窗口对应返回 newer patch，仍按时间倒序。它没有 replace 事件，但消费者不能据此重排或覆盖 seq 分页边界。客户端需保留已知 postSeq、已有相对位置以及 seq cursor/hasOlder/hasNewer；该消费端保护由客户端主任务验收。诊断证据在 `sequence-final-tests.log` 的 `sequence-live-patch` 行。

Opaque cursor 是版本 2 对象：`version/channelId/direction/nextSeq/headSeq/membershipEpoch/revision`。older/newer 可仅带 cursor，不依赖可见 anchor；所有 seq 字段是字符串。前端不自行做 seq 运算，不从最后一条可见消息推导是否耗尽。

## 命令生命周期

| 阶段 | 终态/所有权 |
| --- | --- |
| 入口 | 公开 `client_api::build_command` → `ImModule::handle(Tick::Command)`，归类远程只读 + 本地缓存写 |
| 成功 | metadata → bounded local slots/body → 缺段 HTTP → PersistAtomic → slots/body readback → 单次 read result |
| 输入校验 | 任何 I/O 前返回 ModuleError；无 pending/timer |
| HTTP/业务/本地读取错误 | read error；取消 timer，删除该 corr，不改已发布窗口 |
| 持久化失败 | 不发布消息或覆盖证明，不推进同步 cursor；终止并取消 timer |
| 超时/stop/身份切换 | read error + CancelTimer，移除拥有的 corr；迟到/重复回复无效 |
| 关联 | req_id 必须非空且本模块 pending 唯一；每步只有一个 owned corr，deadline 跨所有重试不重置 |
| WS 发出/写业务命令 | N/A；本路径没有 WS Send 或远端写入 |

## 验证证据

- 公开 seam 红绿：初始新 flag 被原 strict parser 拒绝；本轮另有 center 尾部、i64 MAX、并发删除、同 eventSeq 旧 revision 覆盖、cursor revision 变化、旧 metadata 乱序的红绿记录。
- `cargo test -p helix-im --test sequence_pagination_test`：8 passed。覆盖旧服务/关闭开关降级、鉴权错误不降级、deadline/stop/迟到回复、非法 cursor、revision 失效、元信息围栏及只读 locate 参数边界。
- `cargo test -p helix-driver-native --test sequence_pagination_storage`：19 passed。真实 SQLite + 生产 Host executor；HTTP authority 为合成 fixture。覆盖稀疏补页、持久化重启复用、只补缺正文/slot、预算续扫、forward/center、原子失败、普通追加保留缓存、权限变化、并发 HTTP/WS/删除、实时编辑锚点、只读 locate 不改变窗口与逆时间实时 patch 消费边界。
- 最后补齐 `postSeq` 投影先红后绿，验证大于 2^53 的字符串精度、createAt 倒序时保持 seq 升序、重启复用后输出一致；只读 locate 先复现创建多余窗口、参数未校验，再转绿。红测 `post-seq-order-red.log`、`detached-locate-red.log`、`detached-locate-contract-red.log`，最终 8 IM +19 Native 通过日志 `sequence-final-tests.log`。
- 六包广域测试尝试受 0.1.86 已有 Host 测试编译失败阻断：`helix-driver-host/tests/upsert_guard.rs:477` 引用不存在的 `upsert_channel_full_op`。未修改该测试或相关生产函数；日志 `shared-tests.log`。其余适用检查另列结果，不把失败写成通过。
- `cargo check -p cses-helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web`、`cargo build --target wasm32-unknown-unknown -p helix-driver-web`、`cargo clippy -p helix-driver-ffi --all-targets` 均 exit 0；clippy 保留既有 warning，未扩大修复范围。命令与退出码见 `remaining-checks.json`。
- core/IM/Web/FFI 广域测试合计 1880 passed、136 failed。130 项依赖缺失的外部绝对路径参考包；3 项旧频道设置测试仍仅识别 `Persist`，当前既有实现返回 `PersistAtomic`；3 项 FFI 发送测试未建立当前账号/频道 render scope，未收到 `im:post:sending`。后两类仅核对源码归属，未改动工作区复建原基线；不把本轮全套称为通过。日志 `core-im-web-ffi-tests.log`。
- Native 全测合计 138 passed、101 failed。94 项缺少外部账号、隔离会话或翻译配置；余 7 项是旧草稿 render scope、64 MiB 上传阈值与同步夹具断言。相关生产同步、发送及上传路径均未修改，未扩修。该轮执行于最后 `postSeq` 投影补齐前，当时新增序列 16 项、原 Native 泵和存储压力目标均通过；最终新增投影后的定向结果见上项，日志 `native-tests.log`。
- `git diff --check` 通过；core 依赖树未引入 I/O 或平台依赖。未执行 C++ 链接冒烟与移动设备实机验收，本次没有 C ABI 变更。
- 最终 `attach:false` 与 postSeq 源码冻结后重跑五 crate check、Web WASM build 均 exit 0；基线、源码 SHA-256、27 项定向结果与命令退出码见 `final-source-verification.json`。
- 真实 Go HTTP/WS、业务账号权限、主/独立窗口 WDIO、规模压测：本文未验证，由主任务合并证据。5000 人群、5 RPS、多群并行、1 秒送达是目标，非本轮测试结论。

所有运行日志放在 `artifacts/post-sequence-20261023/helix-verification/`。测试只清理本轮临时 SQLite 文件；未清理业务库或修改远端数据。

## 2026-10-25 撤回分页回归

真实 Docker/Tauri 主任务发现撤回群进入失败，Helix 把 Go 返回的 `isVisible:true,revoke:true` 提示行误判为 `REMOTE_VISIBILITY_CONFLICT`；本地接受阶段也排除了该行。修复仅改 `query/sequence/data.rs` 与 `lifecycle.rs`，没有修改 Go、旧 PostFields/readback、schema、event 或同步规则。

复用旧 readback upsert 时另确认其没有 `revoke` 和 `quick_reply` 列，因此 seq 适配层补齐撤回标记及空回应，仍放在原 `PersistAtomic` 和正文 event_seq 版本保护内。本地较新 WS 撤回尚保留旧正文时，seq 读取仅清洗返回副本，不修改同步写入语义；后续权威撤回 range 可清除持久正文。

- 先红：新增 6 项均失败，分别复现冷/热页、撤回定位、旧缓存撤回、较新 WS 撤回、较新 WS 正常正文和隐藏/撤回混合场景。
- 最终命令：`cargo test -p helix-im --test sequence_pagination_test -p helix-driver-native --test sequence_pagination_storage -- --nocapture`，exit 0，8 IM + 25 Native 全部通过；原子失败、旧版本覆盖、权限清除、只读定位与重启回归一起保留。
- 121 个 slot（120 条撤回提示 + 1 条 NOTICE）在冷缓存和模块重启后均按 20、50、50、1 返回，全部 121 条无重无漏。冷页每页 1 metadata + 1 range，热页每页 1 metadata + 0 range。
- 测试使用公开命令、生产 Native/Host executor 和独立 SQLite，HTTP authority 为按 Go `omitempty` 行为构造的合成数据。日志中的耗时仅为该夹具观测，不是实际服务性能结果；主/独立窗口 WDIO 与真实网络性能由客户端主任务继续验收。

本轮日志和最终源码哈希：客户端 `artifacts/message-page-size-20261025/helix-revoked-red.log`、`helix-revoked-green.log`、`helix-revoked-verification.json`。未提交或改版本。

## 2026-10-26 发布准备

用户已授权合并、发布 Helix 下一 patch、push 并应用到当前客户端分支。PRE f85109e7 已通过真实 A/B 主窗口与独立窗口 6 组 WDIO；96 条 UI 消息的 seq、创建 event 与接收未读经数据库核对通过。证据在客户端 `artifacts/post-sequence-deployed-20261026/`，Go `docs/交接/post-sequence-default-read-deployed-20261026.json`。这部分证明待合并前的当前 seq 候选；合并及发布版接入后继续按影响范围验证。

### 合入远端后的验证

- 已合入远端两提交，seq 与回应的 correlation/取消/回包分派并存；仅 SESSION 两处记录冲突，保留双方。
- seq 33、回应 sans-IO 19、回应 SQLite 1，共 53 项聚焦通过；五 crate check、core tests、WASM 通过，core 依赖树无运行时依赖。
- IM 广域本轮 1748 通过、133 失败、11 ignored：延续缺外部绝对路径样本及旧 channel settings Persist 断言失败；不改本次无关夹具。
- Host 未读测试组 16 通过、1 旧频道删除分页断言失败；该失败函数与调用路径不由本批改动，新增回应 SQLite 场景单独通过。
- 原始日志与汇总：artifacts/post-sequence-release-20261026/merged-verification.json。

### 0.1.87 发布与客户端接入结果

- 四公共 crate 已按既定脚本发布到 crates.io，dry-run、八 crate check、上传和回读全部通过。版本提交 `aa0d5a7156ea6e3cb85eb6435408000b2b663c9b`；注释 Tag `helix-v-0.1.87` 已推送并核对远端一致。
- 客户端当前 `dev-message` 的 `src-tauri/Cargo.toml` 四依赖精确固定 `=0.1.87`，锁文件仅改变这四个包的版本与 checksum。Cargo metadata 确认来自 registry，七份关键源码逐字节匹配发布源码；客户端其他既有改动保留。前端依赖变更留在当前工作区，未另行提交或 push。
- 客户端 3 个定向 Jest 文件 / 22 项通过；A/B 在原 bundle、identifier、账号和数据库上重编签名。正式安装包升级、Windows、移动端及 5000 人容量不在本次实机验收内。
- 发布版真实 PRE WDIO 六组通过：主窗口 / 独立窗口历史翻页、latest 进入清未读后上跳、unread 进入逐步清除后下跳。造数群已有 238 个 slot，B 可见 90、隐藏 148，两窗口全量翻到且隐藏不泄露；上跳分割线全采样 0px，首帧主窗口 346ms、独立窗口 328ms，仅为单次观测。
- 96 条 UI 发送均经正式 API 与 PostgreSQL 回读确认：每条 seq>0、恰好一个创建 event，接收端持久阅读游标覆盖全部；主窗口 24→7→0、独立窗口 24→12→0。关群通知发生在清未读验收之后，产生额外 NOTICE；不可用关群后的 unread=1 否定此前阅读结果。
- 本轮两测试群通过正式接口关闭，并以 `channel.delete_at>0` 核对。附加“关群立即从会话列表消失”观察超时，不能写成该 UI 行为通过；未修改本次范围外的旧频道删除投影。A/B watcher 已恢复原配置和默认关闭的 WDIO 端口，二进制与验收候选一致。
- 旧 `cursorVersion:1` canary 响应摘要在本轮前后不变。证据：客户端 `artifacts/helix-registry-0.1.87-20261026/`（candidate、registry-source-proof、run-status、wdio-default-all、server-readback、ui-db-proof、cleanup-db-proof、restoration）。

### 未关闭：造数发送进入死信

240 次 HTTP 造数中序号 184/185/186 未进入 post 表；发送接口成功只表示 Pulsar 接收，不能当作落库。3 条记录均在 `message_disposition` 里为 `dead_letter_confirmed`，attempt_count=3、failure_reason=handler_error；temporaryId 为 preseqc19a06d3184/185/186。消费积压与未确认均为 0；根因未确定，不能认定与新 seq 写入无关。已有 post 的 seq 连续、无 NULL；未发生客户端漏翻已存在消息。已询问用户是否有其他消费者/运维操作。

该问题不回滚已发布的不可变 crate；Helix 发布和本地客户端接入完成，全面上线验收仍有阻断，不建议以本次六组 UI 通过直接放行客户端正式版。后续需从对应 delivery/trace 排查异步消费者的 handler_error，不重放或删除死信，不自动修改 Go/运维配置。证据：fixture-db-audit、sender-disposition-audit、sender-stats。
