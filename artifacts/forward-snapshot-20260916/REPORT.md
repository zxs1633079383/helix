# 合并转发快照：Helix 源码交接

任务 `6aaa06ccba9349dc11743ae6`。本批源码及本地公开 seam 验证完成；真实 Go、手机制品和 UI 性能未在本工作包验收。分支 `reactor/MessageV3-helix`，基线 `0143874b4c608e4e223561c7fa8512db0746e012`，未 commit/push、未提版。原有三个 artifacts 目录未修改。

合同：Client `artifacts/forward-nesting-audit-20260916/CONTRACT.md`，由主会话协调 Go/Helix/Client。收尾追加允许 `offset == itemCount` 的空终页（`nextOffset:null`）；其余空页拒绝。

## 已落地

- `im_relay_messages` 经共享 `client_api::build_command` 映射 `im_create_posts`；严格接收 `post_ids,target_channel_ids,mode,command_id,req_id`。HTTP 只有 `postIds,targetChannelIds,mode,commandId`，不读本地源全文、不接收客户端快照/子树。`forward_shape.rs` 已删除。
- MULTIPLY 的历史、消息增量、频道 lastPost 共用 bounded `props.forward`；删除递归 `forwardDetail` 解码。卡片只含 snapshotId/title/itemCount/最多 3 行 preview。
- `im_forward_detail({root_post_id,path,offset,req_id})` 调用 `/posts/forwardDetail`。成功字节为 `{event:"im:read:result",data:{req_id,body:<page>}}`，`body.items` 直接是 ChatMessage，稳定 id/temporaryId 为 `forward:{snapshotId}:{itemKey}`，保留 `forwardItemKey`。子卡保留作者和时间。
- 每页最多 20 项、item JSON 合计 1 MiB，单项 256 KiB，最多 31 段 path；普通项拒绝递归字段、未知顶层字段和 MULTIPLY 冒充。无新增依赖或持久详情缓存。
- `im_cancel_forward_detail({req_id})` 发 `CANCELLED` 并立即释放模块 corr/timer；迟到回包不再 Emit。**这只是逻辑取消，不立即中断 HTTP socket**。Native/FFI 沿已有 Host HTTP timeout，Web 沿已有 fetch timeout/AbortController 回收；未新增 CancelHttp。
- commandId 是稳定幂等键；req_id 是本次等待标识。正在执行的同 commandId 返回 `FORWARD_IN_PROGRESS`，不覆盖原等待者。WS 的 `tracing.csesTrackId=commandId` 映射回本次 req_id。
- 每目标记录唯一 durable Post，逐条转发须等全部项持久化并回读后才在最后一条 received 上补 requestId。HTTP accepted 不结算；committed 重试先 posts/get → Persist → 每条持久回读。committed IDs 数量须与源数量（合并为 1）一致。WS 先完成也释放仍在途的 HTTP correlation。

## 命令生命周期

| 命令 / 分类 | 成功终态 | 失败、超时、取消 | 清理与 N/A |
|---|---|---|---|
| im_relay_messages / 远程写、WS 驱动 | 每目标最后一条唯一 durable Post 的 im:post:received.requestId；committed 重试须持久回读 | 输入拒绝无 Effect；HTTP/业务/读取/Persist 失败产批次失败；FORWARD_TIMEOUT；身份替换/stop 为 CANCELLED | 所有目标结束后释放 ledger/timer/本命令 corr；失败/超时/取消亦清理；公共消息 Persist 的独立状态继续由既有 Channel 状态机拥有 |
| im_forward_detail / 远程只读 | im:read:result.data.body 直接分页模型 | 非法 req 有可用 req_id 时发 error；HTTP、业务、坏页均 error；FORWARD_DETAIL_TIMEOUT；身份替换/stop/显式取消为 CANCELLED | corr 在 PortReply 分发时移除；成功和错误均 CancelTimer；重复/迟到回包无业务 Emit；Persist/WS N/A |
| im_cancel_forward_detail / 本地确定性 | 当前 pending 请求发一次 CANCELLED | 非法 payload 拒绝；未知/已结束 req_id 幂等无 Effect | 立即移除详情 corr 与 timer；HTTP/WS/Persist N/A；物理 HTTP 由已有超时回收 |

详情最多 32 个在途；转发最多 64 个在途命令，每次最多 50 目标、100 源。详情相同 req_id/相同参数不重复发 HTTP；同 req_id 冲突关闭原等待并发一次错误。

入口同步：共享命令 alias、ImModule accepts/dispatch、read PortReply、timer/身份替换/stop 已接线。FFI `api/command.rs::build_app_command` 和 Web `HelixWebClient::invoke` 均调用共享构造器，Native 交付既有 Tick/Effect；无需增加每端私有业务命令表或 Event reducer。storage-schema-lock 的 message.props 由旧 merge 字段改为 forward；event schema 的开放 data 不需更改。Spec 047 Gate 分母未改。

## 验证证据

