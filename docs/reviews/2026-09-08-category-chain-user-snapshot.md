# 分类接龙展示快照透传

结论：Helix 字段修复与候选源码验证完成；真实 Go/客户端运行链路未在本任务重跑。

本轮租约：Helix `reactor/MessageV3-helix @ caf20f3b` 的 `category_chain/wire.rs`、`tests/category_chain_lifecycle.rs`，及客户端 `.cargo/category-chain-helix` 对应两文件的精确增量。保留既有 dirty 改动；不改 TEXT、runtime/facts、版本、锁文件，不提交、不重启。客户端入口和 Go DTO/资料补齐由各自任务负责。

## 冻结合同

| 边界 | 契约 |
|---|---|
| 入口 | `im_category_chain_set_participation`、`im_category_chain_publish`；canonical 名称保持不变 |
| 公开 payload | 可选 `userSnapshot` 或规范顶层 `user_snapshot`；既有公开归一化只转换顶层名称 |
| 内层展示字段 | 可选字符串 `userName`、`deptName`、`orgName`，保持大小写和字符串原值，不 trim、不补造姓名 |
| 身份 | 允许兼容 TEXT 的可选字符串 `userId` 提示，但不作为 actor、不提升到顶层；最终身份由 Go 认证会话覆盖。客户端本轮不传此字段 |
| 校验 | 整体缺省/null 时不发送；空对象/空字符串保留；未知键、非字符串或任意字符串超过 2000 Unicode 字符拒绝 |
| HTTP | `post-chain/append?mode=CATEGORY` 与 `post-chain/publish?mode=CATEGORY` body 新增可选 `userSnapshot`；mode、mutation、revision、selections 不变 |
| 终态 | HTTP 展示提示不是成功事实；仍以 Go 返回 authority → Persist → readback → `im:category_chain:operation`/既有 post event 为准 |
| 失败/资源 | 非法输入零 Effect；发布身份 Persist 屏障、HTTP/业务失败、PersistErr、timeout/cancel 和迟到去重沿用既有生命周期，不新增 correlation/timer |
| 响应 | 不新增/重命名 EntryPreview 字段；仍为 `displayName`/`departmentName`。Go 的旧空快照补齐和版本推进不在 Helix 本轮实现范围 |

首个已修复失配：`wire::build_publish` / `build_participation` 原先将 `user_snapshot` 拒绝为 unknown/authority field。现在仅这两个 builder 放行，由共享 `copy_user_snapshot` 完成有界校验和一次 HTTP 边界复制。TEXT 五个领域、命令、WS、提交文件逐字未改。

## 影响核对

静态调用链：公开 command normalization → Module `is_command` → `runtime::handle_category_command` 的 `wire_body` 校验 → outbound registry 的 `wire_body` → Go HTTP Effect。新 helper 仅有 publish/set-participation 两处调用；其他命令 whitelist 不变。

客户端 GitNexus 原 CLI wrapper 尝试下载 runner，因 registry DNS 不可达失败；改用索引元信息指向的已有 GitNexus 1.6.11/Node 20 runner。对候选私有 builder 的 impact 查询返回 target not found / `UNKNOWN`，不能作为低风险或零调用方证据。候选 source 位于隐藏的 `.cargo` 补丁目录，图覆盖未证实；以直接调用点和公开 seam 回归补充核验。

流程偏差如实记录：候选最小同步后才补跑该客户端 impact，未满足其“先 impact 后编辑”的时序；没有将本次图查询记为完整影响图通过，也没有更新图谱、提交或发布。

## 验证

- 主仓 RED：新增 4 项中 3 failed / 1 passed；失败均为合法 `user_snapshot` 被拒绝。
- 主仓 GREEN：分类生命周期 35 passed；含 client 提示进入 HTTP、实际 Emit 使用不同的 Go author fixture、发布 HTTP 403 失败、缺省/null/空对象以及非法对象零资源残留。
- 主仓单元测试 371 passed；core/IM/Native/FFI/Web `cargo check` 和 Web wasm build PASS。
- 候选 0.1.9：临时源码副本的分类生命周期 33 passed，含本次 4 项与既有 TEXT 九路由兼容。
- 候选首次运行缺少 5 个既有 canonical fixture；仅将主仓原 fixture 原字节复制到临时验证目录后重跑通过，未生成/改写 fixture。
- 由于实际候选没有 workspace Cargo.lock，验证在 `/tmp` 副本内解析临时锁；实际候选及主仓所有已记录 manifest/lock SHA 未变。测试副本的 wire/test SHA 与实际候选一致。
- 热路径审计、`git diff --check` PASS。无新的 runtime/platform 依赖或 schema 修改。
- REAL_NATIVE_CHAIN / UI_DOM：本任务未运行。客户端任务负责其两个实际 Host 适配 seam、facade/session 回归和客户端编译；不能把本任务的 focused 结果称为运行中的客户端已修复。

## 候选同步证据

写前逐字核验了两个候选文件，拒绝覆盖并发改动；wire 使用相同基线的窄变化，测试仅追加新段，候选原测试前缀保持不变。上一轮本仓 runtime/facts 优化没有被带入本次同步。

| 文件 | 修复后 SHA256 |
|---|---|
| 主仓与候选 `category_chain/wire.rs` | `9e84fa5ba5f471a09a196dcf391bba2d8b776a6576e4eee1558dd7da3a357758` |
| 候选 `tests/category_chain_lifecycle.rs` | `b574441fd7e2a5f9fef717437710d88c7ce9f010b2db982e9730ee52a55586da` |

本机证据目录：`/tmp/helix-user-snapshot-20260908/`。其中 `before.json`、`candidate-sync.json`、`unchanged.json`、`snapshot-only.patch` 为改动对账；`red.log`、`green-main.log`、`green-candidate-final.log`、`main-lib.log`、`platform-check.log`、`wasm.log` 为分层验证。临时副本和日志保留供协同任务复核，没有修改共享数据库或启动测试外部服务。