| 证据 | 结果与边界 |
|---|---|
| 先红：`cargo test -p helix-im --test forward_snapshot_test` | 生产修改前 3 个公开 seam 测试按预期失败：command_id 被拒绝、详情/取消命令不可达；会话工具输出留证 |
| 五 crate check | cses-helix-core、helix-im、native、ffi、web 通过；第一次完成模块后的检查，9.27 秒。最后 IM 收尾由下列最终库/定向与 WASM 重新编译覆盖 |
| 完整 IM + Core | [日志](logs/full-im-core.log)：159 个 suite，1554 passed / 2 failed / 12 ignored。两个失败见下一节；不是全库 GREEN |
| 最终库 + 定向 | [日志](logs/final-focused.log)：461 passed / 0 failed。IM 库 421，forward_snapshot 10，G03a 8，G03b 8，G05 3，cp_create_posts 2，one_line_server_contracts 9 |
| WASM 最终构建 | [日志](logs/wasm.log)：`cargo build --target wasm32-unknown-unknown -p helix-driver-web` PASS，13.05 秒 |
| Core 零运行时 | [依赖树](logs/core-tree.log)：无 tokio/sqlx/reqwest/web-sys/tauri |
| 真实 producer bytes | [G-05 产物](G-05/generated/provenance.json)：公开 Command → HTTP/WS authority 回灌 → PersistOk/读回 → 实际 Emit 编码；sender/receiver 两视角。属于本地回放，authority 输入为 fixture，不是真实 Go |
| Canonical Schema | [日志](logs/canonical-schema.log)：既有 schema validator 二进制读取本批 4 份产物 PASS；artifact SHA256 与 storage schema hash 全匹配 |
| 格式/差异 | rustfmt 与 git diff --check PASS；不包含既有 untracked artifacts |

关键可证伪覆盖：100 条 RICH 五页无遗漏/重复且正文/媒体字段保留；1/8/18/32 层输入有界、33 层拒绝；20 项/1 MiB/256 KiB 保护；错误 root/path/分页/递归项拒绝；取消/超时/错误/stop/迟到/重复；在途容量释放；committed Persist 错误；100 条逐条转发只有最后一条关联 requestId、重复回声不重复计数、WS 全完成后 HTTP/Timer 迟到无业务 Emit。

## 全量失败与未验收

两处失败不在本批改动的测试或行为：

1. `message_v3_mv3_g05l::toolbar_action_ticks_are_transparent_to_the_next_real_command`：负向控制重复 inbound 未改变 Effect 序列，触发原 `assert_ne!`。SESSION 原有记录亦有该失败，本批未改该测试或 Channel 去重流程。
2. `message_v3_mv3_g07b::wire_body_is_the_full_announcement_post_shape`：实际 `[群公告] Phase 1 群公告`，测试期待 `Phase 1 群公告`。HEAD 已有 typed announcement 前缀，本批未修改该分支。未另建干净 checkout 复跑基线。

按 AGENTS 的第二个无关失败停止扩张；两个目标保持 PARTIAL，不扩大本任务到公告/工具栏修复。

Go 权限与事务/持久化、真实 HTTP/WS/SQLite 链、Native 成品、Android/iOS archive 和消费者、Web 浏览器运行、100 条冷/热打开与滚动性能，均由主会话分别验收。以上本地回放/编译不替代这些证据。不得据此称全端交付或流畅度 SLA。

本批为未提交源码：[source-state.json](source-state.json) 固定最终 Rust/测试/schema 文件 SHA256；canonical 的 helix_head 是基线 HEAD，不代表该 commit 已含当前修改。生产源码最终 digest 为 `32500a3265920ca424be7262a58c02020239417a7e3247044f8dba9bab90461d`。

Cargo 预算偏差：共 7 次编译/测试（红测 1、check 1、全量 1、收尾定向 2、WASM 2），另有只读 tree 1。收尾审查发现 WS-first correlation 与 lastPost 投影边界，补齐后重验；没有逐条报错→check 循环。后续应在通知源码冻结前完成全部输出路径审查。

## D5 业务引用决议

业务快照项携带原 channelId/viewers，itemKey 为原 postId。render_ready/forward 对明确十种业务类型恢复 id=itemKey 与频道/受众，temporaryId 保持 forward:snapshot:itemKey；业务 props 与文档 live/snapshot 模式原样保留。普通项/子卡不接收业务来源上下文。原业务权限仍由原接口决定。新增身份隔离和畸形上下文回归；当前 lib 2 项与 forward_snapshot_test 10 项通过。Native 验证另见 Client BUSINESS-CARDS.md。

### D5 当前发送形态与精确读取补充

当前任务 wire 是 TASK_CARD + taskCard/taskCardBlock，会议 wire 是 TEXT + meetingCardBlock；均保留原业务上下文，渲染行键仍属于转发记录。Native WDIO 实测发现 exact-post readback 对 SQLite JSON 文本二次编码导致空 props，现于 project_exact_posts 复用现有 shape_row 再生成严格投影；G11g/G11h 共 7 项通过，补充断言 taskCard、userSnapshot、viewers。桌面可见性依赖的实时订阅验收状态见 Client BUSINESS-CARDS.md。
